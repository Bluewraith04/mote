# Structs and classes

## `struct`

* A `struct` is a value type: binding, assigning, passing, returning and storing it copies it, and `==` compares field by field. <!-- block -->

```mote
pub struct Point {
    var x: Int
    y: Int
}

fn same(p: Point) -> Point {
    return p
}

fn main() {
    var a = Point { x: 1, y: 2 }
    var b = a
    b.x = 9
    println(a.x)
    var c = same(a)
    c.x = 8
    println(a.x)
    let list = [a]
    a.x = 7
    println(list[0].x)
    println(a == Point { x: 7, y: 2 })
    println(a == b)
}
```

```output
1
1
1
true
false
```

* Fields and methods are separated by newlines, never commas, and may be mixed in any order. <!-- block -->

```mote
struct Mixed {
    fn total(self) -> Int {
        return self.a + self.b
    }
    a: Int
    b: Int
}

fn main() {
    println(Mixed { a: 1, b: 2 }.total())
}
```

```output
3
```

```mote
struct Bad {
    a: Int, b: Int // error
}
```

* A field is `pub? (let|var)? name: Type (= default)?`. A field is immutable unless declared `var`; what an immutable field holds cannot change either. <!-- block -->

```mote
struct Config {
    pub name: String
    let id: Int = 1
    var retries: Int = 3
}

class Box {
    var n: Int = 0
}

class Holder {
    box: Box
    var other: Box = Box()
}

fn main() {
    var c = Config { name: "a" }
    c.retries += 1
    println(c.retries)
    var h = Holder { box: Box() }
    h.other.n = 4
    println(h.other.n)
}
```

```output
4
4
```

```mote
struct Config {
    name: String
}

var c = Config { name: "a" }
c.name = "b" // error
```

```mote
class Box {
    var n: Int = 0
}

class Holder {
    box: Box
}

var h = Holder { box: Box() }
h.box.n = 1 // error
```

* A struct may hold only values that cannot change under it: scalars, `String`, other structs, tuples, optionals and fixed arrays of these, `Task`, `Sender`, `Receiver`, `Mutex`, `Once`, `AtomicInt`, `AtomicBool`, `PortEnd`, and immutable classes. A `List`, `Map`, `Set`, array `[T]` or class with `var` fields is an error. <!-- block -->

```mote
class Frozen {
    n: Int
}

struct Inner {
    a: Int
}

struct Holds {
    a: Int
    b: Float
    c: Bool
    d: String
    e: Inner
    f: (Int, String)
    g: Int?
    h: Task<Int>
    i: Sender<Int>
    j: Receiver<Int>
    k: Mutex<Int>
    l: Once<Int>
    m: AtomicInt
    n: AtomicBool
    o: PortEnd
    p: Frozen
    q: [Int; 2]
}

fn main() {
    println("ok")
}
```

```output
ok
```

```mote
struct Bad {
    xs: List<Int> // error
}
```

```mote
struct Bad {
    m: Map<String, Int> // error
}
```

```mote
import Set from std.collections

struct Bad {
    s: Set<Int> // error
}
```

```mote
struct Bad {
    a: [Int] // error
}
```

```mote
class Cell {
    var n: Int = 0
}

struct Bad {
    c: Cell // error
}
```

* A struct containing itself by value is an error. <!-- block -->

```mote
struct Node {
    next: Node? // error
}
```

```mote
struct A {
    b: B // error
}

struct B {
    a: A
}
```

## `class`

* A `class` is a reference type: two bindings see one object. <!-- block -->

```mote
class Counter {
    var n: Int = 0

    pub fn inc(var self) {
        self.n += 1
    }
}

fn main() {
    var a = Counter()
    let b = a
    a.inc()
    println(b.n)
}
```

```output
1
```

* A class may hold any field. A field or payload may name its own type or a later one, so lists and trees work. <!-- block -->

```mote
class Tree {
    var children: List<Tree> = []
    var parent: Tree? = null
    var data: Map<String, Later> = {}
}

class Later {
    var n: Int = 0
}

fn main() {
    var t = Tree()
    t.children.push(Tree())
    println(t.children.len())
}
```

