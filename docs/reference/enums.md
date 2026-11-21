# Enums

## Declaration

* An `enum` is a closed set of variants. Variants are separated by newlines or commas. Methods follow the variants. <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Rect(Int, Int)
    Named { name: String, sides: Int }
    Empty
}

enum Pair { A, B(Int), C }

enum Light {
    Red
    Green

    pub fn next(self) -> Light {
        return match self {
            Red => Green
            Green => Red
        }
    }
}

fn main() {
    println(Shape.Circle(1) == Shape.Circle(1))
    println(Pair.C == Pair.C)
    println(Light.Red.next() == Light.Green)
}
```

```output
true
true
true
```

<!-- block -->
| Variant form | Declared | Built | Matched |
|---|---|---|---|
| unit | `Empty` | `Shape.Empty` | `Empty` |
| tuple | `Circle(Int)` | `Shape.Circle(2)` | `Circle(r)` |
| struct | `Named { name: String }` | `Shape.Named { name: "a", sides: 3 }` | `Named { name, .. }` |

```mote
enum Shape {
    Circle(Int)
    Named { name: String, sides: Int }
    Empty
}

fn show(s: Shape) {
    match s {
        Empty => println("empty")
        Circle(r) => println(r)
        Named { name, .. } => println(name)
    }
}

fn main() {
    show(Shape.Empty)
    show(Shape.Circle(2))
    show(Shape.Named { name: "a", sides: 3 })
}
```

```output
empty
2
a
```

* A variant is written bare, `Circle(2)`, where the expected type is known: an annotation, a parameter, a return, or an arm of a `match` on that type. With no expected type it is qualified, `Shape.Circle(2)`. A variant name is never in scope on its own. <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Empty
}

fn area(s: Shape) -> Int {
    match s {
        Circle(r) => return r
        Empty => return 0
    }
}

fn make() -> Shape {
    return Circle(5)
}

fn main() {
    let s: Shape = Circle(2)
    let t = Shape.Circle(3)
    println(area(s))
    println(area(t))
    println(area(Circle(4)))
    println(area(make()))
}
```

```output
2
3
4
5
```

```mote
enum Shape {
    Circle(Int)
}

let t = Circle(2) // error
```

* A lowercase bare name in a pattern that is not a variant of the subject's type binds a value; an uppercase one is an error. <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Empty
}

fn main() {
    match Shape.Empty {
        Circle(r) => println(r)
        other => println("other")
    }
}
```

```output
other
```

```mote
enum Shape {
    Circle(Int)
    Empty
}

match Shape.Empty {
    Circle(r) => println(r)
    Other => println("other") // error
}
```

* A unit variant is never called, `Empty()`; a struct variant literal takes no `..base`. <!-- block -->

```mote
enum Shape {
    Named { name: String, sides: Int }
    Empty
}

let e = Shape.Empty() // error
```

```mote
enum Shape {
    Named { name: String, sides: Int }
}

let a = Shape.Named { name: "a", sides: 3 }
let b = Shape.Named { ..a, sides: 4 } // error
```

## Matching

* `match` takes every variant or a catch-all. See [patterns.md](patterns.md). <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Rect(Int, Int)
    Named { name: String, sides: Int }
    Empty
}

fn area(s: Shape) -> Int {
    match s {
        Circle(r) => return 3 * r * r
        Rect(w, h) => return w * h
        Named { sides, .. } => return sides
        Empty => return 0
    }
}

fn main() {
    println(area(Shape.Circle(2)))
    println(area(Shape.Rect(2, 3)))
    println(area(Shape.Named { name: "a", sides: 5 }))
    println(area(Shape.Empty))
}
```

```output
12
6
5
0
```

```mote
enum Shape {
    Circle(Int)
    Empty
}

fn area(s: Shape) -> Int {
    match s { // error
        Circle(r) => return r
    }
}
```

* A value `match` yields one expression per arm. <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Rect(Int, Int)
    Empty
}

fn main() {
    let s = Shape.Rect(1, 2)
    let n = match s {
        Circle(r) => r
        Rect(w, h) => w + h
        _ => 0
    }
    println(n)
}
```

```output
3
```

## Equality

* `==` compares the variant, then the payload. <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Rect(Int, Int)
}

fn main() {
    println(Shape.Circle(1) == Shape.Circle(1))
    println(Shape.Circle(1) == Shape.Circle(2))
    println(Shape.Circle(1) == Shape.Rect(1, 1))
    println(Shape.Rect(1, 2) == Shape.Rect(1, 2))
}
```

