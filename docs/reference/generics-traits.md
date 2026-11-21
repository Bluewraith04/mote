# Generics and traits

## Type parameters

* `<T, U>` follows the name of a function, method, struct, class, enum, trait or alias. <!-- block -->

```mote
fn first<T>(xs: List<T>) -> T? {
    return xs.first()
}

struct Pair<A, B> {
    a: A
    b: B

    fn swap(self) -> Pair<B, A> {
        return Pair { a: self.b, b: self.a }
    }

    fn map_a<C>(self, f: (A) -> C) -> Pair<C, B> {
        return Pair { a: f(self.a), b: self.b }
    }
}

class Stack<T> {
    var items: List<T> = []
}

enum Either<L, R> {
    Left(L)
    Right(R)
}

trait Convert<T> {
    fn convert(self) -> T
}

type Names<T> = Map<String, T>

fn main() {
    println(first([4]))
    println(Pair { a: 1, b: "x" }.swap().a)
    println(Pair { a: 1, b: "x" }.map_a(|n| n + 1).a)
    println(Stack<Int>().items.len())
    let e: Either<Int, String> = Either.Left(1)
    let n: Names<Int> = {"a": 1}
    println(n["a"])
}
```

```output
4
x
2
0
1
```

* A call infers each parameter from its arguments, or writes them. <!-- block -->

```mote
fn first<T>(xs: List<T>) -> T? {
    return xs.first()
}

struct Pair<A, B> {
    a: A
    b: B
}

fn main() {
    let a = first([1, 2])
    let b = first<Int>([])
    let p = Pair<Int, String> { a: 1, b: "x" }
    println(a)
    println(b == null)
    println(p.b)
}
```

```output
1
true
x
```

* The arguments fix each parameter first, left to right; the expected type fixes what they leave open. A `null` argument fixes nothing. <!-- block -->

```mote
fn pick<T>(a: T, b: T) -> T {
    return a
}

fn both<T>(a: T?, b: T) -> T {
    return b
}

fn none<T>() -> List<T> {
    return []
}

fn main() {
    println(pick(1, 2))
    println(both(null, 3))
    let xs: List<Int> = none()
    println(xs.len())
}
```

```output
1
3
0
```

```mote
fn pick<T>(a: T, b: T) -> T {
    return a
}

let x = pick(1, "a") // error
```

* A type argument no context fixes is an error; it is never guessed. <!-- block -->

```mote
fn none<T>() -> List<T> {
    return []
}

let xs = none() // error
```

```mote
fn only_null<T>(a: T?) -> Int {
    return 1
}

let n = only_null(null) // error
```

* A struct or enum literal, and a variant, fix their parameters from their fields the same way: `Pair { a: 1, b: "x" }` is a `Pair<Int, String>`. A `null` field fixes nothing, and `Either.Left(1)` leaves `R` open and is an error. <!-- block -->

```mote
struct Pair<A, B> {
    a: A
    b: B
}

enum Either<L, R> {
    Left(L)
    Right(R)
}

fn main() {
    let p = Pair { a: 1, b: "x" }
    println(typeof(p.a))
    println(typeof(p.b))
    let q: Pair<Int?, String> = Pair { a: null, b: "y" }
    println(q.b)
    let e: Either<Int, String> = Either.Left(1)
    match e {
        Left(n) => println(n)
        Right(s) => println(s)
    }
}
```

```output
Int
String
y
1
```

```mote
enum Either<L, R> {
    Left(L)
    Right(R)
}

let e = Either.Left(1) // error
```

```mote
struct Holder<T> {
    value: T?
}

let h = Holder { value: null } // error
```

* A generic function used as a value takes its type arguments from the expected type: `let f: (Int) -> Int = id`. <!-- block -->

```mote
fn id<T>(x: T) -> T {
    return x
}

fn main() {
    let f: (Int) -> Int = id
    println(f(4))
}
```

```output
4
```

```mote
fn id<T>(x: T) -> T {
    return x
}

let f = id // error
```

* Type arguments are written on a call, method call, or type, never on a function value. <!-- block -->

```mote
fn id<T>(x: T) -> T {
    return x
}

let f = id<Int> // error
```

