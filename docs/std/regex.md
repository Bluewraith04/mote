# std.regex

A small backtracking regex engine. `import { Regex } from std.regex`.

It matches over bytes, not Unicode scalar values: `.` and a class match one byte, and `\w`, `\d`, `\s` are ASCII-only. A literal pattern still matches non-ASCII text correctly.

| Syntax | Meaning |
|---|---|
| `.` | any byte |
| `[abc]`, `[^abc]`, `[a-z]` | a class, its negation, ranges |
| `\d \D \w \W \s \S` | ASCII classes, usable inside `[...]` |
| `^` `$` | start and end of the whole string |
| `\b` `\B` | word boundary, non-boundary |
| `(...)`, `(?:...)`, `(?<name>...)` | capturing, non-capturing and named groups |
| `a\|b` | alternation |
| `* + ? {m} {m,} {m,n}` | quantifiers; a trailing `?` makes one lazy |

No backreferences, lookahead or lookbehind. A repeat count above 1000 is an error.

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

`Regex.compile` answers `Result<Regex, Error>`; the `Err` names the first syntax problem and its byte offset.

| Method | Answers |
|---|---|
| `is_match(text)` | whether the pattern matches anywhere |
| `find(text)` | the leftmost `Match?` |
| `captures(text)` | the leftmost match's `Captures?` |
| `find_all(text)` | every non-overlapping `Match` |
| `split(text)` | `text` cut around every match |
| `replace_all(text, with)` | every match replaced by `with`, taken literally |

A `Match` has `start`, `end` (byte offsets) and `text`. `Captures` has `get(i)`, `get_named(name)` and `len()`; group 0 is the whole match.

The engine backtracks, so a pathological pattern such as `(a*)*b` on a long non-matching input is slow. It is not suited to untrusted patterns on untrusted input.
