# Bindings

## `let` and `var`

* `let` is immutable, `var` mutable. An initializer is required. <!-- block -->

```mote
fn main() {
    let a = 1
    var b = 2
    b = 3
    println(a + b)
}
```

```output
4
```

```mote
let a: Int // error
```

```mote
let a = 1
a = 2 // error
```

* A type may be written. <!-- block -->

```mote
let c: Int = 4
```

* Assigning to a `let` name is an error. `let` is deep: nothing reached through it can be written, for structs and classes alike. <!-- block -->

```mote
struct Point {
    var x: Int
}

let p = Point { x: 1 }
p.x = 5 // error
```

```mote
class Counter {
    var n: Int = 0

    fn inc(var self) {
        self.n += 1
    }
}

let c = Counter()
c.inc() // error
```

```mote
class Counter {
    var n: Int = 0

    fn inc(var self) {
        self.n += 1
    }
}

fn main() {
    var d = Counter()
    d.inc()
    println(d.n)
}
```

```output
1
```

* A write needs a `var` path: a `var` binding, `var` parameter or `var self`, and a `var` field at every field step. Writing means assigning a field or index, calling a `var self` method, or passing the place to a `var` parameter. <!-- block -->

```mote
struct Inner {
    var n: Int
}

struct Outer {
    let inner: Inner
}

fn set(var i: Inner) {
    i.n = 1
}

fn main() {
    var o = Outer { inner: Inner { n: 0 } }
    set(o.inner) // error: this place has no `var` path to write through
}
```

```mote
struct Inner {
    var n: Int
}

fn set(var i: Inner) {
    i.n = 1
}

fn main() {
    var a = Inner { n: 0 }
    set(a)
    a.n += 1
    println(a.n)
}
```

```output
2
```

* Another reference to the same object can still write it; the rule restricts the path, not the object. <!-- block -->

```mote
class Box {
    var n: Int = 0
}

fn main() {
    var a = Box()
    let b = a
    a.n = 5
    println(b.n)
}
```

```output
5
```

## Scope and shadowing

* A name is visible from its binding to the end of its block. <!-- block -->

```mote
fn main() {
    if true {
        let inner = 1
    }
    println(inner) // error
}
```

* A name cannot be bound twice in one scope; a nested block, lambda or `fn` may bind it again. <!-- block -->

```mote
let x = 1
let x = 2 // error
```

```mote
fn main() {
    let x = 1
    if true {
        let x = "inner"
        println(x)
    }
    let f = |x: Int| x + 1
    println(f(x))
}
```

```output
inner
2
```

## Destructuring

* The binding target is a pattern: tuples, structs, variants, nesting. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

fn main() {
    let pair = (1, 2)
    let nested = (3, (4, 5))
    let p = Point { x: 6, y: 7 }
    let (a, b) = pair
    let (n, (x, y)) = nested
    let Point { x: px, y: py } = p
    let Point { x: qx, .. } = p
    println(a + b)
    println(n + x + y)
    println(px + py)
    println(qx)
}
```

```output
3
12
13
6
```

* A binding pattern must always match. `let Some(x) = o` is an error; use `if let`. <!-- block -->

```mote
fn main() {
    let o: Int? = 4
    let Some(x) = o // error
}
```

```mote
fn main() {
    let o: Int? = 4
    if let Some(x) = o {
        println(x)
    }
}
```

```output
4
```

* Names bound by a pattern are read-only. <!-- block -->

```mote
fn main() {
    let (a, b) = (1, 2)
    a = 3 // error
}
```

## Assignment

* Assignment is a statement, not an expression: `a = b = c` does not parse. <!-- block -->

```mote
fn main() {
    var a = 1
    var b = 2
    var c = 3
    a = b = c // error
}
```

* Targets: a `var` name, a `var` field, an index. <!-- block -->

```mote
struct Point {
    var x: Int
}

fn main() {
    var x = 0
    var p = Point { x: 0 }
    var xs = [0, 0]
    x = 1
    p.x = 2
    xs[0] = 3
    println(x)
    println(p.x)
    println(xs)
}
```

```output
1
2
[3, 0]
```

* Compound forms: `+= -= *= /= %= &= |= ^= <<= >>= ??=`. The target is evaluated once. <!-- block -->

```mote
fn index() -> Int {
    println("index")
    return 0
}

fn main() {
    var xs = [1, 2]
    xs[index()] += 1
    var n = 7
    n -= 1
    n *= 3
    n /= 2
    n %= 5
    n <<= 2
    n >>= 1
    n &= 7
    n |= 8
    n ^= 1
    println(xs)
    println(n)
}
```

```output
index
[2, 2]
9
```

* `x ??= v` assigns only when `x` is `None`. <!-- block -->

```mote
fn load() -> Int {
    println("load")
    return 5
}

fn main() {
    var a: Int? = null
    var b: Int? = 1
    a ??= load()
    b ??= load()
    println(a)
    println(b)
}
```

```output
load
5
1
```

## Globals

* A top-level `let` or `var` is a module global. Only the module's own top-level code may assign it; a function, lambda or `spawn` cannot. <!-- block -->

```mote
var counter = 0
counter += 1

fn main() {
    println(counter)
}
```

```output
1
```

```mote
var counter = 0

fn bump() {
    counter += 1 // error
}
```

```mote
var counter = 0

fn main() {
    let f = || counter = 2 // error
}
```

* `pub let` exports a global for reading. <!-- block -->

```mote
pub let MAX = 100

fn main() {
    println(MAX)
}
```

```output
100
```

## Start-up order

<!-- block -->
| Step | Order |
|---|---|
| Modules | each after the modules it imports, the prelude first |
| Globals of a module | each after every global it reads, directly or through the functions it calls; otherwise in source order |
| Top-level statements | after the module's globals |
| `let (a, b) = e` | one step, ordered by the globals `e` reads |
| A global that reads itself | an error naming the cycle |

```mote
fn two() -> Int {
    println("two")
    return b + 1
}

let a = two()
let b = 1
let (c, d) = (a, b)

println("top")

fn main() {
    println(c)
    println(d)
}
```

```output
two
top
2
1
```

```mote
let a = b + 1
let b = a + 1 // error
```

* A `spawn` that start-up runs, in a global's initializer, top-level code or a function they call, cannot read a `var` global, directly or through the functions it calls: top-level code may still assign it. Once start-up ends, globals no longer change. <!-- block -->

```mote
var counter = 0

fn read() -> Int {
    return counter
}

fn start() {
    let t = spawn read() // error: a task started at start-up cannot read the `var` global `counter`
}

start()
```

```mote
var counter = 0

fn read() -> Int {
    return counter
}

fn main() {
    let t = spawn read()
    println(t.await())
}
```

```output
Ok(0)
```

* State that changes while the program runs belongs in a `Mutex` or an atomic. <!-- block -->

```mote
fn main() {
    let m = Mutex(0)
    m.lock(|n| { n += 1 })
    println(m.lock(|n| { return n }))
}
```

```output
1
```

## Expression statements

* Any expression may stand alone; its value is dropped. <!-- block -->

```mote
fn main() {
    var list: List<Int> = []
    list.push(1)
    list.len()
    1 + 2
    println(list)
}
```

```output
[1]
```
