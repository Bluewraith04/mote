# Modules and packages

Every `.mote` file is a module. A path names a file without its extension.

## Importing

| Written | Brings in |
|---|---|
| `import .helper` | the sibling `helper.mote`, used as `helper.f()` |
| `import .sub.worker` | `sub/worker.mote` |
| `import ..util` | `util.mote` one directory up |
| `import std.iter as it` | a standard-library module under the name `it` |
| `import pane` | a dependency package, the names its `src/lib.mote` exports |
| `import pane.nav` | another file of that package, `src/nav.mote`, used as `nav.f()` |
| `import { compute } from .helper` | one name, used bare |
| `import { helper as ha } from .a` | one name under another name |

A module name cannot be used as a value, so import a function by name to pass it. A circular import is an error that names the modules.

## The standard library

Everyday modules sit directly under `std`: `string`, `math`, `collections`, `iter`, `stream`, `time`, `date`, `random`, `regex`, `task` and `test`. The rest sit in three groups.

| Group | Members |
|---|---|
| `std.data` | `json`, `base64`, `uuid`, `crypto` |
| `std.sys` | `io`, `fs`, `env`, `process`, `net`, `http_server`, `tls`, `gui` |
| `std.dev` | `log`, `args`, `libtools` |

A member is imported as `import std.data.json as json`, as the whole group (`import std.data`, used as `data.json.parse(t)`), or by name (`import { json, base64 } from std.data`).

```mote
import std.data
import { fs } from std.sys

fn main() {
    let doc = data.json.parse("{\"name\": \"demo\", \"ports\": [80, 443]}")!
    println(data.json.to_text(doc)!)
    println(fs.exists("/no/such/path"))
}
```

```output
{"name":"demo","ports":[80,443]}
false
```

## Exporting

Nothing is visible outside its module unless marked `pub`.

| Written | Exports |
|---|---|
| `pub fn`, `pub let`, `pub struct`, `pub class`, `pub enum` | the item |
| `pub import { real } from .origin` | a name from another module, as a facade |

For a `pub` type each member has its own visibility. This is `shapes.mote`:

```mote,skip
pub struct Point {
    pub x: Int
    y: Int
    pub var z: Int

    pub fn new(x: Int, y: Int) -> Point {
        return Point { x: x, y: y, z: 0 }
    }

    pub fn sum(self) -> Int {
        return self.x + self.y
    }

    fn hidden(self) -> Int {
        return self.y
    }
}
```

| Member | Another module may |
|---|---|
| a `pub` field | read it |
| a private field | not read it |
| any field | never assign it, even a `pub var`; go through a method |
| a `pub` method | call it |
| a private method | not call it |
| the literal `Point { … }` | not write it; use a `pub` static method such as `Point.new(1, 2)` |

A `pub` item cannot name a type that is not `pub`. The variants of a `pub` enum are public.

## Globals are written only at start-up

A module-level `let` or `var` can be read anywhere, but only the module's own top-level code may assign it. Assigning from a function, lambda or `spawn` block is an error.

```mote,error
var counter = 0

fn bump() -> Int {
    counter = counter + 1
    return counter
}
```

State that changes while the program runs belongs in a [`Shared`](concurrency.md#shared) cell.