```mote
struct Box {
    fn get<T>(self, x: T) -> T {
        return x
    }
}

fn main() {
    println(Box {}.get<Int>(1))
}
```

```output
1
```

## Bounds

* `T: Trait` limits a parameter; `T: A + B` needs both. The body may call the trait's methods. <!-- block -->

```mote
import { Display, Ord } from std.traits

fn largest<T: Ord>(xs: List<T>) -> T? {
    var best: T? = null
    for x in xs {
        if best == null || x.compare(best!) > 0 {
            best = x
        }
    }
    return best
}

fn show_largest<T: Ord + Display>(a: T, b: T) -> String {
    if a.compare(b) > 0 {
        return a.to_string()
    }
    return b.to_string()
}

fn main() {
    println(largest([3, 9, 4]))
    println(show_largest(2, 1))
    println(show_largest("a", "b"))
}
```

```output
9
2
b
```

```mote
import Ord from std.traits

fn both<T: Ord + Display>(a: T) -> Int { // error
    return 1
}
```

* A bound is checked at each call: the type needs every method of the trait that has no default, visible to the caller, with the trait's parameters and result and `Self` as the type. Each concrete type gets its own compiled copy, so dispatch is static. <!-- block --> <!-- test: a_bound_is_met_by_a_type_with_the_methods_and_checked_at_each_call, a_call_through_a_bound_reaches_the_types_own_method, a_bound_needs_methods_the_caller_sees_and_a_pub_trait_is_imported_like_a_type -->

```mote
trait Shape {
    fn area(self) -> Int
    fn describe(self) -> String {
        return "area ${self.area()}"
    }
}

struct Square {
    side: Int

    pub fn area(self) -> Int {
        return self.side * self.side
    }
}

struct Circle {
    r: Int

    pub fn area(self) -> Int {
        return 3 * self.r * self.r
    }
}

fn total<T: Shape>(shape: T) -> Int {
    return shape.area()
}

fn main() {
    println(total(Square { side: 2 }))
    println(total(Circle { r: 1 }))
}
```

```output
4
3
```

```mote
trait Shape {
    fn area(self) -> Int
}

struct Blob {}

fn total<T: Shape>(shape: T) -> Int {
    return shape.area()
}

let n = total(Blob {}) // error
```

```mote
trait Shape {
    fn area(self) -> Int
}

struct Wrong {
    pub fn area(self) -> String {
        return "x"
    }
}

fn total<T: Shape>(shape: T) -> Int {
    return shape.area()
}

let n = total(Wrong {}) // error
```

* Only the copies a program reaches from `main`, a test, a global or top-level code are built. A function whose type arguments grow at each call, such as `f((x, x))` inside `f<T>`, is an error once they nest 32 deep. <!-- block -->

```mote
fn grow<T>(x: T) {
    grow((x, x))
}

fn main() {
    println("not reached")
}
```

```output
not reached
```

```mote
fn grow<T>(x: T) {
    grow((x, x)) // error: type arguments nest too deeply
}

fn main() {
    grow(1)
}
```

* `T: Eq` allows `==` on `T`; `T == null` needs no bound. <!-- block -->

```mote
import Eq from std.traits

fn same<T: Eq>(a: T, b: T) -> Bool {
    return a == b
}

fn missing<T>(x: T?) -> Bool {
    return x == null
}

fn main() {
    println(same(1, 1))
    println(same("a", "b"))
    let o: Int? = null
    println(missing(o))
}
```

```output
true
false
true
```

```mote
fn same<T>(a: T, b: T) -> Bool {
    return a == b // error
}
```

* A method of a bound's trait that has no `self` is called on the parameter: `T.make()` with `T: Maker`. <!-- block -->

```mote
trait Maker {
    fn make() -> Self
}

struct Item: (Maker) {
    n: Int

    pub fn make() -> Item {
        return Item { n: 7 }
    }
}

fn build<T: Maker>() -> T {
    return T.make()
}

fn main() {
    let item: Item = build()
    println(item.n)
}
```

```output
7
```

* A type's own parameters take bounds too, `class Set<T: Key>`. Each use of the type checks them: an annotation, a literal, a signature, a field, a method call. <!-- block --> <!-- test: a_types_own_bounds_are_checked_where_it_is_used -->

