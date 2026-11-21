// Layout for pane: a flat list of element records goes through clay and comes back as a draw list.
#define CLAY_IMPLEMENTATION
#include "pane_fonts.h"
#include "vendor/clay.h"
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#define EXPORT __declspec(dllexport)
#else
#include <pthread.h>
#define EXPORT __attribute__((visibility("default")))
#endif
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

enum { OP_WIDTH = 24, CMD_WIDTH = 12, BOX_WIDTH = 6 };
enum { OP_OPEN = 1, OP_TEXT = 2, OP_CLOSE = 3 };
enum { CMD_RECT = 1, CMD_TEXT = 2, CMD_BORDER = 3, CMD_CLIP = 4, CMD_UNCLIP = 5 };

#ifdef _WIN32
static SRWLOCK lock = SRWLOCK_INIT;
#define LAYOUT_LOCK() AcquireSRWLockExclusive(&lock)
#define LAYOUT_UNLOCK() ReleaseSRWLockExclusive(&lock)
#else
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
#define LAYOUT_LOCK() pthread_mutex_lock(&lock)
#define LAYOUT_UNLOCK() pthread_mutex_unlock(&lock)
#endif
static void *memory = NULL;
static Clay_Context *context = NULL;
static char problem[256];
static int problem_length = 0;

static Clay_Dimensions measure(Clay_StringSlice text, Clay_TextElementConfig *config, void *user) {
    (void)user;
    float width = pf_measure((int)config->fontId, (int)config->fontSize, (int)config->letterSpacing, text.chars, text.length);
    int height = config->lineHeight ? config->lineHeight : config->fontSize;
    return (Clay_Dimensions){width, (float)height};
}

static void failed(Clay_ErrorData error) {
    int length = error.errorText.length < (int)sizeof problem ? error.errorText.length : (int)sizeof problem;
    if (problem_length == 0) {
        memcpy(problem, error.errorText.chars, (size_t)length);
        problem_length = length;
    }
}

static Clay_Color color_of(int64_t packed) {
    return (Clay_Color){(float)((packed >> 24) & 255), (float)((packed >> 16) & 255), (float)((packed >> 8) & 255), (float)(packed & 255)};
}

static int64_t pack_of(Clay_Color color) {
    return ((int64_t)color.r << 24) | ((int64_t)color.g << 16) | ((int64_t)color.b << 8) | (int64_t)color.a;
}

static Clay_SizingAxis sizing_of(int64_t type, int64_t value) {
    if (type == 1) return (Clay_SizingAxis){.size = {.minMax = {0, 0}}, .type = CLAY__SIZING_TYPE_GROW};
    if (type == 2) return (Clay_SizingAxis){.size = {.minMax = {(float)value, (float)value}}, .type = CLAY__SIZING_TYPE_FIXED};
    if (type == 3) return (Clay_SizingAxis){.size = {.percent = (float)value / 1000.0f}, .type = CLAY__SIZING_TYPE_PERCENT};
    return (Clay_SizingAxis){.size = {.minMax = {0, 0}}, .type = CLAY__SIZING_TYPE_FIT};
}

static Clay_LayoutAlignmentX align_x(int64_t v) { return v == 1 ? CLAY_ALIGN_X_CENTER : v == 2 ? CLAY_ALIGN_X_RIGHT : CLAY_ALIGN_X_LEFT; }
static Clay_LayoutAlignmentY align_y(int64_t v) { return v == 1 ? CLAY_ALIGN_Y_CENTER : v == 2 ? CLAY_ALIGN_Y_BOTTOM : CLAY_ALIGN_Y_TOP; }

static int ready(void) {
    if (context) return 1;
    uint32_t size = Clay_MinMemorySize();
    memory = malloc(size);
    if (!memory) return 0;
    Clay_Arena arena = Clay_CreateArenaWithCapacityAndMemory(size, memory);
    context = Clay_Initialize(arena, (Clay_Dimensions){800, 600}, (Clay_ErrorHandler){failed, NULL});
    Clay_SetMeasureTextFunction(measure, NULL);
    return context != NULL;
}

