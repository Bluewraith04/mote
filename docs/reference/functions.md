# Functions

## Declaration

* `fn name(params) -> Ret { body }`. The return type is optional; with none the function returns `null`. `pub` exports it. <!-- block -->

```mote
pub fn add(a: Int, b: Int) -> Int {
    return a + b
}

fn hello() {
    println("hello")
}

fn main() {
    println(add(1, 2))
    println(hello() == null)
}
```

```output
3
hello
true
```

* A function that declares a return type must `return` a value on every path. `fn main()` is the entry point. <!-- block -->

```mote
fn sign(n: Int) -> Int {
    if n > 0 {
        return 1
    }
} // error
```

```mote
fn main() {
    println("entry")
}
```

```output
entry
```

## Parameters

* Parameters are read-only: a field write, an index assignment or a writing method call on one, or on anything reached through it, is an error. Nothing is copied. <!-- block -->

```mote
struct Point {
    var x: Int
    var y: Int
}

fn total(p: Point) -> Int {
    return p.x + p.y
}

fn main() {
    println(total(Point { x: 1, y: 2 }))
}
```

```output
3
```

```mote
struct Point {
    var x: Int
}

fn bad(p: Point) {
    p.x = 1 // error
}
```

```mote
fn bad(xs: List<Int>) {
    xs[0] = 1 // error
}
```

```mote
fn bad(xs: List<Int>) {
    xs.push(1) // error
}
```

```mote
class Box {
    var n: Int = 0
}

class Holder {
    var box: Box = Box()
}

fn bad(h: Holder) {
    h.box.n = 1 // error
}
```

```mote
class Box {
    var n: Int = 0
}

fn same(a: Box, b: Box) -> Bool {
    return a == b
}

fn main() {
    let a = Box()
    println(same(a, a))
}
```

```output
true
```

* This holds for objects too: a function writes a class passed to it only through a `var` parameter. <!-- block -->

```mote
class Counter {
    var n: Int = 0

    fn inc(var self) {
        self.n += 1
    }
}

fn read(c: Counter) -> Int {
    return c.n
}

fn reset(var c: Counter) {
    c.n = 0
}

fn main() {
    var c = Counter()
    c.inc()
    println(read(c))
    reset(c)
    println(read(c))
}
```

```output
1
0
```

```mote
class Counter {
    var n: Int = 0
}

fn bad(c: Counter) {
    c.n = 0 // error
}
```

* `var name: T` is writable: the caller's variable or field changes when the call returns. The argument needs no mark. <!-- block -->

```mote
struct Holder {
    var n: Int
}

fn bump(var n: Int) {
    n += 1
}

fn main() {
    var count = 0
    bump(count)
    bump(count)
    println(count)
    var h = Holder { n: 10 }
    bump(h.n)
    println(h.n)
}
```

```output
2
11
```

* A `var` parameter takes only a writable place: a `var` binding, or a field reached through one. `bump(1 + 2)` hands the callee its own value. <!-- block -->

```mote
fn bump(var n: Int) {
    n += 1
}

fn main() {
    var count = 0
    bump(1 + 2)
    println(count)
}
```

```output
0
```

```mote
fn bump(var n: Int) {
    n += 1
}

fn main() {
    let count = 0
    bump(count) // error
}
```

* A `var` parameter cannot have a default, and a function with one cannot be used as a value except as a `(var T) -> R` type. <!-- block -->

```mote
fn bump(var n: Int = 0) { // error
    n += 1
}
```

```mote
fn bump(var n: Int) {
    n += 1
}

fn main() {
    let f = bump
    var x = 1
    f(x)
}
```

```mote
fn bump(var n: Int) {
    n += 1
}

fn main() {
    let f: (var Int) -> Null = bump
    var x = 1
    f(x)
    println(x)
}
```

```output
2
```

## Default parameters

* `name: T = expr`. Defaults are trailing: once one has a default, every later one does. A default cannot mention other parameters. <!-- block -->

```mote
fn pad(s: String, width: Int = 8, fill: String = " ") -> String {
    return "${s}|${width}|${fill}"
}

fn main() {
    println(pad("x"))
    println(pad("x", 4))
    println(pad("x", 4, "-"))
}
```

