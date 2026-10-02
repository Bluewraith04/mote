# Generics and traits

## Generic functions and types

A type parameter lets one declaration work for many types. A call fills it in from its arguments, or from the expected type; when nothing decides it, the call must write it out (`first<Int>(xs)`).

```mote
fn first<T>(xs: List<T>) -> T? {
    return xs.first()
}

class Stack<T> {
    var items: List<T>

    pub fn new() -> Stack<T> {
        return Stack { items: [] }
    }

    pub fn push(var self, x: T) {
        self.items.push(x)
    }

    pub fn pop(var self) -> T? {
        return self.items.pop()
    }
}

struct Pair<A, B> {
    a: A
    b: B
}

fn main() {
    let n: Int = first([7, 8])!
    println(n)
    var s: Stack<Int> = Stack.new()
    s.push(1)
    let p = Pair { a: 1, b: "x" }
    println(s.pop()!)
    println(p.b)
}
```

```output
7
1
x
```

`T?` and `Result<T, E>` are ordinary generic types, and their payload types flow through `?`, `!`, `unwrap_or` and `match` bindings ([Absence and failure](errors.md)). Types are invariant: a `Stack<Int>` is not a `Stack<Any>`. A bare `Stack` is an error. A generic function used as a value takes its type arguments from the function type expected there.

Inside a body, `T` is unknown: it can be passed on, printed with `.to_string()`, and used with `+` (a type that cannot take the operator faults at that line). Anything else needs a bound. A value still records its type arguments, so `typeof([1])` is `"List<Int>"`.

## Traits

A trait lists method signatures, with `Self` for the implementing type. A member with a body is a default.

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

println(Square { side: 2 }.describe())
```

```output
area 4
```

A type satisfies a trait when it has a matching method for every member; nothing needs declaring. Listing the trait (`class Square: (Shape)`) also checks the methods at the declaration and copies the defaults in.

### Bounds

`<T: Shape>` says `T` must satisfy `Shape`, and the body may call its methods. Several bounds are written `T: A + B`.

```mote
trait Shape {
    fn area(self) -> Int
}

class Square {
    side: Int

    pub fn area(self) -> Int {
        return self.side * self.side
    }
}

fn total<T: Shape>(shapes: List<T>) -> Int {
    var sum = 0
    for s in shapes {
        sum = sum + s.area()
    }
    return sum
}

println(total([Square { side: 2 }, Square { side: 3 }]))
```

```output
13
```

Each concrete type gets its own compiled copy, so a call is a direct call and there are no trait objects. Not supported: bounds on a type's own parameters (`class Box<T: Shape>`), bounded generic methods, traits with parameters, and one trait extending another.

### Built-in traits

`Display`, `Debug`, `Eq` and `Ord` are always in scope. `Int`, `Float`, `Char` and `String` satisfy all four; `Bool` satisfies all but `Ord`. `x.compare(y)` gives `-1`, `0` or `1`, and `std.iter.sort` needs `T: Ord`.

### Deriving

`@derive(Display, Debug)` adds `to_string` and `debug` to a struct, class or enum. `@derive(Json)` and `@derive(Args)` are covered in [std.data.json](std/json.md) and [std.dev.args](std/args.md).

```mote
@derive(Display, Debug)
class Point {
    x: Int
    y: Int
}

let p = Point { x: 1, y: 2 }
println(p.to_string())
println(p.debug())
```

```output
Point(x: 1, y: 2)
Point { x: 1, y: 2 }
```
