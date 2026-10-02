# Structs, classes and enums

## Structs and classes

Both group named fields. A struct is copied on assignment; a class is shared.

| | `struct` | `class` |
|---|---|---|
| Assignment, passing, returning | copies the value | shares the object |
| `==` | field by field | true only for the same object |
| Fields it may hold | values that cannot change under it | anything |
| Use for | small plain data | identity or changing state |

Each field goes on its own line as `name: Type`. A literal gives every field exactly once; there are no field defaults. A field can be assigned only when declared `var`.

```mote
struct Point {
    var x: Int
    var y: Int
}

var a = Point { x: 1, y: 2 }
var b = a
b.x = 99
println(a.x)
println(b.x)
println(a == Point { x: 1, y: 2 })

class Counter {
    var count: Int
}

let c = Counter { count: 0 }
let d = c
d.count += 1
println(c.count)
```

```output
1
99
true
1
```

`let` on a struct is fixed all the way down: `let p = Point { … }` then `p.x = 5` is an error. On a class `let` fixes only the name.

A struct holds only `Int`, `Float`, `Bool`, `Char`, `String`, other structs, tuples, optionals of these, `Task`, `Sender`, `Receiver`, `Shared`, and classes with no `var` fields. A `List`, `Map`, `Set`, `Bytes` or a class with `var` fields is a compile error; make the type a class to hold one.

### Methods

A method is a `fn` in the type body. The first parameter says how it treats the object.

| Receiver | Meaning |
|---|---|
| `self` | reads the object |
| `var self` | may change the caller's object |
| none | a static method, called `Type.name(…)` |

```mote
struct Duration {
    var nanos: Int

    pub fn from_millis(n: Int) -> Duration {
        return Duration { nanos: n * 1000000 }
    }

    pub fn as_millis(self) -> Int {
        return self.nanos / 1000000
    }

    pub fn plus(self, other: Duration) -> Duration {
        return Duration { nanos: self.nanos + other.nanos }
    }
}

class Counter {
    var n: Int

    pub fn bump(var self) {
        self.n += 1
    }
}

let total = Duration.from_millis(1500).plus(Duration.from_millis(2000))
println(total.as_millis())
var c = Counter { n: 0 }
c.bump()
c.bump()
println(c.n)
```

```output
3500
2
```

`Point(1, 2)` calls the static method `Point.new(1, 2)`; a type with no `new` cannot be called this way. A method call is resolved from the static type of the receiver, so it does not resolve on an `Any`. There is no inheritance; the colon on a declaration lists traits ([Generics and traits](generics.md#traits)).

## Enums

An enum is exactly one of a fixed set of named variants, each optionally carrying values.

| Variant | Declared | Built |
|---|---|---|
| unit | `North` | `Dir.North` |
| tuple payload | `Circle(Int)` | `Shape.Circle(5)` |
| named fields | `Rect { w: Int, h: Int }` | `Shape.Rect { w: 2, h: 3 }` |

`match` names the variant and its payload. A named-field pattern lists every field or ends with `..`.

```mote
enum Shape {
    Circle(Int)
    Rect { w: Int, h: Int }
    Dot
}

fn area(s: Shape) -> Int {
    match s {
        Circle(r) => { return r * r * 3 }
        Rect { w, h } => { return w * h }
        Dot => { return 0 }
    }
}

println(area(Shape.Circle(2)))
println(area(Shape.Rect { w: 4, h: 5 }))
println(area(Shape.Dot))
```

```output
12
20
0
```

Two enums may share a variant name; write `Shape.Circle(5)` when the context does not decide. An enum body lists variants first, then methods:

```mote
enum Light {
    Red
    Green

    fn next(self) -> Light {
        match self {
            Light.Red => return Light.Green
            Light.Green => return Light.Red
        }
    }

    fn start() -> Light { return Light.Red }
}

let l = Light.start()
println(l.next())
println(l.next().next())
```

```output
Green
Red
```

A method cannot share a variant's name, and `var self` is refused: return a new value instead.

### Enum or union

| | Enum | Union (`A \| B`) |
|---|---|---|
| Cases are | variants declared with the type | existing types |
| A case with no value | yes | no |
| Two cases with one payload type | yes | no |
| May refer to itself | yes | not through an alias |
| Taken apart with | variant patterns | type patterns |
| Cost | a tag and a payload | none |

Use a union when the cases are types you already have. Use an enum when a case carries nothing, needs a name, or the type contains itself.
