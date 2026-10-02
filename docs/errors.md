# Absence and failure

Both types are in scope everywhere without an import.

| Type | Holds | Use for |
|---|---|---|
| `T?` | a `T`, or `None` | a value that may be missing |
| `Result<T, E>` | `Ok(T)` or `Err(E)` | an operation that may fail, with the reason |

`Some(v)` wraps a value and `None` is the absence. At run time `None` is `null` and `Some(v)` is `v`, so an optional never allocates. `Option<T>` is another spelling of `T?`; the examples use `T?`.

```mote
fn first_positive(a: Int, b: Int) -> Int? {
    if a > 0 { return Some(a) }
    if b > 0 { return Some(b) }
    return None
}

fn pick<T>(xs: List<T>, i: Int) -> T? {
    if i < xs.len() { return Some(xs.get(i)) }
    return None
}

fn main() {
    match first_positive(0, 7) {
        Some(v) => { println(v) }
        None => { println("none") }
    }
    println(pick(["a", "b"], 1)!)
    println(pick(["a", "b"], 5) == None)
}
```

```output
7
b
true
```

## Reading an optional

A field or method on a `T?` is an error until the value is known to be there. The compiler narrows a name after `x != null`, `x is T`, or a guard such as `if x == null { return }`. Only a `let` or a parameter narrows; a `var` or a field can change between check and use.

```mote
class Node {
    name: String
    next: Node?
}

fn label(n: Node?) -> String {
    if n == null { return "none" }
    return n.name
}

fn second(n: Node?) -> String {
    return n?.next?.name ?? "none"
}

println(label(Node { name: "a", next: None }))
println(label(None))
let tail = Node { name: "b", next: None }
println(second(Node { name: "a", next: tail }))
println(second(Node { name: "a", next: None }))
```

```output
a
none
b
none
```

| Operator | Result |
|---|---|
| `a ?? b` | the payload of `a`, else `b` |
| `x ??= v` | assigns `v` only when `x` is `None` |
| `a?.f`, `a?.f()` | `None` when `a` is `None`, else the member as an optional |

## `?` and `!`

Two postfix operators take the payload out of an optional or a `Result`. They differ in what a failure does.

| Operator | On `None` or `Err` |
|---|---|
| `x?` | returns it from the enclosing function, so the caller sees the same failure |
| `x!` | faults, stopping the task with a message |

`x!` is the short form of `x.unwrap()`; the two are the same. Examples use `!` where a failure would be a bug and `?` where the function can pass it on.

```mote
import std.string as str

fn half(n: Int) -> Int? {
    if n % 2 == 0 { return Some(n / 2) }
    return None
}

fn quarter(n: Int) -> Int? {
    let h = half(n)?
    return half(h)
}

fn double(s: String) -> Result<Int, Error> {
    let n = str.parse_int(s)?
    return Ok(n * 2)
}

println(quarter(8))
println(quarter(6))
println(half(84)!)
println(double("21")!)
println(double("x").is_err())
```

```output
2
null
42
42
true
```

`?` needs a function that returns the same kind: an optional (`T?`) for an optional, a `Result` for a `Result`. In a `Result` function, turn an optional into one with `o.ok_or(e)?`. A function that declares no result type, including `main` and top-level code, also accepts `?`: on a failure it simply ends there, and the failure is dropped.

## Methods

| Method | On | Result |
|---|---|---|
| `is_some()`, `is_none()` | optional | `Bool` |
| `is_ok()`, `is_err()` | `Result` | `Bool` |
| `unwrap()` | both | the payload; a fault on `None` or `Err`; the same as `x!` |
| `unwrap_or(d)`, `unwrap_or_else(f)` | both | the payload, else `d` or `f()` |
| `map(f)` | both | the same shape with `f` applied |
| `and_then(f)` | both | `f(payload)`, itself an optional or `Result` |
| `ok_or(e)` | optional | `Ok(payload)` or `Err(e)` |
| `or(other)`, `or_else(f)` | `Result` | the `Result`, else `other` or `f()` |

## `Error`

Every fallible standard-library operation returns a `Result` whose `Err` holds an `Error`, with a `kind` and a `message`. `ErrorKind` is `NotFound`, `PermissionDenied`, `InvalidData`, `Interrupted`, `AlreadyExists`, `WouldBlock`, `TimedOut`, `Unsupported` or `Other`.

```mote
import { read_file } from std.sys.io

match read_file("missing.txt") {
    Ok(text) => println(text)
    Err(e) => {
        match e.kind {
            NotFound => println("no such file")
            _ => println("other error")
        }
    }
}
```

```output
no such file
```

See [std.error](std/error.md).
