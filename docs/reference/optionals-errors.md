# Optionals and errors

## Types

<!-- block -->
| Type | Holds | Use |
|---|---|---|
| `T?` (also `Option<T>`) | a `T`, or `None` | a value that may be missing |
| `Result<T, E>` | `Ok(T)` or `Err(E)` | an operation that may fail, with the reason |

```mote
fn find(xs: List<Int>, n: Int) -> Int? {
    for i in 0..xs.len() {
        if xs[i] == n {
            return Some(i)
        }
    }
    return None
}

fn divide(a: Int, b: Int) -> Result<Int, String> {
    if b == 0 {
        return Err("zero")
    }
    return Ok(a / b)
}

fn main() {
    let a: Option<Int> = find([4, 5], 5)
    let b: Int? = a
    println(b)
    println(find([4], 9) == null)
    println(divide(6, 3))
    println(divide(1, 0))
}
```

```output
1
true
Ok(2)
Err("zero")
```

* `None` is `null`; `Some(v)` is `v`. An optional never allocates. `Option`, `Some`, `None`, `Result`, `Ok`, `Err`, `Error` and `ErrorKind` are in scope everywhere. <!-- block --> <!-- test: differential_values_the_compiled_code_shows_the_recorded_results -->

```mote
fn main() {
    let a: Int? = None
    let b: Int? = Some(3)
    let c: Result<Int, Error> = Ok(1)
    let d: Result<Int, Error> = Err(Error.other("x"))
    let e: ErrorKind = NotFound
    println(a == null)
    println(b == 3)
    println(c.is_ok())
    println(d.is_err())
    println(e == NotFound)
}
```

```output
true
true
true
true
true
```

## Reading an optional

* A field or method on `T?` is an error until the value is known to be present. <!-- block -->

```mote
class Node {
    name: String
}

fn label(n: Node?) -> String {
    return n.name // error
}
```

* A name narrows to `T` after `x != null`, `x is T`, or a guard that leaves the block. Only a `let` binding or a parameter narrows; a `var` or a field does not. <!-- block -->

```mote
class Node {
    name: String
}

fn by_guard(n: Node?) -> String {
    if n == null {
        return "none"
    }
    return n.name
}

fn by_test(n: Node?) -> String {
    if n != null {
        return n.name
    }
    return "none"
}

fn by_is(n: Node?) -> String {
    if n is Node {
        return n.name
    }
    return "none"
}

fn main() {
    let n: Node? = Node { name: "a" }
    println(by_guard(n))
    println(by_test(null))
    println(by_is(n))
    let local: Node? = Node { name: "b" }
    if local != null {
        println(local.name)
    }
}
```

```output
a
none
a
b
```

```mote
class Node {
    name: String
}

fn main() {
    var n: Node? = Node { name: "a" }
    if n != null {
        println(n.name) // error
    }
}
```

```mote
class Node {
    name: String
}

class Holder {
    var node: Node? = null
}

fn main() {
    let h = Holder()
    if h.node != null {
        println(h.node.name) // error
    }
}
```

* `if let` binds the payload. <!-- block -->

```mote
fn find(xs: List<Int>, n: Int) -> Int? {
    return xs.first()
}

fn main() {
    if let Some(v) = find([7], 3) {
        println(v)
    }
}
```

```output
7
```

## Operators

<!-- block -->
| Form | Result |
|---|---|
| `a ?? b` | the payload of `a`, else `b` |
| `x ??= v` | assigns `v` when `x` is `None` |
| `a?.f` `a?.m()` | `None` when `a` is `None`, else the member as an optional |
| `a?` | the payload, or return the `None` or `Err` from the function |
| `a!` | the payload, or fault |