```output
x|8| 
x|4| 
x|4|-
```

```mote
fn pad(width: Int = 8, s: String) -> String { // error
    return s
}
```

```mote
fn pad(a: Int, b: Int = a) -> Int {
    return b
}

let x = pad(1) // error
```

## Named arguments

* `f(name=value)`. Positional arguments come first; named ones in any order. They skip defaults. <!-- block -->

```mote
fn pad(s: String, width: Int = 8, fill: String = " ") -> String {
    return "${s}|${width}|${fill}"
}

fn main() {
    println(pad("x", fill="-"))
    println(pad(fill="*", s="x"))
}
```

```output
x|8|-
x|8|*
```

* Allowed on calls of a function, method or constructor. Not on a function value, lambda, variadic function, variant or built-in. An unknown name, a name given twice or a missing required parameter is an error. <!-- block -->

```mote
struct Box {
    var n: Int

    fn new(n: Int) -> Box {
        return Box { n: n }
    }

    fn get(self, plus: Int = 0) -> Int {
        return self.n + plus
    }
}

fn main() {
    let b = Box(n=4)
    println(b.get(plus=1))
}
```

```output
5
```

```mote
fn f(a: Int) -> Int {
    return a
}

let g = f
let x = g(a=1) // error
```

```mote
let f = |a: Int| a
let x = f(a=1) // error
```

```mote
fn sum(...xs: Int) -> Int {
    return 0
}

let x = sum(xs=1) // error
```

```mote
enum E {
    A(Int)
}

let x = E.A(v=1) // error
```

```mote
let x = max(a=1, b=2) // error
```

```mote
fn f(a: Int) -> Int {
    return a
}

let x = f(b=1) // error
```

```mote
fn f(a: Int) -> Int {
    return a
}

let x = f(a=1, a=2) // error
```

```mote
fn f(a: Int, b: Int) -> Int {
    return a
}

let x = f(a=1) // error
```

## Variadic parameters

* `...name: T` is the last parameter and collects the trailing arguments into a `List<T>`. It excludes defaults. <!-- block -->

```mote
fn sum(...xs: Int) -> Int {
    var t = 0
    for x in xs {
        t += x
    }
    return t
}

fn main() {
    println(sum(1, 2, 3))
    println(sum())
}
```

```output
6
0
```

```mote
fn bad(...xs: Int, n: Int) -> Int { // error
    return n
}
```

```mote
fn bad(...xs: Int = 1) -> Int { // error
    return 0
}
```

* Only a direct call by name packs: through a stored function value, pass the list. <!-- block -->

```mote
fn sum(...xs: Int) -> Int {
    return xs.len()
}

fn main() {
    let f = sum
    println(f([1, 2]))
}
```

```output
2
```

```mote
fn sum(...xs: Int) -> Int {
    return xs.len()
}

let f = sum
let n = f(1, 2) // error
```

## Lambdas

* `|params| expr` and `|params| { block }`. A parameter type comes from the expected type or is written; with neither it is an error. The result type is inferred or written. <!-- block -->

```mote
fn main() {
    let inc = |n: Int| n + 1
    let twice: (Int) -> Int = |n| n * 2
    let add = |a: Int, b: Int| -> Int { return a + b }
    let hi = || println("hi")
    println(inc(1))
    println(twice(4))
    println(add(1, 2))
    hi()
}
```

```output
2
8
3
hi
```

```mote
let f = |n| n + 1 // error
```

* A lambda parameter is read-only unless written `|var s|` or the expected type marks its slot `var`. A function with a read-only parameter fits a `var` slot; not the reverse. <!-- block -->

```mote
fn read(n: Int) {
    println(n)
}

fn bump(var n: Int) {
    n += 1
}

fn main() {
    let f = |var n: Int| { n += 1 }
    var x = 1
    f(x)
    println(x)
    let g: (var Int) -> Null = |n| { n += 1 }
    g(x)
    println(x)
    let h: (var Int) -> Null = read
    h(x)
}
```

```output
2
3
3
```

```mote
let f = |n: Int| { n += 1 } // error
```

