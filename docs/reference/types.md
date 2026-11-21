# Types

## Primitive types

* `Int` is a signed 64-bit integer, `Float` a 64-bit IEEE float, `Bool` is `true`/`false`, `String` is immutable UTF-8, `Null` is `null`. <!-- block -->

```mote
fn main() {
    let a: Int = 9223372036854775807
    let b: Int = -a
    let c: Float = 0.1 + 0.2
    let d: Bool = true
    let e: String = "héllo"
    let f: Null = null
    println(a)
    println(b)
    println(c)
    println(d)
    println(e.len())
    println(f == null)
}
```

```output
9223372036854775807
-9223372036854775807
0.30000000000000004
true
6
true
```

* A `String` cannot be written through an index. <!-- block -->

```mote
var s = "ab"
s[0] = "x" // error
```

* Sized numbers: `Int8 Int16 Int32 UInt8 UInt16 UInt32 UInt64 Float32`. `Int64` and `Float64` are other names for `Int` and `Float`. See [numbers.md](numbers.md). <!-- block -->

```mote
fn main() {
    let a: Int8 = -128
    let b: Int16 = 1
    let c: Int32 = 1
    let d: UInt8 = 255
    let e: UInt16 = 1
    let f: UInt32 = 1
    let g: UInt64 = 1
    let h: Float32 = 1.5
    let i: Int64 = 5
    let j: Float64 = 1.5
    let k: Int = i
    let l: Float = j
    println(a)
    println(d)
    println(h)
    println(k)
    println(l)
}
```

```output
-128
255
1.5
5
1.5
```

* There are no implicit conversions between number types, including `Int` and `Float`. <!-- block -->

```mote
let x: Float = 3 // error: expected Float, found Int
```

```mote
let u: UInt8 = 1
let n: Int = u // error: expected Int, found UInt8
```

```mote
let f = Float.from(3)
let g = 3 as Float
```

## Type arguments are never guessed

* A binding, parameter, return type or field fixes an otherwise open type argument. With nothing to fix it, it is an error. <!-- block -->

```mote
let xs: List<Int> = []
let m = Map<String, Int>()
```

```mote
let bad = Map() // error: cannot infer the type argument `K`
```

* A bare generic name in a type is an error: write its arguments. <!-- block -->

```mote
fn f(xs: List) {} // error: write the type arguments of `List`
```

## Lists and arrays

* `List<T>` is a growable sequence. `[T]` is an array whose length is fixed when made. `[T; N]` is an array with a compile-time length. <!-- block -->

```mote
fn main() {
    var xs: List<Int> = [1, 2, 3]
    xs.push(4)
    let a: [Int] = [1, 2, 3]
    let b: [UInt8; 4] = [0; 4]
    println(xs.len())
    println(a.len())
    println(b[3])
}
```

```output
4
3
0
```

```mote
let a: [Int] = [1, 2, 3]
a.push(4) // error
```

* `[1, 2, 3]` is a `List` unless an annotation says array. <!-- block -->

```mote
fn main() {
    let xs = [1, 2, 3]
    let a: [Int] = [1, 2, 3]
    println(typeof(xs))
    println(typeof(a))
}
```

```output
List<Int>
[Int]
```

* `[T; N]` is an inline value that copies; `N` is an integer literal. `[T]` is an object that aliases. <!-- block -->

```mote
fn main() {
    var a: [Int; 2] = [1, 2]
    var b = a
    b[0] = 9
    println(a)
    var c: [Int] = [1, 2]
    var d = c
    d[0] = 9
    println(c)
}
```

```output
[1, 2]
[9, 2]
```

```mote
let n = 2
let a: [Int; n] = [0; 2] // error
```

* `[v; n]` repeats `v`: it is a `[T]` unless annotated `[T; N]`, and then `n` is the literal `N`. `v` is evaluated once and copied, so it must be a value type. <!-- block -->

```mote
fn main() {
    let n = 3
    let zeros: [Int] = [0; n]
    let fixed: [Int; 3] = [0; 3]
    println(zeros)
    println(fixed)
}
```