```mote
import Ord from std.traits

class Sorted<T: Ord> {
    items: List<T>

    fn first(self) -> T? {
        return self.items.first()
    }

    fn make(item: T) -> Sorted<T> {
        return Sorted { items: [item] }
    }
}

fn main() {
    let s: Sorted<Int> = Sorted { items: [3, 1] }
    println(s.first())
    println(Sorted.make(5).first())
}
```

```output
3
5
```

```mote
import Ord from std.traits

class Sorted<T: Ord> {
    items: List<T>
}

struct Plain {}

let s: Sorted<Plain> = Sorted { items: [] } // error
```

```mote
import Ord from std.traits

class Sorted<T: Ord> {
    items: List<T>
}

struct Plain {}

let s = Sorted { items: [Plain {}] } // error
```

```mote
import Ord from std.traits

class Sorted<T: Ord> {
    items: List<T>
}

struct Plain {}

fn use_it(s: Sorted<Plain>) {} // error
```

```mote
import Ord from std.traits

class Sorted<T: Ord> {
    items: List<T>
}

struct Plain {}

struct Holder {
    s: Sorted<Plain> // error
}
```

```mote
import Ord from std.traits

class Sorted<T: Ord> {
    items: List<T>

    fn make(item: T) -> Sorted<T> {
        return Sorted { items: [item] }
    }
}

struct Plain {}

let s = Sorted.make(Plain {}) // error
```

* A type parameter without a bound is opaque. See [types.md](types.md). <!-- block -->

```mote
fn bad<T>(x: T) -> String {
    return x.to_string() // error
}
```

## `trait`

* A trait lists method signatures; a member with a body is a default. `Self` is the implementing type. <!-- block -->

```mote
trait Shape {
    fn area(self) -> Int
    fn describe(self) -> String {
        return "area ${self.area()}"
    }
}

trait Dup {
    fn dup(self) -> Self
}

struct Square: (Shape, Dup) {
    side: Int

    fn area(self) -> Int {
        return self.side * self.side
    }

    fn dup(self) -> Square {
        return Square { side: self.side }
    }
}

fn main() {
    let s = Square { side: 3 }
    println(s.describe())
    println(s.dup().side)
}
```

```output
area 9
3
```

* A trait takes type parameters: `trait Convert<T> { fn convert(self) -> T }`. A bound writes them, `U: Convert<Int>`, and a type lists the trait with arguments, `struct A: (Convert<Int>)`. A default body may name the trait's parameters. <!-- block --> <!-- test: a_trait_takes_type_arguments_in_bounds_and_lists, a_default_method_of_a_generic_trait_reads_the_listed_arguments -->

```mote
trait Convert<T> {
    fn convert(self) -> T

    fn twice(self) -> List<T> {
        let one: T = self.convert()
        return [one, one]
    }
}

struct A: (Convert<Int>) {
    n: Int

    fn convert(self) -> Int {
        return self.n
    }
}

fn run<U: Convert<Int>>(u: U) -> Int {
    return u.convert() + 1
}

fn main() {
    println(run(A { n: 4 }))
    println(A { n: 2 }.twice().len())
}
```

```output
5
2
```

## Implementing a trait

* A type lists traits after its name in parentheses. There is no `impl` block. <!-- block -->

```mote
trait Shape {
    fn area(self) -> Int
    fn describe(self) -> String {
        return "area ${self.area()}"
    }
}

class Square: (Shape) {
    side: Int

    fn area(self) -> Int {
        return self.side * self.side
    }
}

fn main() {
    println(Square { side: 2 }.describe())
}
```

```output
area 4
```

```mote
trait Shape {
    fn area(self) -> Int
}

struct Square {
    side: Int
}

impl Shape for Square { // error
    fn area(self) -> Int {
        return 1
    }
}
```

<!-- block --> <!-- test: a_listed_trait_is_checked_and_gives_its_defaults, a_bound_needs_methods_the_caller_sees_and_a_pub_trait_is_imported_like_a_type -->
| Rule | Detail |
|---|---|
| Listing | copies each default the type lacks, makes the trait's methods `pub`, and checks every method exists with matching parameter and return types; a missing or different one is an error |
| Not listing | a type with matching methods still satisfies a bound; its defaults are not copied |
| Inheritance | `class C: Parent` is an error; a class has no parent |
| `pub trait` | imported like a type |

