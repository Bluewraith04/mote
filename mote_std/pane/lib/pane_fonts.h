#ifndef PANE_FONTS_H
#define PANE_FONTS_H

#include <stdint.h>

enum {
    PANE_FONT_SANS = 0,
    PANE_FONT_BOLD = 1,
    PANE_FONT_SERIF = 2,
    PANE_FONT_MONO = 3,
    PANE_FONT_COUNT = 4
};

// Returns the embedded TTF byte buffer and byte length for `font_id` (clamped to Sans on out-of-range).
const uint8_t *pf_bytes(int font_id, int *length_out);

// Returns (ascent - descent) / units_per_em so raylib's line-sized font matches an em height of `size`.
float pf_line_ratio(int font_id);

// Measures UTF-8 text width in pixels for `font_id` at `em_size` using the face's glyph advances.
float pf_measure(int font_id, int em_size, int letter_spacing, const char *text, int length);

#endif