```output
true
false
false
true
```

## Methods

* A method is a `fn` after the variants. `self` is read-only; `var self` is an error. <!-- block -->

```mote
enum Light {
    Red
    Green

    pub fn next(self) -> Light {
        return match self {
            Red => Green
            Green => Red
        }
    }

    pub fn start() -> Light {
        return Light.Red
    }
}

fn main() {
    println(Light.start().next() == Light.Green)
}
```

```output
true
```

```mote
enum Light {
    Red
    Green

    pub fn flip(var self) { // error
    }
}
```

* A method cannot share a variant's name. <!-- block -->

```mote
enum Light {
    Red
    Green

    pub fn Red(self) -> Int { // error
        return 1
    }
}
```

## Traits

* `enum Light: (Display)` lists traits it implements. <!-- block -->

```mote
import Display from std.traits

enum Light: (Display) {
    Red
    Green
}

fn main() {
    println(Light.Green.to_string())
}
```

```output
Green
```

## Payload types

* A payload has its instantiated type. A recursive payload is allowed in a class or in the enum itself. <!-- block -->

```mote
enum Tree {
    Leaf(Int)
    Node(Tree, Tree)
}

class Wrap {
    var shape: Shape? = null
}

enum Shape {
    Boxed(Wrap)
    Plain
}

fn sum(t: Tree) -> Int {
    return match t {
        Leaf(n) => n
        Node(a, b) => sum(a) + sum(b)
    }
}

fn main() {
    println(sum(Tree.Node(Tree.Leaf(1), Tree.Node(Tree.Leaf(2), Tree.Leaf(3)))))
    let w = Wrap()
    println(w.shape == null)
}
```

```output
6
true
```

## Type parameters

* `enum Either<L, R> { Left(L) Right(R) }`. <!-- block -->

```mote
enum Either<L, R> {
    Left(L)
    Right(R)
}

fn main() {
    let e: Either<Int, String> = Either.Left(1)
    match e {
        Left(n) => println(n + 1)
        Right(s) => println(s)
    }
}
```

```output
2
```

## Unit variants with values

* A unit variant may carry an integer constant. Two variants cannot share one. <!-- block -->

```mote
enum Level {
    Low = 1
    High = 10
}

fn main() {
    println(Level.High as Int)
}
```

```output
10
```

```mote
enum Level {
    Low = 1
    High = 1 // error
}
```

* `e as Int` reads the constant. It needs every variant to be a unit variant with a constant, and each constant to fit the target integer type. There is no cast back. <!-- block -->

```mote
enum Level {
    Low = 1
    High = 10
}

fn main() {
    println(Level.High as Int)
    println(Level.Low as UInt8)
}
```

```output
10
1
```

```mote
enum Level {
    Low = 1
    High
}

let n = Level.Low as Int // error
```

```mote
enum Level {
    Low = 1
    Some(Int)
}

let n = Level.Low as Int // error
```

```mote
enum Level {
    Low = 1
    High = 300
}

let n = Level.Low as UInt8 // error
```

```mote
enum Level {
    Low = 1
}

let n = 1 as Level // error
```

## Enum or union

<!-- block -->
| Use | When |
|---|---|
| `enum` | the cases are named, carry different payload shapes or have methods |
| `A \| B` | the cases are existing types, and nothing needs a name |

```mote
enum Shape {
    Circle(Int)
    Rect(Int, Int)

    pub fn describe(self) -> String {
        return match self {
            Circle(r) => "circle ${r}"
            Rect(w, h) => "rect ${w}x${h}"
        }
    }
}

fn show(v: Int | String) {
    match v {
        n: Int => println(n + 1)
        s: String => println(s)
    }
}

fn main() {
    println(Shape.Rect(1, 2).describe())
    show(1)
    show("a")
}
```

```output
rect 1x2
2
a
```

## Built-in enums

* `Result<T, E>` is `Ok(T)` or `Err(E)`; an optional `T?` is `Some(T)` or `None`. See [optionals-errors.md](optionals-errors.md). <!-- block -->

```mote
fn main() {
    let r: Result<Int, String> = Ok(1)
    match r {
        Ok(n) => println(n)
        Err(e) => println(e)
    }
    let o: Int? = null
    match o {
        Some(n) => println(n)
        None => println("none")
    }
}
```

```output
1
none
```
