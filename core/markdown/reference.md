# markdown: reference

CommonMark to a `doc` tree, to HTML, and a `doc` tree back to Markdown. Native: a Rust crate (`pulldown-cmark`) behind the `doc` arena.

`import { parse, render, parse_with, render_with, to_markdown, github, commonmark, Extensions } from markdown`

| Function | Notes |
|---|---|
| `parse(text) -> Result<Doc, Error>` | CommonMark; `Err` for text that is not UTF-8 |
| `parse_with(text, extensions)` | raw HTML in the text becomes `Raw` nodes |
| `render(text) -> Result<String, Error>` | the HTML |
| `render_with(text, extensions)` | |
| `to_markdown(node: Node) -> String` | the document node or an element, ending in a newline; an element with no Markdown form is written as HTML |
| `commonmark()` | `Extensions` with all off |
| `github()` | tables, strikethrough and task lists on |
| `Extensions { tables, strikethrough, tasklists, smart_punctuation }` | `Bool`s |

Elements the tree uses are the HTML names (`h1`, `p`, `ul`, `li`, `a` with `href`, `code`, `pre`, ...); walk it with `doc`'s `Node` (`children()`, `find_all("a")`, `attr("href")`).

```mote
import { github, parse_with, render, to_markdown } from markdown

fn main() -> Result<Null, Error> {
    println(render("# Title\n\nsome *text*")?)
    let tree = parse_with("| a | b |\n|---|---|\n| 1 | 2 |\n", github())?
    println(tree.root().find_all("td").len())
    println(to_markdown(tree.root()))
    return Ok(null)
}
```

Tests: `tools/test-core.sh markdown`.