```mote
trait Shape {
    fn area(self) -> Int
    fn describe(self) -> String {
        return "area ${self.area()}"
    }
}

struct Listed: (Shape) {
    fn area(self) -> Int {
        return 1
    }
}

struct Unlisted {
    pub fn area(self) -> Int {
        return 2
    }
}

fn total<T: Shape>(shape: T) -> Int {
    return shape.area()
}

fn main() {
    println(Listed {}.describe())
    println(total(Unlisted {}))
}
```

```output
area 1
2
```

```mote
trait Shape {
    fn area(self) -> Int
}

struct Missing: (Shape) {} // error
```

```mote
trait Shape {
    fn area(self) -> Int
}

struct Different: (Shape) { // error
    fn area(self) -> String {
        return "x"
    }
}
```

```mote
trait Shape {
    fn area(self) -> Int
    fn describe(self) -> String {
        return "d"
    }
}

struct Unlisted {
    pub fn area(self) -> Int {
        return 2
    }
}

let d = Unlisted {}.describe() // error
```

```mote
class Parent {}

class C: Parent {} // error
```

## Traits in the language

* From `std.traits`. <!-- block -->

<!-- block -->
| Trait | Required method |
|---|---|
| `Display` | `fn to_string(self) -> String` |
| `Debug` | `fn debug(self) -> String` |
| `Eq` | `fn eq(self, other: Self) -> Bool` |
| `Ord` | `fn compare(self, other: Self) -> Int` giving `-1`, `0` or `1` |
| `Clone` | `fn clone(self) -> Self`: a new value whose fields are the same values |
| `Closeable` | `fn close(self) -> Result<Null, Error>`; what `with` needs |

```mote
import { Display, Debug, Eq, Ord, Clone, Closeable } from std.traits

struct Num: (Display, Debug, Eq, Ord, Clone, Closeable) {
    n: Int

    fn to_string(self) -> String {
        return "num ${self.n}"
    }

    fn debug(self) -> String {
        return "Num<${self.n}>"
    }

    fn eq(self, other: Num) -> Bool {
        return self.n == other.n
    }

    fn compare(self, other: Num) -> Int {
        if self.n < other.n {
            return -1
        }
        if self.n > other.n {
            return 1
        }
        return 0
    }

    fn clone(self) -> Num {
        return Num { n: self.n }
    }

    fn close(self) -> Result<Null, Error> {
        println("closed")
        return Ok(null)
    }
}

fn main() {
    let a = Num { n: 1 }
    let b = Num { n: 2 }
    println(a.to_string())
    println(a.debug())
    println(a.eq(b))
    println(a.compare(b))
    println(b.compare(a))
    println(a.compare(a))
    println(a.clone().n)
    with r = a {
        println("body")
    }
    println(1.compare(2))
    println("b".compare("a"))
}
```

```output
num 1
Num<1>
false
-1
1
0
1
body
closed
-1
1
```

* Numbers and `String` satisfy `Display Debug Eq Ord Clone`; `Bool` all but `Ord`. <!-- block -->

```mote
import { Display, Debug, Eq, Ord, Clone } from std.traits

fn all<T: Display + Debug + Eq + Ord + Clone>(x: T) -> String {
    return x.to_string()
}

fn most<T: Display + Debug + Eq + Clone>(x: T) -> String {
    return x.to_string()
}

fn main() {
    println(all(1))
    println(all(2.5))
    println(all(3u8))
    println(all(-4i16))
    println(all("s"))
    println(most(true))
}
```

```output
1
2.5
3
-4
s
true
```

```mote
import Ord from std.traits

fn ordered<T: Ord>(x: T) -> Int {
    return 1
}

let n = ordered(true) // error
```

* `List`, `Map`, `Set` and arrays have `clone`. A class has `clone` only when it lists `Clone` or writes the method; a struct is a value and copies on assignment. <!-- block -->

```mote
import { Set, set_from } from std.collections

fn main() {
    let xs = [1, 2]
    let a: [Int] = [3, 4]
    let m = {"k": 1}
    let s = set_from([5])
    println(xs.clone().len())
    println(a.clone().len())
    println(m.clone().len())
    println(s.clone().len())
}
```

