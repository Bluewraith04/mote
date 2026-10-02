# Functions

A function has a name, typed parameters and an optional result type. A function with a result type must `return` on every path; one without returns `null`.

```mote
fn add(a: Int, b: Int) -> Int {
    return a + b
}

fn greet(name: String) {
    println("hi " + name)
}

println(add(2, 3))
greet("Ada")
```

```output
5
hi Ada
```

## Parameters

Defaults go last, and a call may omit any trailing parameter that has one. A default cannot mention another parameter.

```mote
fn box(width: Int, height: Int = 1, label: String = "box") -> String {
    return "${label} ${width}x${height}"
}

println(box(4))
println(box(4, 2))
println(box(4, 2, "crate"))
```

```output
box 4x1
box 4x2
crate 4x2
```

The last parameter may be `...name: Type`; the remaining arguments arrive as a `List`. A function with `...` cannot also have defaults.

```mote
fn total(...xs: Int) -> Int {
    var sum = 0
    for x in xs {
        sum += x
    }
    return sum
}

println(total(1, 2, 3))
println(total())
```

```output
6
0
```

A plain parameter is read-only. A `var` parameter may be changed, and the caller sees the change. The argument is written plainly at the call and must be something the caller can write.

```mote
struct Point {
    var x: Int
    var y: Int
}

fn shift(var p: Point) {
    p.x += 10
}

fn bump(var n: Int) {
    n += 1
}

var p = Point { x: 1, y: 2 }
var count = 0
shift(p)
bump(count)
println(p.x)
println(count)
```

```output
11
1
```

## Functions are values

A function's name can be stored, passed and called. The type is written `(ArgTypes) -> Result`. A method is not a value by itself; wrap it in a lambda.

```mote
fn double(n: Int) -> Int {
    return n * 2
}

fn apply(f: (Int) -> Int, x: Int) -> Int {
    return f(x)
}

println(apply(double, 21))
```

```output
42
```

## Lambdas

A lambda is written between `|` bars, with one expression or a block as its body.

| Form | Meaning |
|---|---|
| `\|x\| x * 2` | one expression |
| `\|a, b\| { return a + b }` | a block body |
| `\|a: Job, b: Job\| a.weight > b.weight` | written parameter types |
| `\|var s\| { s.count += 1 }` | a parameter the lambda may change |

Unwritten parameter types come from where the lambda is used.

```mote
fn twice(f: (Int) -> Int, x: Int) -> Int {
    return f(f(x))
}

println(twice(|n| n * 3, 2))
let add: (Int, Int) -> Int = |a, b| a + b
println(add(4, 5))
```

```output
18
9
```

A lambda shares the variables around it, and keeps them after the function that made it returns.

```mote
fn counter() -> () -> Int {
    var n = 0
    return || {
        n += 1
        return n
    }
}

let next = counter()
println(next())
println(next())
```

```output
1
2
```

A `var` declared in a loop body is new on each pass. A module-level `var` is read-only inside a function or lambda. `spawn { }` copies what it captures ([Concurrency](concurrency.md)).

## Generators

A function that returns `Stream<T>` and uses `yield` is a generator. Calling it gives a lazy stream: the body runs only as far as the consumer asks.

```mote
fn countdown(start: Int) -> Stream<Int> {
    var n = start
    while n > 0 {
        yield n
        n -= 1
    }
}

for n in countdown(3) {
    println(n)
}
```

```output
3
2
1
```

`for`, `.next()` and the adapters in [`std.stream`](std/stream.md) consume a stream.
