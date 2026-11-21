# Patterns

## Where patterns appear

<!-- block -->
| Site | Must always match |
|---|---|
| `match` arm | no |
| `if let` / `while let` | no |
| `let` / `var` | yes |
| `for` | yes |

```mote
fn main() {
    let o: Int? = 4
    match o {
        Some(1) => println("one")
        Some(n) => println(n)
        None => println("none")
    }
    if let Some(1) = o {
        println("one")
    }
    var q = [1]
    while let Some(2) = q.pop() {
        println("two")
    }
    let (a, b) = (1, 2)
    var (c, d) = (3, 4)
    c += 1
    for (e, f) in [(5, 6)] {
        println(a + b + c + d + e + f)
    }
}
```

```output
4
22
```

```mote
let o: Int? = 4
let Some(x) = o // error
```

```mote
var o: Int? = 4
var Some(x) = o // error
```

```mote
let xs: List<Int?> = [1]
for Some(x) in xs { // error
    println(x)
}
```

* A `let`, `var` or `for` pattern is a binding pattern: no top-level `name: Type`, no or-pattern, no literal. <!-- block -->

```mote
fn main() {
    let 1 = 1 // error
}
```

```mote
fn main() {
    let a | b = 1 // error
}
```

```mote
for n: Int in [1] { // error
    println(n)
}
```

## Wildcard

* `_` matches anything and binds nothing. <!-- block -->

```mote
fn main() {
    match 3 {
        _ => println("any")
    }
    let (_, b) = (1, 2)
    println(b)
}
```

```output
any
2
```

```mote
let (_, b) = (1, 2)
println(_) // error
```

## Binding

* A bare name matches anything and binds it. A bare name that is a unit variant of the subject's enum matches that variant instead. <!-- block -->

```mote
enum Level {
    Low
    High
}

fn main() {
    match 7 {
        0 => println("zero")
        other => println(other)
    }
    let level = Level.High
    match level {
        Low => println("low")
        High => println("high")
    }
}
```

```output
7
high
```

* On an enum subject, a bare name that starts with an uppercase letter and is not a variant is an error, so a misspelt variant does not bind. <!-- block -->

```mote
enum Level {
    Low
    High
}

let level = Level.Low
match level {
    Low => println("low")
    Hgih => println("high") // error
}
```

* Names bound by a pattern are read-only. <!-- block -->

```mote
fn main() {
    match 7 {
        n => {
            n = 8 // error
        }
    }
}
```

## Literal

* An `Int` (negative allowed), `String`, `Bool` or `null`. `Float` literals are not patterns. <!-- block -->

```mote
fn name(n: Int) -> String {
    match n {
        -1 => return "minus one"
        0 => return "zero"
        _ => return "other"
    }
}

fn main() {
    let s = "b"
    match s {
        "a" => println(1)
        "b" => println(2)
        _ => println(0)
    }
    match true {
        true => println("t")
        false => println("f")
    }
    let o: Int? = null
    match o {
        null => println("null")
        _ => println("some")
    }
    println(name(-1))
    println(name(0))
    println(name(5))
}
```

```output
2
t
null
minus one
zero
other
```

```mote
match 1.5 {
    1.5 => println("x") // error
    _ => println("y")
}
```

## Range

* `a..b` and `a..=b` over integer literals. <!-- block -->

```mote
fn size(n: Int) {
    match n {
        0..10 => println("digit")
        10..=99 => println("two digits")
        _ => println("more")
    }
}

fn main() {
    size(9)
    size(10)
    size(99)
    size(100)
}
```

```output
digit
two digits
two digits
more
```

```mote
match 1.5 {
    0.0..1.0 => println("x") // error
    _ => println("y")
}
```

## `@` binding

* `name @ pattern` matches the pattern and binds the whole value. <!-- block -->

```mote
fn main() {
    match 4 {
        d @ 1..=9 => println(d)
        _ => {}
    }
    let o: Int? = 3
    match o {
        whole @ Some(n) => println(n)
        None => println("none")
    }
}
```

```output
4
3
```

## Or

* `p | q` matches either. Each alternative binds the same names with the same types. <!-- block -->

```mote
fn main() {
    match 5 {
        4 | 5 => println("mid")
        _ => {}
    }
    match (1, 2) {
        (x, 0) | (0, x) => println(x)
        (x, _) => println(x + 10)
    }
}
```

```output
mid
11
```

```mote
match (1, 2) {
    (x, 0) | (0, y) => println("x") // error
    _ => {}
}
```

```mote
match (1, "a") {
    (x, 0) | (0, x) => println("x") // error
    _ => {}
}
```

## Tuple

* `(p, q)` matches a tuple of the same arity, element by element. `(p)` is `p`. <!-- block -->

```mote
fn main() {
    match (0, 8) {
        (0, y) => println(y)
        (x, y) => println(x + y)
    }
    match (3, 4) {
        (0, y) => println(y)
        (x, y) => println(x + y)
    }
    match 5 {
        (n) => println(n)
    }
}
```

```output
8
7
5
```

