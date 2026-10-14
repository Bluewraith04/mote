# std.regex

Regular expressions over the Rust `regex` crate. `import { Regex } from std.regex`.

It matches Unicode code points: `.` and a class match one character, and `\w`, `\d`, `\s` and `\b` are Unicode. Time is linear in the text, whatever the pattern.

| Syntax | Meaning |
|---|---|
| `.` | any character except a line break (`(?s)` includes it) |
| `[abc]`, `[^abc]`, `[a-z]`, `[[:alpha:]]` | a class, its negation, ranges, POSIX names |
| `\d \D \w \W \s \S` | Unicode classes, usable inside `[...]` |
| `\p{L}`, `\P{L}` | a general category |
| `^` `$` | start and end of the text (of each line with `(?m)`) |
| `\b` `\B` | word boundary, non-boundary |
| `(...)`, `(?:...)`, `(?<name>...)` | capturing, non-capturing and named groups |
| `a\|b` | alternation |
| `* + ? {m} {m,} {m,n}` | quantifiers; a trailing `?` makes one lazy |
| `(?i) (?m) (?s) (?x) (?U)` | flags: ignore case, multi-line, `.` matches line breaks, free spacing, swap greedy and lazy |

No backreferences or lookaround (a compile error), and no scripts: `\p{Greek}` is a compile error.

```mote
import { Regex } from std.regex

let re = Regex.compile("(\\d+)-(\\d+)")!
println(re.is_match("id 12-345"))
println(re.find("id 12-345")!.text)

let caps = re.captures("id 12-345")!
println(caps.get(1)!.text)
println(caps.get(2)!.text)
```

```output
true
12-345
12
345
```

`Regex.compile` answers `Result<Regex, Error>`; the `Err` carries the compiler's reason in one line.

| Method | Answers |
|---|---|
| `is_match(text)` | whether the pattern matches anywhere |
| `find(text)` | the leftmost `Match?` |
| `find_from(text, start)` | the leftmost `Match?` at byte offset `start` or later; the text before `start` still counts for `^` and `\b` |
| `captures(text)` | the leftmost match's `Captures?` |
| `find_all(text)` | every non-overlapping `Match` |
| `split(text)` | `text` cut around every match |
| `replace_all(text, with)` | every match replaced by `with`, taken literally |

A `Match` has `start`, `end` (byte offsets) and `text`. `Captures` has `get(i)`, `get_named(name)` and `len()`; group 0 is the whole match.

An empty match is never reported inside a character, and `find_all` skips an empty match right after a match.

A program that imports `std.regex` is built on the full runtime, since the engine adds about 1 MB to the binary.
