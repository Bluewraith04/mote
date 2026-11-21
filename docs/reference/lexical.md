# Lexical

## Source

* A file is UTF-8 text. <!-- test: graph_load_a_source_file_that_is_not_utf8_is_an_error_naming_the_file -->

## Comments

* `//` runs to the end of the line. <!-- test: comments_are_kept_outside_the_tokens -->

```mote
let a = 1 // note
```

* `/* */` nests. <!-- test: comments_are_kept_outside_the_tokens, an_unterminated_block_comment_is_an_error -->

```mote
/* a /* b */ c */
```

* `///` alone on a line documents the next item, field, variant or method. `////` and a trailing `///` are plain comments. <!-- test: comments_are_kept_outside_the_tokens, docs_attach_to_the_item_below, docs_attach_to_fields_variants_and_methods -->

```mote
/// Adds one.
fn inc(n: Int) -> Int { return n + 1 }
```

## Names

* A name matches `[a-zA-Z_][a-zA-Z0-9_]*`. A lone `_` is the wildcard pattern. <!-- test: identifiers_keep_their_text, an_unexpected_character_is_an_error_token_and_lexing_goes_on, grammar_every_case_gives_the_result_it_records -->

```mote
let _count = 1
```

* `test`, `Send` and `_` are ordinary names that mean something only in one position: `test "name" {` at item level, `Send (…) -> T` in a type, `_` in a pattern. <!-- test: grammar_every_case_gives_the_result_it_records -->
* `impl`, `mut` and `try` are ordinary names. <!-- test: retired_words_are_identifiers -->

## Keywords

<!-- table: keywords -->
* Reserved: `Self` `as` `break` `class` `comptime` `const` `continue` `else` `enum` `extend` `extern` `false` `fn` `for` `from` `if` `import` `in` `is` `let` `match` `null` `pub` `return` `scope` `self` `spawn` `struct` `super` `trait` `true` `type` `var` `while` `with` `yield`.
<!-- /table -->
* A keyword cannot be a name or follow a `.`, except `from`, which may name a method. <!-- test: keywords_are_not_identifiers, grammar_every_case_gives_the_result_it_records -->

```mote
let x = Float.from(3)
```

## Statement ends

* There are no semicolons required. A newline outside any bracket ends a statement when the previous token is a name, literal, `return`, `break`, `continue`, `yield`, `)`, `]`, `}`, `?` or `!`. <!-- test: a_newline_ends_a_statement_after_each_listed_token_and_after_no_other, a_newline_ends_a_statement_after_a_closing_token, newlines_inside_brackets_are_ignored -->

```mote
let a = 1
let b = 2
```

* A trailing operator continues the line. <!-- test: a_trailing_operator_continues_the_line, a_newline_ends_a_statement_after_each_listed_token_and_after_no_other -->

```mote
let c = a +
  b
```

* A line starting with `.` or `?.` continues the previous line. <!-- test: a_line_starting_with_a_member_access_continues_the_previous_one, a_member_access_line_continues_a_top_level_chain -->

```mote
let n = xs
  .map(f)
  .len()
```

* Inside `( )`, `[ ]` and `{ }` newlines are ignored and statements need no separator; `;` may separate them. An expression ends where the grammar cannot continue it. A `(` that starts a line is never a call, so in a match, `(x, y) => …` on the next line begins a new arm. <!-- test: newlines_inside_brackets_are_ignored, statements_in_a_block_need_no_separator, a_paren_on_a_new_line_is_not_a_call -->

```mote
fn f() -> Int {
    let a = 1
    let b = 2
    return a + b
}
```

* `return` and `break` take the next expression if one starts right after them, so put nothing after a bare `return` or `break` on the same block level except `}`. <!-- test: control_flow_statements -->

## Integer literals

* Decimal, `0x` hex, `0b` binary, `0o` octal; `_` separates digits. Prefixes are lowercase. The value must fit 128 bits. <!-- test: integers_carry_value_radix_and_suffix, malformed_numbers_are_one_error_token, the_largest_integer_fits_and_one_more_does_not -->

```mote
let n = 0xFF_FF
```

* A suffix `i8 i16 i32 i64 u8 u16 u32 u64` gives the sized type; `_` may precede it. <!-- test: integers_carry_value_radix_and_suffix -->

```mote
let x = 128u8
let y = 1_000_i32
```

## Float literals

* Digits with a `.` and digits, or an exponent; suffix `f32` or `f64`. <!-- test: floats_carry_value_and_suffix -->

```mote
let a = 1.5
let b = 2.5e-3
let c = 1.5f32
```

* `1.max(2)` and `1..2` do not lex as floats. After a `.`, digits are a tuple index: `t.0.1`. <!-- test: a_dot_after_digits_is_not_always_a_float, tuple_indexes_after_a_dot_are_plain_integers -->

## Strings

* `"…"` is UTF-8 and may span lines. There are no character literals and no raw strings; `'` is not a token. <!-- test: strings_decode_escapes, strings_may_span_lines, a_prefix_does_not_make_a_raw_string, a_single_quote_is_not_a_token -->
* Escapes: `\0 \n \r \t \\ \' \" \$ \u{…}`. `\x` is an error. <!-- test: strings_decode_escapes, bad_escapes_are_reported_and_dropped -->

```mote
let s = "tab\there \u{1F600}"
```

* `${expr}` inserts `expr.to_string()`. A string may appear inside the braces. `\${` is literal. <!-- block -->

```mote
fn shout(s: String) -> String {
    return s + "!"
}

fn main() {
    let n = 1
    println("n=${n + 1} ${shout("x")} \${n}")
}
```

```output
n=2 x! ${n}
```

## Operators and punctuation

* `+ - * / % & | ^ ~ ! << >> == != < <= > >= && || ?? ?. = += -= *= /= %= &= |= ^= <<= >>= ??= -> => :: . .. ..= ... @ ( ) [ ] { } , : ; ?` <!-- test: operators_take_the_longest_match, single_character_punctuation -->
* `::` and `super` are reserved and have no use outside import paths. <!-- test: colon_colon_and_super_have_no_use_outside_import_paths -->
