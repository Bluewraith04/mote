// The window of pane: one raylib window on a thread of its own that draws the latest draw list and pushes input events to a port.
#include "pane_fonts.h"
#include "vendor/raylib/src/raylib.h"
#ifdef _WIN32
#include "pane_win32.h"
#define EXPORT __declspec(dllexport)
#define strdup _strdup
typedef PaneThread pane_thread_t;
typedef PaneMutex pane_mutex_t;
typedef PaneCond pane_cond_t;
#define PANE_MUTEX_INIT {NULL}
#define pm_init(m) pw_mutex_init(m)
#define pm_lock(m) pw_mutex_lock(m)
#define pm_unlock(m) pw_mutex_unlock(m)
#define pc_init(c) pw_cond_init(c)
#define pc_wait(c, m) pw_cond_wait(c, m)
#define pc_broadcast(c) pw_cond_broadcast(c)
#define pt_create(t, f, a) pw_thread_create(t, f, a)
#define pt_join(t) pw_thread_join(t)
#else
#include <pthread.h>
#define EXPORT __attribute__((visibility("default")))
typedef pthread_t pane_thread_t;
typedef pthread_mutex_t pane_mutex_t;
typedef pthread_cond_t pane_cond_t;
#define PANE_MUTEX_INIT PTHREAD_MUTEX_INITIALIZER
#define pm_init(m) pthread_mutex_init(m, NULL)
#define pm_lock(m) pthread_mutex_lock(m)
#define pm_unlock(m) pthread_mutex_unlock(m)
#define pc_init(c) pthread_cond_init(c, NULL)
#define pc_wait(c, m) pthread_cond_wait(c, m)
#define pc_broadcast(c) pthread_cond_broadcast(c)
#define pt_create(t, f, a) pthread_create(t, NULL, f, a)
#define pt_join(t) pthread_join(t, NULL)
#endif
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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
    pane_thread_t thread;
    pane_mutex_t lock;
    pane_cond_t started;
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

static pane_mutex_t only = PANE_MUTEX_INIT;
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

enum { FONT_SLOTS = 32 };

// The ranges loaded: Latin, punctuation, arrows, box drawing, blocks and shapes.
static const int ranges[][2] = {{32, 126}, {160, 255}, {0x2010, 0x2027}, {0x2190, 0x21FF}, {0x2500, 0x25FF}};

typedef struct Sized {
    int font_id;
    int size;
    Font font;
} Sized;

static Sized fonts[FONT_SLOTS];
static int fonts_used;
static int fonts_next;

// The font for `(font_id, size)` pixels, loaded once on first use; only the window thread calls this.
static Font font_for(int font_id, int size) {
    if (font_id < 0 || font_id >= PANE_FONT_COUNT) font_id = PANE_FONT_SANS;
    for (int at = 0; at < fonts_used; at++) {
        if (fonts[at].font_id == font_id && fonts[at].size == size) return fonts[at].font;
    }
    int count = 0;
    for (size_t range = 0; range < sizeof ranges / sizeof ranges[0]; range++) count += ranges[range][1] - ranges[range][0] + 1;
    int *codepoints = malloc((size_t)count * sizeof(int));
    if (!codepoints) return GetFontDefault();
    int filled = 0;
    for (size_t range = 0; range < sizeof ranges / sizeof ranges[0]; range++) {
        for (int codepoint = ranges[range][0]; codepoint <= ranges[range][1]; codepoint++) codepoints[filled++] = codepoint;
    }
    int data_len = 0;
    const uint8_t *data = pf_bytes(font_id, &data_len);
    int line = (int)((float)size * pf_line_ratio(font_id) + 0.5f);
    if (line < 1) line = 1;
    Font font = LoadFontFromMemory(".ttf", data, data_len, line, codepoints, count);
    free(codepoints);
    if (font.texture.id == 0) return GetFontDefault();
    SetTextureFilter(font.texture, TEXTURE_FILTER_BILINEAR);
    int slot = fonts_used < FONT_SLOTS ? fonts_used++ : fonts_next++ % FONT_SLOTS;
    if (fonts[slot].font.texture.id != 0) UnloadFont(fonts[slot].font);
    fonts[slot] = (Sized){font_id, size, font};
    return font;
}

