// The window of pane: one raylib window on a thread of its own that draws the latest draw list and pushes input events to a port.
#include "vendor/raylib/src/raylib.h"
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define EXPORT __attribute__((visibility("default")))
#define CMD_WIDTH 12

typedef struct MotePort MotePort;

typedef struct MoteHost {
    uint64_t size;
    int32_t (*port_push)(MotePort *, const uint8_t *, int64_t);
    void (*port_close)(MotePort *);
    int32_t (*fd_wait)(int32_t, int32_t, int64_t);
} MoteHost;

static MoteHost host;

EXPORT void mote_init(const MoteHost *given) { host = *given; }
EXPORT void mote_init_pane(const MoteHost *given) { host = *given; }

// Event kinds in the first of the six little-endian i32 words of a message.
enum { EVENT_CLICK = 1, EVENT_KEY = 2, EVENT_TEXT = 3, EVENT_WHEEL = 4, EVENT_RESIZED = 5, EVENT_CLOSE = 6 };

typedef struct Window {
    pthread_t thread;
    pthread_mutex_t lock;
    pthread_cond_t started;
    MotePort *events;
    int width, height;
    char *title;
    int state;  // 0 starting, 1 running, -1 failed
    int stop;
    int headless;
    int64_t *commands;
    int64_t count;
    uint8_t *texts;
    int64_t texts_length;
} Window;

static pthread_mutex_t only = PTHREAD_MUTEX_INITIALIZER;
static int taken;

static void send(Window *window, int32_t kind, int32_t a, int32_t b, int32_t c, int32_t d, int32_t e) {
    int32_t words[6] = {kind, a, b, c, d, e};
    uint8_t bytes[24];
    for (int word = 0; word < 6; word++) {
        uint32_t value = (uint32_t)words[word];
        for (int at = 0; at < 4; at++) bytes[word * 4 + at] = (uint8_t)(value >> (8 * at));
    }
    host.port_push(window->events, bytes, 24);
}

static Color color_of(int64_t value) {
    return (Color){(unsigned char)(value >> 24), (unsigned char)(value >> 16), (unsigned char)(value >> 8), (unsigned char)value};
}

typedef struct Clip {
    int x, y, w, h;
} Clip;

static void apply(const Clip *stack, int depth) {
    EndScissorMode();
    if (depth > 0) BeginScissorMode(stack[depth - 1].x, stack[depth - 1].y, stack[depth - 1].w, stack[depth - 1].h);
}

static float roundness(int64_t radius, int64_t w, int64_t h) {
    int64_t least = w < h ? w : h;
    if (least <= 0 || radius <= 0) return 0.0f;
    float value = (float)radius / ((float)least / 2.0f);
    return value > 1.0f ? 1.0f : value;
}

// DejaVu Sans Mono, linked in as bytes; PANE_FONT_PATH is set by build.sh.
__asm__(".section .rodata\nfont_data:\n.incbin \"" PANE_FONT_PATH "\"\nfont_end:\n.previous\n");
extern const unsigned char font_data[] __attribute__((visibility("hidden")));
extern const unsigned char font_end[] __attribute__((visibility("hidden")));

// The face's line (ascent plus descent) is 1.1641 em, and raylib sizes a font by its line, so a font of `size` pixels is loaded at the line height for an em of `size`.
#define LINE_PER_EM 1.1641f
enum { FONT_SLOTS = 16 };

// The ranges loaded: Latin, punctuation, arrows, box drawing, blocks and shapes.
static const int ranges[][2] = {{32, 126}, {160, 255}, {0x2010, 0x2027}, {0x2190, 0x21FF}, {0x2500, 0x25FF}};

typedef struct Sized {
    int size;
    Font font;
} Sized;

static Sized fonts[FONT_SLOTS];
static int fonts_used;
static int fonts_next;

// The font for text of `size` pixels, loaded once on first use; only the window thread calls this.
static Font font_for(int size) {
    for (int at = 0; at < fonts_used; at++) {
        if (fonts[at].size == size) return fonts[at].font;
    }
    int count = 0;
    for (size_t range = 0; range < sizeof ranges / sizeof ranges[0]; range++) count += ranges[range][1] - ranges[range][0] + 1;
    int *codepoints = malloc((size_t)count * sizeof(int));
    if (!codepoints) return GetFontDefault();
    int filled = 0;
    for (size_t range = 0; range < sizeof ranges / sizeof ranges[0]; range++) {
        for (int codepoint = ranges[range][0]; codepoint <= ranges[range][1]; codepoint++) codepoints[filled++] = codepoint;
    }
    int line = (int)((float)size * LINE_PER_EM + 0.5f);
    Font font = LoadFontFromMemory(".ttf", font_data, (int)(font_end - font_data), line, codepoints, count);
    free(codepoints);
    if (font.texture.id == 0) return GetFontDefault();
    SetTextureFilter(font.texture, TEXTURE_FILTER_BILINEAR);
    int slot = fonts_used < FONT_SLOTS ? fonts_used++ : fonts_next++ % FONT_SLOTS;
    if (fonts[slot].font.texture.id != 0) UnloadFont(fonts[slot].font);
    fonts[slot] = (Sized){size, font};
    return font;
}

