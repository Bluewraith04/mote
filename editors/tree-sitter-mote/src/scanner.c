// External tokens of the Mote grammar: nested block comments, a call's `(` on the line of its callee, and the contextual words `Send` and `test`.
#include "tree_sitter/parser.h"

#include <stdbool.h>
#include <stdint.h>

enum TokenType { BLOCK_COMMENT, CALL_PAREN, SEND_WORD, TEST_WORD, STRING_CONTENT, ERROR_SENTINEL };

void *tree_sitter_mote_external_scanner_create(void) { return NULL; }
void tree_sitter_mote_external_scanner_destroy(void *payload) { (void)payload; }
unsigned tree_sitter_mote_external_scanner_serialize(void *payload, char *buffer) {
    (void)payload;
    (void)buffer;
    return 0;
}
void tree_sitter_mote_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
    (void)payload;
    (void)buffer;
    (void)length;
}

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }
static void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

static bool is_word(int32_t c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_'; }

// After `/*`: reads to the matching `*/`, nesting; false when the comment is never closed.
static bool block_comment_rest(TSLexer *lexer) {
    int depth = 1;
    while (depth > 0) {
        if (lexer->eof(lexer)) return false;
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '/' && lexer->lookahead == '*') {
            advance(lexer);
            depth++;
        } else if (c == '*' && lexer->lookahead == '/') {
            advance(lexer);
            depth--;
        }
    }
    return true;
}

// Reads `word` when it is the whole word; false when something else is here.
static bool read_word(TSLexer *lexer, const char *word) {
    for (const char *at = word; *at; at++) {
        if (lexer->lookahead != *at) return false;
        advance(lexer);
    }
    return !is_word(lexer->lookahead);
}

// The text of a string up to a quote, a backslash or a `${`; a `$` that does not open a hole is text.
static bool string_content(TSLexer *lexer) {
    bool any = false;
    for (;;) {
        int32_t c = lexer->lookahead;
        if (lexer->eof(lexer) || c == '"' || c == '\\') break;
        if (c == '$') {
            lexer->mark_end(lexer);
            advance(lexer);
            if (lexer->lookahead == '{') {
                lexer->result_symbol = STRING_CONTENT;
                return any;
            }
            any = true;
            continue;
        }
        advance(lexer);
        any = true;
    }
    lexer->mark_end(lexer);
    lexer->result_symbol = STRING_CONTENT;
    return any;
}

bool tree_sitter_mote_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
    (void)payload;
    if (valid_symbols[ERROR_SENTINEL]) return false;
    if (valid_symbols[STRING_CONTENT]) return string_content(lexer);
    bool crossed_line = false;
    for (;;) {
        int32_t c = lexer->lookahead;
        if (c == ' ' || c == '\t' || c == '\r') {
            skip(lexer);
        } else if (c == '\n') {
            crossed_line = true;
            skip(lexer);
        } else {
            break;
        }
    }
    int32_t c = lexer->lookahead;
    if (c == '(' && valid_symbols[CALL_PAREN] && !crossed_line) {
        advance(lexer);
        lexer->mark_end(lexer);
        lexer->result_symbol = CALL_PAREN;
        return true;
    }
    if (c == '/' && valid_symbols[BLOCK_COMMENT]) {
        advance(lexer);
        if (lexer->lookahead != '*') return false;
        advance(lexer);
        if (!block_comment_rest(lexer)) return false;
        lexer->mark_end(lexer);
        lexer->result_symbol = BLOCK_COMMENT;
        return true;
    }
    if (c == 'S' && valid_symbols[SEND_WORD]) {
        if (!read_word(lexer, "Send")) return false;
        lexer->mark_end(lexer);
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') advance(lexer);
        if (lexer->lookahead != '(') return false;
        lexer->result_symbol = SEND_WORD;
        return true;
    }
    if (c == 't' && valid_symbols[TEST_WORD]) {
        if (!read_word(lexer, "test")) return false;
        lexer->mark_end(lexer);
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t') advance(lexer);
        if (lexer->lookahead != '"') return false;
        lexer->result_symbol = TEST_WORD;
        return true;
    }
    return false;
}