```mote
match (1, 2) {
    (a, b, c) => println(a) // error
}
```

## Enum variant

* `Variant`, `Variant(p, …)`, `Variant { field, … }`. A variant may be qualified: `Shape.Circle(r)`. <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Rect { w: Int, h: Int }
    Unit
}

fn show(shape: Shape) {
    match shape {
        Circle(r) => println(r)
        Rect { w, h } => println(w * h)
        Unit => println("unit")
    }
}

fn qualified(shape: Shape) {
    match shape {
        Shape.Circle(r) => println(r + 1)
        Shape.Rect { w, h: height } => println(w + height)
        Shape.Unit => println("none")
    }
}

fn main() {
    show(Shape.Circle(3))
    show(Shape.Rect { w: 2, h: 5 })
    show(Shape.Unit)
    qualified(Shape.Circle(3))
    qualified(Shape.Rect { w: 2, h: 5 })
    qualified(Shape.Unit)
}
```

```output
3
10
unit
4
7
none
```

* A payload has its instantiated type: in `Result<Int, String>`, `Ok(n)` binds `n: Int`; in `Int?`, `Some(n)` binds `n: Int`. <!-- block -->

```mote
fn main() {
    let r: Result<Int, String> = Ok(1)
    match r {
        Ok(n) => println(typeof(n))
        Err(e) => println(typeof(e))
    }
    let o: Int? = 2
    match o {
        Some(n) => println(typeof(n))
        None => println("none")
    }
}
```

```output
Int
Int
```

## Struct

* `Name { field, field: p, .. }` matches a struct or class. A field alone binds a name; `field: p` matches `p`; `..` ignores the rest. Without `..` every field is named. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

class Box {
    n: Int
    label: String
}

fn main() {
    let p = Point { x: 0, y: 7 }
    let Point { x, y } = p
    println(x + y)
    if let Point { x: 0, y } = p {
        println(y)
    }
    let Point { x: px, .. } = p
    println(px)
    let b = Box { n: 3, label: "b" }
    let Box { n, .. } = b
    println(n)
}
```

```output
7
7
0
3
```

```mote
struct Point {
    x: Int
    y: Int
}

let p = Point { x: 0, y: 7 }
let Point { x } = p // error
```

## Type

* `name: Type` and `_: Type` match a value of that type and bind it as that type. A union target needs parentheses. <!-- block -->

```mote
fn show(v: Int | String | Bool | Null) {
    match v {
        n: Int => println(n + 1)
        s: String => println(s)
        _: (Bool | Null) => println("flag")
    }
}

fn main() {
    show(1)
    show("a")
    show(true)
    show(null)
}
```

```output
2
a
flag
flag
```

* A type pattern no value of the subject can match is an error. A subject can be that type, a union holding it, or an optional of it. <!-- block -->

```mote
let v: Int | String = 1
match v {
    f: Float => println(f) // error
    _ => {}
}
```

```mote
fn main() {
    let o: Int? = 1
    match o {
        n: Int => println(n)
        _ => println("none")
    }
}
```

## Nesting

* Patterns nest to any depth. <!-- block -->

```mote
fn show(r: Result<(Int, Int)?, String>) {
    match r {
        Ok(Some((a, b))) => println(a + b)
        Ok(None) => println("none")
        Err(e) => println(e)
    }
}

fn main() {
    show(Ok((1, 2)))
    show(Ok(null))
    show(Err("bad"))
}
```

```output
3
none
bad
```

## Guards

* `pattern if cond =>` matches only when `cond` holds. A guarded arm does not count toward exhaustiveness. <!-- block -->

```mote
fn size(n: Int) {
    match n {
        x if x > 5 => println("big")
        _ => println("small")
    }
}

fn main() {
    size(9)
    size(1)
}
```

```output
big
small
```

```mote
let o: Int? = 1
match o { // error
    Some(x) if x > 0 => println(x)
    None => println("none")
}
```

## Exhaustiveness

* A `match` must cover its subject. An enum needs every variant; a union every member; `Bool` both values; an optional `Some` and `None`; anything else a catch-all. See [control-flow.md](control-flow.md). <!-- block -->

```mote
enum Level {
    Low
    High
}

let level = Level.Low
match level { // error
    Low => println("low")
}
```

```mote
match 3 { // error
    1 => println("one")
}
```

* A tuple or struct pattern made only of bindings and wildcards counts as a catch-all. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

fn main() {
    match (1, 2) {
        (a, _) => println(a)
    }
    match Point { x: 3, y: 4 } {
        Point { x, y } => println(x + y)
    }
}
```

```output
1
7
```

## Refutable bindings

* A `let` pattern that can fail is an error. Use `if let` or `match`. <!-- block -->

```mote
let o: Int? = 1
let Some(x) = o // error
```

* A top-level `let` that binds no name is top-level code: it runs at start-up and is checked the same way. <!-- block -->

```mote
fn f() -> Int {
    println("ran")
    return 1
}

let _ = f()

fn main() {
    println("main")
}
```

```output
ran
main
```

```mote
let 1 = 2 // error
```