```output
1
```

* A class with no `var` field is immutable; with Sendable field types it is Sendable. <!-- block -->

```mote
class Frozen {
    n: Int
    label: String
}

fn main() {
    let f = Frozen { n: 1, label: "a" }
    scope {
        spawn { println(f.n) }
    }
}
```

```output
1
```

```mote
class Frozen {
    n: Int
}

let f = Frozen { n: 1 }
f.n = 2 // error
```

```mote
class Open {
    var n: Int = 0
}

fn main() {
    let f = Open()
    scope {
        spawn { println(f.n) } // error
    }
}
```

* `let` is deep for classes and structs. A class changes only through a `var` path: a `var` binding or parameter, and `var` fields. <!-- block -->

```mote
class Counter {
    var n: Int = 0

    pub fn inc(var self) {
        self.n += 1
    }
}

fn main() {
    var c = Counter()
    c.inc()
    c.n += 1
    println(c.n)
}
```

```output
2
```

```mote
class Counter {
    var n: Int = 0

    pub fn inc(var self) {
        self.n += 1
    }
}

let k = Counter()
k.inc() // error
```

## Struct literal

* `Name { field: value, … }` builds a value. Every field without a default is required. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
    z: Int = 0
}

fn main() {
    let p = Point { x: 1, y: 2 }
    println(p.x + p.y + p.z)
}
```

```output
3
```

```mote
struct Point {
    x: Int
    y: Int
}

let p = Point { x: 1 } // error
```

* A bare `field` takes the variable of that name. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

fn main() {
    let x = 1
    let y = 2
    let q = Point { x, y }
    println(q.x + q.y)
}
```

```output
3
```

* `..base` copies the rest from another value. It comes first. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

fn main() {
    let p = Point { x: 1, y: 2 }
    let r = Point { ..p, x: 5 }
    println(r.x + r.y)
}
```

```output
7
```

```mote
struct Point {
    x: Int
    y: Int
}