```mote
class Node {
    name: String
    next: Node?

    fn shout(self) -> String {
        return self.name
    }
}

fn second(n: Node?) -> String {
    return n?.next?.name ?? "none"
}

fn head(xs: List<Int>) -> Int? {
    let x = xs.first()?
    return x + 1
}

fn main() {
    let two = Node { name: "b", next: null }
    let one = Node { name: "a", next: two }
    println(second(one))
    println(second(two))
    println(second(null))
    var x: Int? = null
    x ??= 5
    x ??= 6
    println(x)
    let n: Node? = one
    println(n?.shout())
    println(head([1]))
    println(head([]) == null)
    println(head([3])!)
}
```

```output
b
none
none
5
a
2
true
4
```

## `?` needs a matching function

* `?` on an optional needs a function returning an optional; on a `Result`, a function returning a `Result` whose error type the error fits. Convert with `.ok_or(e)?`. <!-- block -->

```mote
import { parse_int } from std.string

fn half(n: Int) -> Int? {
    if n % 2 == 0 {
        return Some(n / 2)
    }
    return None
}

fn quarter(n: Int) -> Int? {
    let h = half(n)?
    return half(h)
}

fn port(s: String?, missing: Error) -> Result<Int, Error> {
    let t = s.ok_or(missing)?
    return parse_int(t)
}

fn main() {
    println(quarter(8))
    println(quarter(6) == null)
    println(port("80", Error.other("none")).unwrap())
    println(port(null, Error.other("none")).is_err())
}
```

```output
2
true
80
true
```

```mote
fn half(n: Int) -> Int? {
    return Some(n)
}

fn bad(n: Int) -> Int {
    let h = half(n)? // error
    return h
}
```

```mote
import { parse_int } from std.string

fn bad(s: String) -> Int? {
    let n = parse_int(s)? // error
    return n
}
```

```mote
import { parse_int } from std.string

fn bad(s: String) -> Result<Int, String> {
    let n = parse_int(s)? // error
    return Ok(n)
}
```

## Methods on optionals

<!-- block -->
| Method | Result |
|---|---|
| `is_some()` `is_none()` | `Bool` |
| `unwrap()` | the payload; a fault on `None` |
| `unwrap_or(d)` | the payload, else `d` |
| `unwrap_or_else(f)` | the payload, else `f()` |
| `map(f)` | `f` applied to the payload; `U?` |
| `and_then(f)` | `f(payload)`, where `f` returns an optional |
| `ok_or(e)` | `Ok(payload)` or `Err(e)` |

```mote
fn halve(n: Int) -> Int? {
    if n % 2 == 0 {
        return n / 2
    }
    return null
}

fn main() {
    let a: Int? = 4
    let b: Int? = null
    println(a.is_some())
    println(b.is_none())
    println(a.unwrap())
    println(a.unwrap_or(9))
    println(b.unwrap_or(9))
    println(b.unwrap_or_else(|| 7))
    println(a.map(|n| n * 2))
    println(b.map(|n| n * 2) == null)
    println(a.and_then(halve))
    println(Some(3).and_then(halve) == null)
    println(a.ok_or("none"))
    println(b.ok_or("none"))
}
```

```output
true
true
4
4
9
7
8
true
2
true
Ok(4)
Err("none")
```

```mote
let b: Int? = null
let n = b.unwrap() // faults Raised
```

## Methods on `Result`

<!-- block -->
| Method | Result |
|---|---|
| `is_ok()` `is_err()` | `Bool` |
| `unwrap()` | the `Ok` payload; a fault on `Err` |
| `unwrap_or(d)` | the `Ok` payload, else `d` |
| `unwrap_or_else(f)` | the `Ok` payload, else `f()` |
| `map(f)` | `Ok(f(x))`, `Err` unchanged |
| `and_then(f)` | `f(x)` where `f` returns a `Result` |
| `or(other)` `or_else(f)` | `self` if `Ok`, else `other` or `f()` |

