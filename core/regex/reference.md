# regex: reference

Regular expressions in the syntax of the Rust `regex` crate (no look-around or back-references; linear time). Native: a Rust crate.

`import { Regex, Captures, Match } from regex`

| Item | Notes |
|---|---|
| `Regex.new(pattern) -> Result<Regex, Error>` | `Err` says what is wrong; `Closeable` (close frees it; using it after is a fault) |
| `.pattern()`, `.group_names()` | names in order, `""` for an unnamed group, index 0 is the whole match |
| `.is_match(text) -> Bool` | anywhere in the text |
| `.find(text) -> Match?`, `.find_all(text) -> List<Match>` | non-overlapping, left to right; an empty match next to the one before is skipped |
| `.captures(text) -> Captures?`, `.captures_from(text, start)`, `.captures_all(text)` | with groups |
| `.replace(text, template)` / `.replace_all(text, template)` | the template has `$1`, `${1}`, `$name`, `${name}` and `$$` |
| `.split(text) -> List<String>` | the pieces between matches |
| `Match { start, end, text }` | byte offsets: `start` first byte, `end` the byte after the last |
| `Captures` | `.len()` (groups, 0 included), `.group(i) -> Match?` (null if it took no part), `.get(i) -> String?`, `.named(name) -> String?`, `.whole() -> Match`, `.expand(template) -> String` |

Offsets are in bytes, not characters.

```mote
import { Regex } from regex

fn main() -> Result<Null, Error> {
    let re = Regex.new("(?P<key>[a-z]+)=(?P<value>\\d+)")?
    let found = re.captures("size=42")!
    println("${found.named("key")!} -> ${found.named("value")!}")
    println(re.replace_all("a=1 b=2", "$value:$key"))
    re.close()?
    return Ok(null)
}
```

Tests: `tools/test-core.sh regex`.