static void draw_text(const char *text, int length, int64_t x, int64_t y, int size, Color color) {
    char *copy = malloc((size_t)length + 1);
    if (!copy) return;
    memcpy(copy, text, (size_t)length);
    copy[length] = 0;
    Font font = font_for(size);
    // The line is taller than `size`; half of the extra goes above the text so it sits centred in its box.
    float top = (float)y - (float)(font.baseSize - size) / 2.0f;
    float at = (float)x;
    float advance = (float)((size * 6 + 5) / 10);
    for (int offset = 0; offset < length;) {
        int used = 1;
        int codepoint = GetCodepointNext(copy + offset, &used);
        DrawTextCodepoint(font, codepoint, (Vector2){at, top}, (float)font.baseSize, color);
        at += advance;
        offset += used > 0 ? used : 1;
    }
    free(copy);
}

static void paint(const int64_t *commands, int64_t count, const uint8_t *texts) {
    Clip stack[64];
    int depth = 0;
    for (int64_t index = 0; index < count; index++) {
        const int64_t *c = commands + index * CMD_WIDTH;
        Rectangle rec = {(float)c[1], (float)c[2], (float)c[3], (float)c[4]};
        Color color = color_of(c[5]);
        switch (c[0]) {
        case 1:
            if (c[6] > 0) DrawRectangleRounded(rec, roundness(c[6], c[3], c[4]), 8, color);
            else DrawRectangleRec(rec, color);
            break;
        case 2:
            draw_text((const char *)texts + c[8], (int)c[9], c[1], c[2], (int)c[10], color);
            break;
        case 3:
            if (c[6] > 0) DrawRectangleRoundedLinesEx(rec, roundness(c[6], c[3], c[4]), 8, (float)c[7], color);
            else DrawRectangleLinesEx(rec, (float)c[7], color);
            break;
        case 4:
            if (depth < 64) {
                Clip next = {(int)c[1], (int)c[2], (int)c[3], (int)c[4]};
                if (depth > 0) {
                    Clip top = stack[depth - 1];
                    int left = next.x > top.x ? next.x : top.x;
                    int upper = next.y > top.y ? next.y : top.y;
                    int right = next.x + next.w < top.x + top.w ? next.x + next.w : top.x + top.w;
                    int lower = next.y + next.h < top.y + top.h ? next.y + next.h : top.y + top.h;
                    next = (Clip){left, upper, right > left ? right - left : 0, lower > upper ? lower - upper : 0};
                }
                stack[depth++] = next;
                apply(stack, depth);
            }
            break;
        case 5:
            if (depth > 0) depth--;
            apply(stack, depth);
            break;
        }
    }
    EndScissorMode();
}

static void *run(void *argument) {
    Window *window = argument;
    SetTraceLogLevel(LOG_NONE);
    SetConfigFlags(FLAG_WINDOW_RESIZABLE | FLAG_VSYNC_HINT);
    InitWindow(window->width, window->height, window->title);
    int ready = IsWindowReady();
    pthread_mutex_lock(&window->lock);
    window->state = ready ? 1 : -1;
    pthread_cond_broadcast(&window->started);
    pthread_mutex_unlock(&window->lock);
    if (!ready) return NULL;
    SetExitKey(0);
    int closing = 0;
    for (;;) {
        pthread_mutex_lock(&window->lock);
        int stop = window->stop;
        pthread_mutex_unlock(&window->lock);
        if (stop) break;
        BeginDrawing();
        ClearBackground((Color){255, 255, 255, 255});
        pthread_mutex_lock(&window->lock);
        if (window->count > 0) paint(window->commands, window->count, window->texts);
        pthread_mutex_unlock(&window->lock);
        EndDrawing();
        Vector2 mouse = GetMousePosition();
        for (int button = 0; button < 3; button++) {
            if (IsMouseButtonPressed(button)) send(window, EVENT_CLICK, (int32_t)mouse.x, (int32_t)mouse.y, button, 0, 0);
        }
        Vector2 wheel = GetMouseWheelMoveV();
        if (wheel.x != 0.0f || wheel.y != 0.0f) {
            send(window, EVENT_WHEEL, (int32_t)mouse.x, (int32_t)mouse.y, (int32_t)(wheel.x * 100.0f), (int32_t)(wheel.y * 100.0f), 0);
        }
        for (int key = GetKeyPressed(); key != 0; key = GetKeyPressed()) send(window, EVENT_KEY, key, 0, 0, 0, 0);
        for (int codepoint = GetCharPressed(); codepoint != 0; codepoint = GetCharPressed()) send(window, EVENT_TEXT, codepoint, 0, 0, 0, 0);
        if (IsWindowResized()) send(window, EVENT_RESIZED, GetScreenWidth(), GetScreenHeight(), 0, 0, 0);
        if (!closing && WindowShouldClose()) {
            closing = 1;
            send(window, EVENT_CLOSE, 0, 0, 0, 0, 0);
        }
    }
    CloseWindow();
    return NULL;
}