```output
[0, 0, 0]
[0, 0, 0]
```

```mote
let n = 3
let fixed: [Int; 3] = [0; n] // error
```

```mote
let lists = [List<Int>(); 3] // error
```

## Built-in generic types

* `List<T> Map<K, V> Set<T> Sender<T> Receiver<T> Mutex<T> Once<T>` are invariant. `Stream<T>` and `Task<T>` are covariant. <!-- block -->

```mote
let xs: List<Int> = []
let ys: List<Int | String> = xs // error
```

```mote
import { Stream } from std.collections

fn main() {
    let xs: List<Int> = [1, 2]
    let s: Stream<Int | String> = xs.stream()
    println(typeof(s))
}
```

```output
std.collections.Stream<String | Int>
```

`Pointer<T>` is described in [modules.md](modules.md#pointers).

## Nullable types

* `T?` is `T` or `null`, the same as `Option<T>`. `T??` is legal and different from `T?`. `null` and `None` are one value. <!-- block -->

```mote
fn main() {
    let a: Int? = 4
    let b: Int? = null
    let c: Option<Int> = a
    let d: Int?? = null
    let e: Int?? = a
    println(a)
    println(b == None)
    println(c)
    println(d == null)
    println(e == null)
    println(typeof(d))
}
```

```output
4
true
4
true
false
Int??
```

* A bare `T` converts to `T?`. See [optionals-errors.md](optionals-errors.md). <!-- block -->

```mote
fn main() {
    let a: Int? = 4
    println(a)
}
```

```output
4
```

## Union types

* `A | B` holds either; the value keeps its own type. Order is irrelevant and duplicates collapse. <!-- block -->

```mote
fn show(x: Int | String) -> String {
    if x is Int { return "int" }
    return "text"
}

fn main() {
    let a: Int | String = 1
    let b: String | Int = a
    let c: Int | String | Int = "s"
    println(show(a))
    println(show(b))
    println(show(c))
    println(typeof(c))
}
```

```output
int
int
text
String
```

* A union is always written, never inferred. A union member is not an optional: write `(Int | String)?`. <!-- block -->

```mote
let xs: List<Int | String> = [1, "a"]
let ys = [1, "a"] // error
```

```mote
let a: Int | String? = null // error
```

* `is` and a type pattern on `(Int | String)?` ask for a value of that member; `null` is none of them. <!-- block -->

```mote
fn name(x: (Int | String)?) -> String {
    return match x {
        n: Int => "int"
        s: String => "text"
        _ => "none"
    }
}

fn main() {
    println(name(1))
    println(name("a"))
    println(name(null))
    let y: (Int | String)? = 2
    println(y is Int)
    println(y is String)
}
```

```output
int
text
none
true
false
```

* After `is` and in a type pattern a union is written bare or in parentheses. <!-- block -->

```mote
fn f(x: Int | String | Null) -> Bool {
    return x is Int | String
}

fn g(x: Int | String | Null) -> Bool {
    return x is (Int | String)
}

fn kind(x: Int | String | Null) -> Int {
    return match x {
        n: Int | String => 1
        _ => 2
    }
}

fn main() {
    println(f(1))
    println(f(null))
    println(g(null))
    println(kind("s"))
    println(kind(null))
}
```

```output
true
false
false
1
2
```

* A lambda parameter and a function type as a union member need parentheses. <!-- block -->

```mote
fn main() {
    let h = |x: (Int | String)| x
    let g: ((Int) -> Int) | String = "s"
    println(h(1))
    println(g)
}
```

```output
1
s
```

```mote
let h = |x: Int | String| x // error
```

* `|` binds loosest; a function type's result takes the rest: `(Int) -> Int | String` returns `Int | String`. <!-- block -->

```mote
fn main() {
    let f: (Int) -> Int | String = |x| "s"
    println(f(1))
}
```

```output
s
```

* `==`, `!=` and `.to_string()` work on any union. A union has no fields and no other methods, and other operators need narrowing by `is` or `match`. <!-- block -->

```mote
fn main() {
    let a: Int | String = 1
    let b: Int | String = "s"
    println(a == b)
    println(a != b)
    println(b.to_string())
    if b is String {
        println(b.len())
    }
}
```

```output
false
true
s
1
```

```mote
let b: Int | String = "s"
let n = b.len() // error: has no methods
```

```mote
struct A {
    x: Int
}

struct B {
    x: Int
}

let u: A | B = B { x: 2 }
let n = u.x // error: A | B has no fields
```

```mote
let a: Int | String = 1
let b = a + 1 // error
```

## Tuples

* `(A, B)` is structural: same arity and element types means same type. `()` is the empty tuple, `(A,)` a 1-tuple, `(A)` is `A`. <!-- block -->

```mote
fn pair() -> (Int, String) { return (1, "a") }

fn main() {
    let t: (Int, String) = pair()
    let e: () = ()
    let one: (Int,) = (1,)
    let g: (Int) = 3
    println(t)
    println(e)
    println(one.0)
    println(typeof(one))
    println(typeof(g))
}
```

```output
(1, "a")
()
1
(Int,)
Int
```

* `t.0`, `t.1` index elements. `==` compares element by element. A tuple may be a `struct` field. <!-- block -->

```mote
struct S {
    t: (Int, Int)
}

fn main() {
    let t = (1, "a")
    println(t.0)
    println(t.1)
    println(t == (1, "a"))
    println(t == (1, "b"))
    println(S { t: (1, 2) }.t.1)
}
```

```output
1
a
true
false
2
```

## Function types

* `(A, B) -> R` takes `A` and `B`, returns `R`. A function with no result returns `null`. <!-- block -->

```mote
fn nothing() {}

fn main() {
    let add: (Int, Int) -> Int = |a, b| a + b
    let n: () -> Null = nothing
    println(add(1, 2))
    println(n() == null)
}
```

```output
3
true
```

* `var` before a parameter type marks a writable parameter: `(var Point) -> Null`. <!-- block -->

```mote
struct Point {
    var x: Int
}

fn move(var p: Point) {
    p.x = 5
}

fn main() {
    let f: (var Point) -> Null = move
    var p = Point { x: 1 }
    f(p)
    println(p.x)
}
```

```output
5
```

* `Send (A) -> R` is a function that may be called inside `spawn`. A plain function type is not Sendable. <!-- block -->

```mote
fn run(f: Send (Int) -> Int) {
    let t = spawn f(1)
}
```

```mote
fn run(f: (Int) -> Int) {
    spawn f(1) // error
}
```

## Type aliases

* `type Name = T` is another name for `T`, with parameters allowed. It names a type, not a constructor. <!-- block -->

```mote
type Id = Int | String
type Pair<T> = (T, T)
pub type Names = List<String>

fn main() {
    let a: Id = "x"
    let p: Pair<Int> = (1, 2)
    let n: Names = ["a"]
    println(a)
    println(p)
    println(n)
}
```

```output
x
(1, 2)
["a"]
```

```mote
type Names = List<String>
let n = Names() // error
```

* A cycle, wrong argument count or unknown name in an alias is an error. <!-- block -->

```mote
type A = B
type B = A // error
```

```mote
type Pair<T> = (T, T)
let p: Pair<Int, Int> = (1, 2) // error
```

```mote
type A = Missing // error
```

## `Self`

* `Self` inside a trait or type body is the declaring or implementing type. <!-- block -->

```mote
trait Same {
    fn same(self, other: Self) -> Bool
}

struct P: (Same) {
    x: Int

    fn same(self, other: Self) -> Bool { return self.x == other.x }
}

fn main() {
    println(P { x: 1 }.same(P { x: 1 }))
}
```

```output
true
```

## Assignability

* A value of type `A` fits `B` when one holds: same type; `B` is `T?` and `A` is `Null` or fits `T`; `B` is a union and `A` fits a member, or `A` is a union and every member fits `B`; both are `Stream` or both `Task` with fitting elements; both are the same invariant type with identical arguments. <!-- block -->

```mote
fn main() {
    let a: Int = 1
    let b: Int? = null
    let c: Int? = a
    let d: Int | String = a
    let e: Int | String | Null = d
    let xs: List<Int> = [1]
    let ys: List<Int> = xs
    println(c)
    println(e)
    println(ys)
}
```

```output
1
1
[1]
```

```mote
let a: Int | String = 1
let b: Int = a // error
```

* Function types fit when parameter counts agree, each expected parameter fits what the other takes, and the result fits. Any result fits a `Null` result. <!-- block -->

```mote
fn num(x: Int) -> Int { return x }
fn wide(x: Int | String) -> Int { return 1 }

fn main() {
    let a: (Int) -> Int = wide
    let b: (Int) -> Null = num
    println(a(1))
    println(b(1) == null)
}
```

```output
1
true
```

```mote
fn two(x: Int, y: Int) -> Int { return x }
let f: (Int) -> Int = two // error
```

## Type parameters inside a declaration

* `T` is opaque: assignable to itself, comparable to `null`, and nothing else. Every other operation comes from a bound: `.to_string()` needs `T: Display`, and `+` is an error. <!-- block -->

```mote
import { Display } from std.traits

fn show<T: Display>(x: T) -> String { return x.to_string() }

fn same<T>(a: T) -> T {
    let b: T = a
    return b
}

fn main() {
    println(show(1))
    println(same(2))
}
```

```output
1
2
```

```mote
fn add<T>(a: T, b: T) -> T { return a + b } // error
```

```mote
fn show<T>(x: T) -> String { return x.to_string() } // error
```

## Methods of built-in types

* A method of a built-in type is declared in `std` on the type's lang declaration: `Int` on the `int` declaration, `[T]` on `Array<T>`, `T?` on `Option<T>`, `String` on `string`. Tuples, function types and unions have no methods besides `==` and `.to_string()`. <!-- block -->

```mote
fn main() {
    println(3.max(5))
    println([1, 2].len())
    let a: [Int] = [1, 2, 3]
    println(a.len())
    println(Some(4).unwrap())
    println("ab".to_upper())
    println((1, 2).to_string())
}
```

```output
5
2
3
4
AB
(1, 2)
```

```mote
let t = (1, 2)
t.len() // error
```

## Sendable types

* A value may enter `spawn` only if Sendable: numbers, `Bool`, `Null`, `String`; value structs of Sendable fields; classes with no `var` field and Sendable field types; tuples, enums and `Result` of Sendable parts; `Sender Receiver Task Mutex Once atomic.Int64 atomic.Bool PortEnd`; a nullable of a Sendable; a named function or a lambda capturing no `var`. <!-- block -->

```mote
import { Channel, Mutex } from std.sync
import std.atomic as atomic

struct P {
    x: Int
}

class C {
    x: Int
}

fn work(n: Int) -> Int { return n }

fn main() {
    let a = spawn work(1)
    let b = spawn P { x: 1 }.x
    let c = spawn C { x: 2 }.x
    let t = spawn (1, "a")
    let m = Mutex(0)
    let d = spawn m.lock(|v| v)
    let n = atomic.Int64(0)
    let e = spawn n.load()
    let o: Int? = 4
    let f = spawn o
    println(a.await())
    println(b.await())
    println(c.await())
    println(t.await())
    println(d.await())
    println(e.await())
    println(f.await())
}
```

```output
Ok(1)
Ok(1)
Ok(2)
Ok((1, "a"))
Ok(0)
Ok(0)
Ok(4)
```

```mote
var xs: List<Int> = []
let t = spawn xs.len() // error
```

```mote
class Box {
    var n: Int
}

fn read(b: Box) -> Int {
    return b.n
}

fn main() {
    let b = Box { n: 0 }
    let t = spawn read(b) // error: `b` is not Sendable: Box
}
```