let p = Point { x: 1, y: 2 }
let r = Point { x: 5, ..p } // error
```

* In an `if`, `while`, `for` or `match` head, `Name { x: …`, `Name { x, …` and `Name { ..` read as a literal; `Name { }` and `Name { x }` read as the body block. Parenthesize to force a literal. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

fn main() {
    let x = 1
    let y = 2
    let p = Point { x: 1, y: 2 }
    if p == Point { x: 1, y: 2 } {
        println("a")
    }
    if p == Point { x, y } {
        println("b")
    }
    if p == Point { ..p } {
        println("c")
    }
    let ok = true
    if ok {
        x
    }
    if ok { }
    if p == (Point { x: 1, y: 2 }) {
        println("d")
    }
    match Point { x: 3, y: 4 } {
        Point { x: a, y: b } => println(a + b)
    }
}
```

```output
a
b
c
d
7
```

## Construction

<!-- block -->
| Written | Does |
|---|---|
| `Point { x: 1, y: 2 }` | builds from fields |
| `Point(1, 2)` | calls the static `Point.new(1, 2)` |
| `Point()` | with no `new`: builds from every field default |
| `Box<Int>(3)` | type arguments may be written |

```mote
struct Point {
    x: Int
    y: Int

    pub fn new(x: Int, y: Int) -> Point {
        return Point { x: x, y: y }
    }
}

class Cache {
    var hits: Int = 0
    var items: List<Int> = []
}

struct Box<T> {
    value: T

    fn new(value: T) -> Box<T> {
        return Box { value: value }
    }
}

fn main() {
    let a = Point { x: 1, y: 2 }
    let b = Point(1, 2)
    let c = Cache()
    let d = Box<Int>(3)
    println(a == b)
    println(c.hits)
    println(c.items.len())
    println(d.value)
}
```

```output
true
0
0
3
```

* A type with no `new` and a field lacking a default cannot be called with `T()`. <!-- block -->

```mote
struct P {
    x: Int
}

let p = P() // error
```

* A defined `new` replaces default construction. <!-- block -->

```mote
class Cache {
    var hits: Int = 0

    pub fn new(start: Int) -> Cache {
        let c = Cache { hits: start }
        return c
    }
}

fn main() {
    println(Cache(5).hits)
}
```

```output
5
```

```mote
class Cache {
    var hits: Int = 0

    pub fn new(start: Int) -> Cache {
        return Cache { hits: start }
    }
}

let c = Cache() // error
```

* A default is evaluated for each construction, and for each instantiation of a generic type. <!-- block -->

```mote
fn tick() -> Int {
    println("tick")
    return 1
}

struct S {
    n: Int = tick()
}

struct G<T> {
    n: Int = tick()
    v: T? = null
}

class H<T> {
    var items: List<T> = []
}

fn main() {
    let a = S()
    let b = S()
    println(a.n + b.n)
    let g = G<Int>()
    let g2 = G<String>()
    println(g.n + g2.n)
    let h = H<String>()
    println(h.items.len())
}
```

```output
tick
tick
2
tick
tick
2
0
```

## Field access

* `v.name` reads a field; `v.name = e` writes a `var` field. Another module may read a `pub` field and never assigns any field. See [modules.md](modules.md). <!-- block --> <!-- test: another_module_reads_pub_fields_and_calls_pub_methods_only -->

```mote
struct Point {
    pub x: Int
    var hits: Int
}

fn main() {
    var p = Point { x: 3, hits: 0 }
    println(p.x)
    p.hits += 1
    println(p.hits)
}
```

```output
3
1
```

```mote
struct Point {
    x: Int
}

var p = Point { x: 3 }
p.x = 4 // error
```

## Methods

* A method is a `fn` in the body. See [functions.md](functions.md#methods). <!-- block -->

```mote
struct Point {
    x: Int
    y: Int

    pub fn sum(self) -> Int {
        return self.x + self.y
    }

    pub fn origin() -> Point {
        return Point { x: 0, y: 0 }
    }
}

fn main() {
    println(Point { x: 1, y: 2 }.sum())
    println(Point.origin().sum())
}
```

```output
3
0
```

* A struct method that writes needs `var self`, and the caller needs a `var` binding. <!-- block -->

```mote
struct Tally {
    var n: Int

    pub fn bump(var self) {
        self.n += 1
    }
}

fn main() {
    var t = Tally { n: 0 }
    t.bump()
    println(t.n)
}
```

```output
1
```

```mote
struct Tally {
    var n: Int

    pub fn bump(self) {
        self.n += 1 // error
    }
}
```

```mote
struct Tally {
    var n: Int

    pub fn bump(var self) {
        self.n += 1
    }
}

let t = Tally { n: 0 }
t.bump() // error
```

* A field and a method of one type cannot share a name. <!-- block -->

```mote
struct S {
    size: Int

    fn size(self) -> Int { // error
        return 1
    }
}
```

## Equality and copies

* Structs compare by field; classes by identity. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

class Box {
    var n: Int = 0
}

fn main() {
    println(Point { x: 1, y: 2 } == Point { x: 1, y: 2 })
    println(Box() == Box())
    let b = Box()
    println(b == b)
}
```

```output
true
false
true
```

## Traits on a type

* `struct T: (A, B)` lists the traits it implements. See [generics-traits.md](generics-traits.md). <!-- block -->

```mote
import { Display, Eq } from std.traits

struct Id: (Display, Eq) {
    n: Int

    fn eq(self, other: Id) -> Bool {
        return self.n == other.n
    }
}

fn main() {
    let a = Id { n: 1 }
    println(a.eq(Id { n: 1 }))
    println(a.to_string())
}
```

```output
true
Id(n: 1)
```

## Type parameters

* `struct Pair<A, B>`, `class Stack<T>`. Field defaults are allowed on generic types. <!-- block -->

```mote
struct Pair<A, B> {
    first: A
    second: B
}

class Stack<T> {
    var items: List<T> = []

    pub fn push(var self, item: T) {
        self.items.push(item)
    }
}

fn main() {
    let p = Pair { first: 1, second: "a" }
    println(p.first)
    println(p.second)
    var s = Stack<Int>()
    s.push(4)
    println(s.items.len())
}
```

```output
1
a
1
```