```mote
fn bump(var n: Int) {
    n += 1
}

let f: (Int) -> Null = bump // error
```

* A union in a lambda parameter or result type needs parentheses. <!-- block -->

```mote
fn main() {
    let f = |x: (Int | String)| x.to_string()
    println(f(1))
    println(f("a"))
}
```

```output
1
a
```

```mote
let f = |x: Int | String| x // error
```

* A lambda captures a `var` by reference: the lambda and the enclosing function share one variable. A `let` or a parameter is captured as a value: a struct is copied, a class is shared. <!-- block -->

```mote
struct P {
    var x: Int
}

class Box {
    var n: Int = 0
}

fn capture_struct(p: P) -> () -> Int {
    return || p.x
}

fn capture_class(c: Box) -> () -> Int {
    return || c.n
}

fn main() {
    var n = 0
    let inc = || n += 1
    inc()
    inc()
    println(n)
    var p = P { x: 1 }
    let sp = capture_struct(p)
    p.x = 9
    println(sp())
    var c = Box()
    let sc = capture_class(c)
    c.n = 9
    println(sc())
}
```

```output
2
1
9
```

* A lambda that captures a `var`, or a value that is not Sendable, is not Sendable. A named function is Sendable. <!-- block -->

```mote
fn work() {}

fn main() {
    scope {
        spawn { work() }
    }
    println("ok")
}
```

```output
ok
```

```mote
fn main() {
    var n = 0
    let f = || n += 1
    scope {
        spawn { f() } // error
    }
}
```

```mote
fn main() {
    let xs = [1]
    let f = || xs.len()
    scope {
        spawn { f() } // error
    }
}
```

* A block lambda yields its value with `return`. Its result type is the written or expected one, else the type of its first `return`, else `Null`. <!-- block -->

```mote
fn main() {
    let a = |n: Int| { return n + 1 }
    let b = |n: Int| { println(n) }
    let c = |n: Int| -> Float { return 2.0 }
    println(typeof(a(1)))
    println(b(1) == null)
    println(typeof(c(1)))
    let d: (Int) -> Float = |n| { return 1.0 }
    println(typeof(d(1)))
}
```

```output
Int
1
true
Float
Float
```

## Higher-order

* A function value has the type `(A, B) -> R`. <!-- block -->

```mote
fn apply(f: (Int) -> Int, x: Int) -> Int {
    return f(x)
}

fn combine(f: (Int, Int) -> Int) -> Int {
    return f(2, 3)
}

fn main() {
    println(apply(|n| n * 2, 4))
    println(combine(|a, b| a * b))
}
```

```output
8
6
```

## Nested functions

* A `fn` statement inside a body is local and sees names declared before it and itself, so it may call itself. It takes no type parameters, defaults or `...` parameter, and its parameters are typed. <!-- block -->

```mote
fn outer() -> Int {
    fn inner(n: Int) -> Int {
        return n + 1
    }
    fn count(n: Int) -> Int {
        if n == 0 {
            return 0
        }
        return 1 + count(n - 1)
    }
    return inner(count(3))
}

fn main() {
    println(outer())
}
```

```output
4
```

```mote
fn outer() {
    fn inner<T>(n: T) {} // error
}
```

```mote
fn outer() {
    fn inner(n: Int = 1) {} // error
}
```

```mote
fn outer() {
    fn inner(...n: Int) {} // error
}
```

```mote
fn outer() {
    fn inner(n) {} // error
}
```

## Methods

* A method is a `fn` inside a `struct`, `class` or `enum` body. Its first parameter selects the kind. <!-- block -->

<!-- block -->
| Receiver | Meaning |
|---|---|
| `self` | reads the object; writing it is an error |
| `var self` | may write the caller's object in place |
| none | static: called `Type.method(…)` |

```mote
class Counter {
    var n: Int = 0

    pub fn inc(var self) {
        self.n += 1
    }

    pub fn get(self) -> Int {
        return self.n
    }

    pub fn start(n: Int) -> Counter {
        var c = Counter()
        c.n = n
        return c
    }
}

enum Light {
    Red
    Green

    fn flip(self) -> Light {
        return match self {
            Red => Green
            Green => Red
        }
    }
}

fn main() {
    var c = Counter.start(5)
    c.inc()
    println(c.get())
    println(Light.Red.flip() == Light.Green)
}
```

