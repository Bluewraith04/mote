// Embedded DejaVu font set (Sans, Sans Bold, Serif, Sans Mono) and TrueType advance measurement.
#include "pane_fonts.h"
#include <math.h>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <pthread.h>
#endif

#define STBTT_STATIC
#define STB_TRUETYPE_IMPLEMENTATION
#include "vendor/raylib/src/external/stb_truetype.h"

#ifdef _WIN32
extern const unsigned char font_sans_data[];
extern const unsigned int font_sans_size;
extern const unsigned char font_bold_data[];
extern const unsigned int font_bold_size;
extern const unsigned char font_serif_data[];
extern const unsigned int font_serif_size;
extern const unsigned char font_mono_data[];
extern const unsigned int font_mono_size;
#else
__asm__(
    ".section .rodata\n"
    ".balign 4\nfont_sans_data:\n.incbin \"" PANE_FONT_SANS_PATH "\"\nfont_sans_end:\n"
    ".balign 4\nfont_bold_data:\n.incbin \"" PANE_FONT_BOLD_PATH "\"\nfont_bold_end:\n"
    ".balign 4\nfont_serif_data:\n.incbin \"" PANE_FONT_SERIF_PATH "\"\nfont_serif_end:\n"
    ".balign 4\nfont_mono_data:\n.incbin \"" PANE_FONT_MONO_PATH "\"\nfont_mono_end:\n"
    ".previous\n"
);

extern const unsigned char font_sans_data[] __attribute__((visibility("hidden")));
extern const unsigned char font_sans_end[] __attribute__((visibility("hidden")));
extern const unsigned char font_bold_data[] __attribute__((visibility("hidden")));
extern const unsigned char font_bold_end[] __attribute__((visibility("hidden")));
extern const unsigned char font_serif_data[] __attribute__((visibility("hidden")));
extern const unsigned char font_serif_end[] __attribute__((visibility("hidden")));
extern const unsigned char font_mono_data[] __attribute__((visibility("hidden")));
extern const unsigned char font_mono_end[] __attribute__((visibility("hidden")));
#endif

typedef struct FontFace {
    const unsigned char *data;
    const unsigned char *end;
    stbtt_fontinfo info;
    float line_ratio;
    int ready;
} FontFace;

#ifdef _WIN32
static INIT_ONCE fonts_once = INIT_ONCE_STATIC_INIT;
#else
static pthread_once_t fonts_once = PTHREAD_ONCE_INIT;
#endif
static FontFace faces[PANE_FONT_COUNT];

static void init_faces(void) {
    faces[PANE_FONT_SANS].data = font_sans_data;
    faces[PANE_FONT_BOLD].data = font_bold_data;
    faces[PANE_FONT_SERIF].data = font_serif_data;
    faces[PANE_FONT_MONO].data = font_mono_data;
#ifdef _WIN32
    faces[PANE_FONT_SANS].end = font_sans_data + font_sans_size;
    faces[PANE_FONT_BOLD].end = font_bold_data + font_bold_size;
    faces[PANE_FONT_SERIF].end = font_serif_data + font_serif_size;
    faces[PANE_FONT_MONO].end = font_mono_data + font_mono_size;
#else
    faces[PANE_FONT_SANS].end = font_sans_end;
    faces[PANE_FONT_BOLD].end = font_bold_end;
    faces[PANE_FONT_SERIF].end = font_serif_end;
    faces[PANE_FONT_MONO].end = font_mono_end;
#endif

    for (int id = 0; id < PANE_FONT_COUNT; id++) {
        FontFace *face = &faces[id];
        if (stbtt_InitFont(&face->info, face->data, 0)) {
            int ascent = 0, descent = 0, line_gap = 0;
            stbtt_GetFontVMetrics(&face->info, &ascent, &descent, &line_gap);
            float ratio = (float)(ascent - descent) * stbtt_ScaleForMappingEmToPixels(&face->info, 1.0f);
            face->line_ratio = ratio > 0.1f ? ratio : 1.1641f;
            face->ready = 1;
        } else {
            face->line_ratio = 1.1641f;
            face->ready = 0;
        }
    }
}

#ifdef _WIN32
static BOOL CALLBACK init_faces_win(PINIT_ONCE once, PVOID param, PVOID *ctx) {
    (void)once; (void)param; (void)ctx;
    init_faces();
    return TRUE;
}
static void ensure_faces(void) {
    InitOnceExecuteOnce(&fonts_once, init_faces_win, NULL, NULL);
}
#else
static void ensure_faces(void) {
    pthread_once(&fonts_once, init_faces);
}
#endif

static int clamp_id(int font_id) {
    return (font_id >= 0 && font_id < PANE_FONT_COUNT) ? font_id : PANE_FONT_SANS;
}

const uint8_t *pf_bytes(int font_id, int *length_out) {
    ensure_faces();
    FontFace *face = &faces[clamp_id(font_id)];
    if (length_out) *length_out = (int)(face->end - face->data);
    return face->data;
}

float pf_line_ratio(int font_id) {
    ensure_faces();
    return faces[clamp_id(font_id)].line_ratio;
}

static int next_codepoint(const char *s, int len, int *offset) {
    int i = *offset;
    unsigned char c = (unsigned char)s[i];
    if (c < 0x80) {
        *offset = i + 1;
        return c;
    }
    if ((c & 0xE0) == 0xC0 && i + 1 < len) {
        *offset = i + 2;
        return ((c & 0x1F) << 6) | ((unsigned char)s[i + 1] & 0x3F);
    }
    if ((c & 0xF0) == 0xE0 && i + 2 < len) {
        *offset = i + 3;
        return ((c & 0x0F) << 12) | (((unsigned char)s[i + 1] & 0x3F) << 6) | ((unsigned char)s[i + 2] & 0x3F);
    }
    if ((c & 0xF8) == 0xF0 && i + 3 < len) {
        *offset = i + 4;
        return ((c & 0x07) << 18) | (((unsigned char)s[i + 1] & 0x3F) << 12) | (((unsigned char)s[i + 2] & 0x3F) << 6) | ((unsigned char)s[i + 3] & 0x3F);
    }
    *offset = i + 1;
    return c;
}

float pf_measure(int font_id, int em_size, int letter_spacing, const char *text, int length) {
    ensure_faces();
    FontFace *face = &faces[clamp_id(font_id)];
    if (!face->ready || em_size <= 0) {
        int glyphs = 0;
        for (int at = 0; at < length; at++) {
            if (((unsigned char)text[at] & 0xC0) != 0x80) glyphs++;
        }
        int adv = (em_size * 6 + 5) / 10;
        return (float)(glyphs * (adv + letter_spacing));
    }
    int line_px = (int)((float)em_size * face->line_ratio + 0.5f);
    if (line_px < 1) line_px = 1;
    float scale = stbtt_ScaleForPixelHeight(&face->info, (float)line_px);
    float width = 0.0f;
    int offset = 0;
    while (offset < length) {
        int cp = next_codepoint(text, length, &offset);
        int adv = 0;
        stbtt_GetCodepointHMetrics(&face->info, cp, &adv, NULL);
        // Match raylib LoadFontFromMemory's integer glyph advanceX truncation.
        int px = (int)((float)adv * scale);
        width += (float)px + (float)letter_spacing;
    }
    return width;
}
