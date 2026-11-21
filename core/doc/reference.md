# doc: reference

A document as an arena of nodes: elements with attributes, text, comments and raw HTML. `markdown` builds one; `pane` lays one out. Native: Rust (the arena lives in the native; the byte form is stable).

`import { Doc, Node, NodeKind } from doc`

`NodeKind`: `Document Element Text Comment Raw`. A `Node` is a value (`doc`, `id`) into its `Doc`. Node 0 is the document node, which holds the top-level nodes; a node made by `element`, `text`, `comment` or `raw` is outside the tree until appended.

| `Doc` | Notes |
|---|---|
| `Doc.new()`, `Doc.from_bytes(bytes) -> Result<Doc, Error>` | `bytes()` is the byte form, read back equal |
| `.root() -> Node` | the document node |
| `.element(name)`, `.text(s)`, `.comment(s)`, `.raw(html)` | new nodes outside the tree |
| `.adopt(node)` | a copy of a node and below, from this or another document |
| `.len()` | all nodes, those outside the tree too |
| `.close()` | `Closeable`; closing twice is an `Err`, using a closed document is a fault |

| `Node` | Notes |
|---|---|
| `.kind()`, `.name()` ("" for non-elements), `.text()` (text and comments), `.text_content()` (every text under it) | |
| `.parent()`, `.child_count()`, `.child(i)`, `.children()`, `.index()` (-1 outside the tree), `.next()`, `.previous()` | |
| `.descendants()`, `.find(name)`, `.find_all(name)` | parents before children |
| `.attr(name) -> String?`, `.attrs() -> List<(String, String)>`, `.set_attr(name, value)`, `.remove_attr(name) -> Bool` | in the order set |
| `.rename(name)`, `.set_text(s)` | |
| `.append(child)`, `.insert_before(child, before)`, `.detach()` | moves a node out of its old place; `Err` when this node cannot have children or the child holds it |
| `.copy()`, `.same(other)` | |
| `.to_html()` | the node, or for the document node its children; nothing added between nodes; void elements `<br />` |

```mote
import { Doc } from doc

fn main() -> Result<Null, Error> {
    let doc = Doc.new()
    let list = doc.element("ul")
    for word in ["a", "b"] {
        let item = doc.element("li")
        item.append(doc.text(word))?
        list.append(item)?
    }
    doc.root().append(list)?
    println(doc.root().to_html())
    return Ok(null)
}
```

Tests: `tools/test-core.sh doc`.