```output
6
true
```

```mote
class Counter {
    var n: Int = 0

    fn bad(self) {
        self.n = 1 // error
    }
}
```

* A method may have its own type parameters and uses its type's. A field and a method cannot share a name. `from` may name a method. <!-- block -->

```mote
struct Box<T> {
    value: T

    fn map<U>(self, f: (T) -> U) -> Box<U> {
        return Box { value: f(self.value) }
    }

    fn from(value: T) -> Box<T> {
        return Box { value: value }
    }
}

fn main() {
    let b = Box.from(4)
    println(b.map(|n| n * 2).value)
}
```

```output
8
```

```mote
struct S {
    size: Int

    fn size(self) -> Int { // error
        return 1
    }
}
```

## Generators

* A function declared `-> Stream<T>` that holds `yield e` is a generator. Calling it returns a lazy `Stream<T>` that runs as `for`, `.next()` or an adapter consumes it. <!-- block -->

```mote
import { Stream } from std.collections

fn count(n: Int) -> Stream<Int> {
    for i in 0..n {
        println("make ${i}")
        yield i
    }
}

fn main() {
    let s = count(2)
    println("made")
    for v in s {
        println(v)
    }
}
```

```output
made
make 0
0
make 1
1
```

* A generator's `return` takes no value and ends the stream. `yield` outside a generator, in a lambda or in a nested function is an error. <!-- block -->

```mote
import { Stream } from std.collections

fn upto(n: Int) -> Stream<Int> {
    for i in 0..10 {
        if i == n {
            return
        }
        yield i
    }
}

fn main() {
    for v in upto(2) {
        println(v)
    }
}
```

```output
0
1
```

```mote
import { Stream } from std.collections

fn bad() -> Stream<Int> {
    yield 1
    return 2 // error
}
```

```mote
fn bad() {
    yield 1 // error
}
```

```mote
import { Stream } from std.collections

fn bad() -> Stream<Int> {
    let f = || { yield 1 } // error
    yield 2
}
```

```mote
import { Stream } from std.collections

fn bad() -> Stream<Int> {
    fn inner() {
        yield 1 // error
    }
    yield 2
}
```

* A generator cannot `yield` inside a `with` or a `scope`: a reader that stops early would leave the resource open. Close the resource before the `yield`, or open it again after. <!-- block -->

```mote
import { Stream } from std.collections

class Res {
    fn close(self) {}
}

fn bad() -> Stream<Int> {
    with r = Res() {
        yield 1 // error
    }
}
```

```mote
import { Stream } from std.collections

fn bad() -> Stream<Int> {
    scope {
        yield 1 // error
    }
}
```

## Calls and construction

* `f(args)` calls. `T(args)` calls the static `T.new(args)`. `T()` with no `new` builds from every field default. See [structs-classes.md](structs-classes.md). <!-- block -->

```mote
struct Point {
    x: Int
    y: Int

    fn new(x: Int, y: Int) -> Point {
        return Point { x: x, y: y }
    }
}

class Counter {
    var n: Int = 7
}

fn main() {
    let p = Point(1, 2)
    println(p.x + p.y)
    println(Counter().n)
}
```

```output
3
7
```

* Type arguments may be written: `first<Int>(xs)`, `Box<Int>(3)`. <!-- block -->

```mote
struct Box<T> {
    value: T

    fn new(value: T) -> Box<T> {
        return Box { value: value }
    }
}

fn first<T>(xs: List<T>) -> T? {
    return xs.first()
}

fn main() {
    println(first<Int>([4, 5]))
    println(Box<Int>(3).value)
}
```

```output
4
3
```

## Native functions

* `native fn name(params) -> Ret` declares a function implemented outside Mote; it has no body. See [modules.md](modules.md). <!-- block -->

```mote
@link("m")
native fn sqrt(x: Float) -> Float

fn main() {
    println("declared")
}
```

```output
declared
```

```mote
native fn sqrt(x: Float) -> Float { // error
    return x
}
```