```mote
fn check(n: Int) -> Result<Int, String> {
    if n > 0 {
        return Ok(n)
    }
    return Err("not positive")
}

fn main() {
    let ok: Result<Int, String> = Ok(2)
    let bad: Result<Int, String> = Err("bad")
    println(ok.is_ok())
    println(bad.is_err())
    println(ok.unwrap())
    println(ok.unwrap_or(9))
    println(bad.unwrap_or(9))
    println(bad.unwrap_or_else(|| 7))
    println(ok.map(|n| n * 2))
    println(bad.map(|n| n * 2))
    println(ok.and_then(check))
    let neg: Result<Int, String> = Ok(-1)
    println(neg.and_then(check))
    println(ok.or(Ok(5)))
    println(bad.or(Ok(5)))
    println(bad.or_else(|| Ok(6)))
}
```

```output
true
true
2
2
9
7
Ok(4)
Err("bad")
Ok(2)
Err("not positive")
Ok(2)
Ok(5)
Ok(6)
```

```mote
let bad: Result<Int, String> = Err("bad")
let n = bad.unwrap() // faults Raised
```

## Matching

* `match` takes `Ok` and `Err`, `Some` and `None`. See [patterns.md](patterns.md). <!-- block -->

```mote
import { parse_int } from std.string

fn main() {
    match parse_int("12") {
        Ok(n) => println(n)
        Err(e) => println(e.message)
    }
    match parse_int("x") {
        Ok(n) => println(n)
        Err(e) => println("failed")
    }
}
```

```output
12
failed
```

## `Error`

<!-- block -->
| Part | Type | Meaning |
|---|---|---|
| `e.kind` | `ErrorKind` | what went wrong |
| `e.message` | `String` | text for a person |

```mote
fn main() {
    let e = Error.new(InvalidData, "bad bytes")
    println(typeof(e.message))
    println(e.message)
    println(e.kind == InvalidData)
    println(Error.other("x").kind == Other)
    println(e.context("reading").message)
}
```

```output
String
bad bytes
true
true
reading: bad bytes
```

* `ErrorKind` is `NotFound`, `PermissionDenied`, `InvalidData`, `Interrupted`, `AlreadyExists`, `WouldBlock`, `TimedOut`, `Unsupported` or `Other`. <!-- block -->

```mote
fn name(kind: ErrorKind) -> String {
    return match kind {
        NotFound => "not found"
        PermissionDenied => "permission denied"
        InvalidData => "invalid data"
        Interrupted => "interrupted"
        AlreadyExists => "already exists"
        WouldBlock => "would block"
        TimedOut => "timed out"
        Unsupported => "unsupported"
        Other => "other"
    }
}

fn main() {
    println(name(NotFound))
    println(name(Other))
}
```

```output
not found
other
```

* The fallible `std.string` parsers return `Result<T, Error>`. <!-- block -->

```mote
import { parse_int, parse_float } from std.string

fn main() {
    let a: Result<Int, Error> = parse_int("4")
    let b: Result<Float, Error> = parse_float("2.5")
    let c: Result<Int, Error> = parse_int("x")
    println(a)
    println(b)
    println(c.is_err())
}
```

```output
Ok(4)
Ok(2.5)
true
```

## Faults

* A fault is a run-time stop with a located message. Causes: `!` or `unwrap()` on `None` or `Err`, integer overflow, division by zero, index out of range, missing map key. <!-- block -->

```mote
let b: Int? = null
let n = b! // faults UnwrapFailed
```

```mote
let r: Result<Int, String> = Err("x")
let n = r! // faults UnwrapFailed
```

```mote
let n = 255u8 + 1 // faults IntegerOverflow
```

```mote
let z = 0
let n = 1 / z // faults DivideByZero
```

```mote
let xs = [1]
let n = xs[3] // faults IndexOutOfRange
```

```mote
let m = {"k": 1}
let n = m["x"] // faults Raised
```

* A fault cannot be caught in the faulting task; an `await` on a faulted task returns `Err`. <!-- block -->

```mote
fn main() {
    let t = spawn {
        let xs = [1]
        return xs[3]
    }
    let r = t.await()
    println(r.is_err())
}
```

```output
true
```