static int error_out(const char *message, uint8_t *texts, int64_t capacity) {
    int length = (int)strlen(message);
    if (length > capacity) length = (int)capacity;
    memcpy(texts, message, (size_t)length);
    return -length - 1;
}

// Lays out `count` records of `ops` in a `width` by `height` window.
// `info` gets the number of commands, the bytes of text and the number of boxes; those are written only up to the capacities given.
// Returns 0, or `-length - 1` with a message in `texts`.
EXPORT int64_t pane_layout(const int64_t *ops, int64_t ops_length, const uint8_t *strings, int64_t strings_length, int64_t width, int64_t height,
                           int64_t *commands, int64_t commands_length, uint8_t *texts, int64_t texts_length, int64_t *boxes, int64_t boxes_length,
                           int64_t *info, int64_t info_length) {
    (void)strings_length;
    (void)info_length;
    LAYOUT_LOCK();
    int64_t result = 0;
    if (!ready()) {
        result = error_out("out of memory", texts, texts_length);
        goto done;
    }
    problem_length = 0;
    Clay_SetCurrentContext(context);
    Clay_SetLayoutDimensions((Clay_Dimensions){(float)width, (float)height});
    Clay_BeginLayout();
    int64_t named = 0;
    for (int64_t at = 0; at + OP_WIDTH <= ops_length; at += OP_WIDTH) {
        const int64_t *op = ops + at;
        if (op[0] == OP_OPEN) {
            Clay_ElementDeclaration declaration = {0};
            declaration.layout.layoutDirection = op[1] == 1 ? CLAY_TOP_TO_BOTTOM : CLAY_LEFT_TO_RIGHT;
            declaration.layout.sizing.width = sizing_of(op[2], op[3]);
            declaration.layout.sizing.height = sizing_of(op[4], op[5]);
            declaration.layout.padding = (Clay_Padding){(uint16_t)op[6], (uint16_t)op[8], (uint16_t)op[7], (uint16_t)op[9]};
            declaration.layout.childGap = (uint16_t)op[10];
            declaration.layout.childAlignment = (Clay_ChildAlignment){align_x(op[11]), align_y(op[12])};
            if (op[13] >= 0) declaration.backgroundColor = color_of(op[13]);
            declaration.cornerRadius = (Clay_CornerRadius){(float)op[14], (float)op[14], (float)op[14], (float)op[14]};
            if (op[15] > 0) {
                declaration.border.width = (Clay_BorderWidth){(uint16_t)op[15], (uint16_t)op[15], (uint16_t)op[15], (uint16_t)op[15], 0};
                declaration.border.color = color_of(op[16]);
            }
            if (op[18] > 0) {
                Clay_String name = {false, (int32_t)op[18], (const char *)strings + op[17]};
                declaration.id = Clay__HashString(name, 0, 0);
                named++;
            }
            if (op[19]) declaration.clip = (Clay_ClipElementConfig){true, true, {0, 0}};
            Clay__OpenElement();
            Clay__ConfigureOpenElement(declaration);
        } else if (op[0] == OP_TEXT) {
            Clay_TextElementConfig config = {0};
            config.textColor = color_of(op[4]);
            config.fontSize = (uint16_t)op[3];
            config.wrapMode = op[5] == 1 ? CLAY_TEXT_WRAP_NONE : CLAY_TEXT_WRAP_WORDS;
            config.lineHeight = (uint16_t)op[6];
            config.textAlignment = op[7] == 1 ? CLAY_TEXT_ALIGN_CENTER : op[7] == 2 ? CLAY_TEXT_ALIGN_RIGHT : CLAY_TEXT_ALIGN_LEFT;
            config.fontId = (uint16_t)(op[8] >= 0 && op[8] < PANE_FONT_COUNT ? op[8] : PANE_FONT_SANS);
            Clay_String text = {false, (int32_t)op[2], (const char *)strings + op[1]};
            Clay__OpenTextElement(text, Clay__StoreTextElementConfig(config));
        } else if (op[0] == OP_CLOSE) {
            Clay__CloseElement();
        }
    }
    Clay_RenderCommandArray list = Clay_EndLayout();
    if (problem_length > 0) {
        result = error_out(problem, texts, texts_length);
        goto done;
    }
    int64_t text_bytes = 0;
    for (int32_t index = 0; index < list.length; index++) {
        Clay_RenderCommand *command = Clay_RenderCommandArray_Get(&list, index);
        int64_t *row = commands + (int64_t)index * CMD_WIDTH;
        int fits = (int64_t)(index + 1) * CMD_WIDTH <= commands_length;
        int64_t record[CMD_WIDTH] = {0};
        record[1] = (int64_t)(command->boundingBox.x + 0.5f);
        record[2] = (int64_t)(command->boundingBox.y + 0.5f);
        record[3] = (int64_t)(command->boundingBox.width + 0.5f);
        record[4] = (int64_t)(command->boundingBox.height + 0.5f);
        switch (command->commandType) {
        case CLAY_RENDER_COMMAND_TYPE_RECTANGLE:
            record[0] = CMD_RECT;
            record[5] = pack_of(command->renderData.rectangle.backgroundColor);
            record[6] = (int64_t)command->renderData.rectangle.cornerRadius.topLeft;
            break;
        case CLAY_RENDER_COMMAND_TYPE_BORDER:
            record[0] = CMD_BORDER;
            record[5] = pack_of(command->renderData.border.color);
            record[6] = (int64_t)command->renderData.border.cornerRadius.topLeft;
            record[7] = command->renderData.border.width.left;
            break;
        case CLAY_RENDER_COMMAND_TYPE_TEXT: {
            Clay_TextRenderData *text = &command->renderData.text;
            record[0] = CMD_TEXT;
            record[5] = pack_of(text->textColor);
            record[8] = text_bytes;
            record[9] = text->stringContents.length;
            record[10] = text->fontSize;
            record[11] = text->fontId;
            if (text_bytes + text->stringContents.length <= texts_length) memcpy(texts + text_bytes, text->stringContents.chars, (size_t)text->stringContents.length);
            text_bytes += text->stringContents.length;
            break;
        }
        case CLAY_RENDER_COMMAND_TYPE_SCISSOR_START: record[0] = CMD_CLIP; break;
        case CLAY_RENDER_COMMAND_TYPE_SCISSOR_END: record[0] = CMD_UNCLIP; break;
        default: break;
        }
        if (fits) memcpy(row, record, sizeof record);
    }
    info[0] = list.length;
    info[1] = text_bytes;
    info[2] = named;
    // Every element with an id: where it ended up.
    int64_t found = 0;
    for (int64_t at = 0; at + OP_WIDTH <= ops_length; at += OP_WIDTH) {
        const int64_t *op = ops + at;
        if (op[0] != OP_OPEN || op[18] <= 0) continue;
        Clay_String name = {false, (int32_t)op[18], (const char *)strings + op[17]};
        Clay_ElementData data = Clay_GetElementData(Clay__HashString(name, 0, 0));
        if ((found + 1) * BOX_WIDTH <= boxes_length) {
            int64_t *row = boxes + found * BOX_WIDTH;
            row[0] = op[17];
            row[1] = op[18];
            row[2] = (int64_t)(data.boundingBox.x + 0.5f);
            row[3] = (int64_t)(data.boundingBox.y + 0.5f);
            row[4] = (int64_t)(data.boundingBox.width + 0.5f);
            row[5] = (int64_t)(data.boundingBox.height + 0.5f);
        }
        found++;
    }
done:
    LAYOUT_UNLOCK();
    return result;
}