```output
2
2
1
1
```

```mote
class Plain {
    var n: Int = 0
}

let c = Plain().clone() // error
```

```mote
import Clone from std.traits

class Listed: (Clone) {
    var n: Int = 1
}

fn main() {
    let a = Listed()
    let b = a.clone()
    println(a == b)
    println(b.n)
}
```

```output
false
1
```

## Derived methods

* `Display`, `Debug` and `Clone` have default methods that inspect the type statically via `std.reflect` and `comptime` builtins. A type that lists one and writes none of its methods gets the default; a method it writes is kept. <!-- block -->

```mote
import { Display, Debug, Clone } from std.traits

struct Point: (Display, Debug) {
    x: Int
    y: Int
}

struct Custom: (Display) {
    n: Int

    fn to_string(self) -> String {
        return "custom"
    }
}

fn main() {
    println(Point { x: 3, y: 4 }.to_string())
    println(Point { x: 3, y: 4 }.debug())
    println(Custom { n: 1 }.to_string())
}
```

```output
Point(x: 3, y: 4)
Point { x: 3, y: 4 }
custom
```

<!-- block -->
| Trait | Default | Result |
|---|---|---|
| `Display` | `to_string` | `Point(x: 3, y: 4)`, what `print` writes; used by `${}` and `.to_string()` |
| `Debug` | `debug` | `Point { x: 3, y: 4 }`; a variant with a tuple payload as `Circle(1.5)` |
| `Clone` | `clone` | a new struct, class, tuple or enum value with the same parts; objects held in fields are shared |

```mote
import { Display, Debug, Clone } from std.traits

struct Point: (Display, Debug) {
    x: Int
    y: Int
}

enum Shape: (Debug) {
    Circle(Float)
    Empty
}

class Inner {
    var n: Int = 0
}

class Account: (Clone) {
    var owner: String
    var inner: Inner
}

fn main() {
    let p = Point { x: 3, y: 4 }
    print(p)
    println("")
    println("${p}")
    println(p.to_string())
    println(p.debug())
    println(Shape.Circle(1.5).debug())
    let a = Account { owner: "o", inner: Inner() }
    let b = a.clone()
    println(a == b)
    println(a.inner == b.inner)
    println(a.owner == b.owner)
}
```

```output
Point(x: 3, y: 4)
Point(x: 3, y: 4)
Point(x: 3, y: 4)
Point { x: 3, y: 4 }
Circle(1.5)
false
true
true
```

* A field is shown by its own fields, whatever methods its type writes. <!-- block -->

```mote
import Display from std.traits

struct Inner: (Display) {
    n: Int

    fn to_string(self) -> String {
        return "custom inner"
    }
}

struct Outer: (Display) {
    inner: Inner
}

fn main() {
    println(Outer { inner: Inner { n: 1 } }.to_string())
}
```

```output
Outer(inner: Inner(n: 1))
```

* `Eq`, `Ord` and `Closeable` have no default; a type that lists one writes it. <!-- block --> <!-- test: derive_a_trait_without_a_default_still_needs_its_method -->

```mote
import Eq from std.traits

struct P: (Eq) { // error
    n: Int
}
```

```mote
import Ord from std.traits

struct P: (Ord) { // error
    n: Int
}
```

```mote
import Closeable from std.traits

struct P: (Closeable) { // error
    n: Int
}
```

## `Self`

* `Self` names the implementing type inside a trait. <!-- block -->

```mote
trait Dup {
    fn dup(self) -> Self
}

struct Cell: (Dup) {
    n: Int

    fn dup(self) -> Cell {
        return Cell { n: self.n + 1 }
    }
}

fn twice<T: Dup>(x: T) -> T {
    return x.dup().dup()
}

fn main() {
    println(twice(Cell { n: 0 }).n)
}
```

```output
2
```

## Variance

* Built-in generic types are invariant except `Stream` and `Task`. See [types.md](types.md). <!-- block -->

```mote
fn takes(xs: List<Int?>) {}

fn main() {
    let xs: List<Int> = [1]
    takes(xs) // error
}
```