static void draw_text(const char *text, int length, int64_t x, int64_t y, int size, int font_id, Color color) {
    char *copy = malloc((size_t)length + 1);
    if (!copy) return;
    memcpy(copy, text, (size_t)length);
    copy[length] = 0;
    Font font = font_for(font_id, size);
    // The line is taller than `size`; half of the extra goes above the text so it sits centred in its box.
    float top = (float)y - (float)(font.baseSize - size) / 2.0f;
    DrawTextEx(font, copy, (Vector2){(float)x, top}, (float)font.baseSize, 0.0f, color);
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
            draw_text((const char *)texts + c[8], (int)c[9], c[1], c[2], (int)c[10], (int)c[11], color);
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
    pm_lock(&window->lock);
    window->state = ready ? 1 : -1;
    pc_broadcast(&window->started);
    pm_unlock(&window->lock);
    if (!ready) return NULL;
    SetExitKey(0);
    int closing = 0;
    for (;;) {
        pm_lock(&window->lock);
        int stop = window->stop;
        pm_unlock(&window->lock);
        if (stop) break;
        BeginDrawing();
        ClearBackground((Color){255, 255, 255, 255});
        pm_lock(&window->lock);
        if (window->count > 0) paint(window->commands, window->count, window->texts);
        pm_unlock(&window->lock);
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
    pm_lock(&only);
    if (taken) {
        pm_unlock(&only);
        host.port_close(events);
        return fail(error, error_length, "a window is already open");
    }
    taken = 1;
    pm_unlock(&only);
    Window *window = calloc(1, sizeof(Window));
    window->width = (int)width;
    window->height = (int)height;
    window->title = strdup(title);
    window->events = events;
    pm_init(&window->lock);
    pc_init(&window->started);
    if (pt_create(&window->thread, run, window) != 0) {
        host.port_close(events);
        free(window->title);
        free(window);
        pm_lock(&only);
        taken = 0;
        pm_unlock(&only);
        return fail(error, error_length, "the window thread did not start");
    }
    pm_lock(&window->lock);
    while (window->state == 0) pc_wait(&window->started, &window->lock);
    int state = window->state;
    pm_unlock(&window->lock);
    if (state < 0) {
        pt_join(window->thread);
        host.port_close(events);
        free(window->title);
        free(window);
        pm_lock(&only);
        taken = 0;
        pm_unlock(&only);
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
    pm_lock(&window->lock);
    free(window->commands);
    free(window->texts);
    window->commands = copy;
    window->texts = text_copy;
    window->texts_length = texts_length;
    window->count = count;
    pm_unlock(&window->lock);
    return 0;
}

// Stops the thread of a window, closes it and gives the port back; a headless window only gives the port back.
EXPORT void pane_close(int64_t handle) {
    Window *window = (Window *)(intptr_t)handle;
    if (!window->headless) {
        pm_lock(&window->lock);
        window->stop = 1;
        pm_unlock(&window->lock);
        pt_join(window->thread);
        pm_lock(&only);
        taken = 0;
        pm_unlock(&only);
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

#ifndef _WIN32
EXPORT int64_t pane_dialog_open(const char *title, const char *filter_ext, uint8_t *out, int64_t out_length) {
    (void)title; (void)filter_ext; (void)out; (void)out_length;
    return -1;
}

EXPORT int64_t pane_dialog_save(const char *title, const char *filter_ext, const char *default_name, uint8_t *out, int64_t out_length) {
    (void)title; (void)filter_ext; (void)default_name; (void)out; (void)out_length;
    return -1;
}
#endif