static int64_t fail(uint8_t *error, int64_t error_length, const char *message) {
    int64_t length = (int64_t)strlen(message);
    if (length > error_length) length = error_length;
    memcpy(error, message, (size_t)length);
    return -length - 1;
}

// Opens the window and gives its events to the port; returns a handle, or `-length - 1` with a message in `error`.
EXPORT int64_t pane_open(int64_t width, int64_t height, const char *title, MotePort *events, uint8_t *error, int64_t error_length) {
    pthread_mutex_lock(&only);
    if (taken) {
        pthread_mutex_unlock(&only);
        host.port_close(events);
        return fail(error, error_length, "a window is already open");
    }
    taken = 1;
    pthread_mutex_unlock(&only);
    Window *window = calloc(1, sizeof(Window));
    window->width = (int)width;
    window->height = (int)height;
    window->title = strdup(title);
    window->events = events;
    pthread_mutex_init(&window->lock, NULL);
    pthread_cond_init(&window->started, NULL);
    if (pthread_create(&window->thread, NULL, run, window) != 0) {
        host.port_close(events);
        free(window->title);
        free(window);
        pthread_mutex_lock(&only);
        taken = 0;
        pthread_mutex_unlock(&only);
        return fail(error, error_length, "the window thread did not start");
    }
    pthread_mutex_lock(&window->lock);
    while (window->state == 0) pthread_cond_wait(&window->started, &window->lock);
    int state = window->state;
    pthread_mutex_unlock(&window->lock);
    if (state < 0) {
        pthread_join(window->thread, NULL);
        host.port_close(events);
        free(window->title);
        free(window);
        pthread_mutex_lock(&only);
        taken = 0;
        pthread_mutex_unlock(&only);
        return fail(error, error_length, "the window could not be opened");
    }
    return (int64_t)(intptr_t)window;
}

// Hands the window the draw list to paint from the next frame on.
EXPORT int64_t pane_present(int64_t handle, const int64_t *commands, int64_t commands_length, const uint8_t *texts, int64_t texts_length, int64_t count) {
    Window *window = (Window *)(intptr_t)handle;
    int64_t *copy = malloc((size_t)(commands_length > 0 ? commands_length : 1) * sizeof(int64_t));
    uint8_t *text_copy = malloc((size_t)(texts_length > 0 ? texts_length : 1));
    if (!copy || !text_copy) {
        free(copy);
        free(text_copy);
        return -1;
    }
    memcpy(copy, commands, (size_t)commands_length * sizeof(int64_t));
    memcpy(text_copy, texts, (size_t)texts_length);
    pthread_mutex_lock(&window->lock);
    free(window->commands);
    free(window->texts);
    window->commands = copy;
    window->texts = text_copy;
    window->texts_length = texts_length;
    window->count = count;
    pthread_mutex_unlock(&window->lock);
    return 0;
}

// Stops the thread of a window, closes it and gives the port back; a headless window only gives the port back.
EXPORT void pane_close(int64_t handle) {
    Window *window = (Window *)(intptr_t)handle;
    if (!window->headless) {
        pthread_mutex_lock(&window->lock);
        window->stop = 1;
        pthread_mutex_unlock(&window->lock);
        pthread_join(window->thread, NULL);
        pthread_mutex_lock(&only);
        taken = 0;
        pthread_mutex_unlock(&only);
    }
    host.port_close(window->events);
    free(window->commands);
    free(window->texts);
    free(window->title);
    free(window);
}

// A window with no screen that holds the port; returns a handle.
EXPORT int64_t pane_headless(MotePort *events) {
    Window *window = calloc(1, sizeof(Window));
    window->headless = 1;
    window->events = events;
    return (int64_t)(intptr_t)window;
}

// Pushes one message into the port of a window as if the user had caused it; returns the push status.
EXPORT int64_t pane_inject(int64_t handle, const uint8_t *bytes, int64_t length) {
    Window *window = (Window *)(intptr_t)handle;
    return host.port_push(window->events, bytes, length);
}
