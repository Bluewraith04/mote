# Basics

## Source text

A newline ends a statement; there are no semicolons. A statement continues onto the next line when the line ends with an operator, or inside `( )`, `[ ]` and `{ }`. Comments are `// …` and nestable `/* … */`.

## Variables

`let` makes a name that cannot be assigned again. `var` makes one that can. A declaration always has a value; a type annotation goes after the name and is needed only when the value does not decide the type.

```mote
fn main() {
    let name = "Ada"
    var count = 0
    count += 1
    println("${name} ${count}")

    let (low, high) = (1, 9)
    println(high - low)
}
```

```output
Ada 1
8
```

Destructuring works only inside a function.

Assignment is a statement, so `a = b = c` is an error. Compound assignment (`+=`, `-=`, `*=`, `/=`, `%=` and the bit forms) works on a name, a field or an index.

## Types

| Type | Holds | Literals |
|---|---|---|
| `Int` | 64-bit signed integer, wraps on overflow | `42`, `1_000_000`, `0xFF`, `0b1010`, `0o17` |
| `Float` | 64-bit float | `1.5`, `2.5e-3` |
| `String` | UTF-8 text | `"hello"` |
| `Char` | one Unicode character | `'a'` |
| `Bool` | true or false | `true`, `false` |
| `Null` | no value | `null` |

`Int` and `Float` do not mix in one operator: `1 + 1.5` is an error; use `Float.from(1)`. There is no cast; `is` tests a type. Strings take the escapes `\n \t \r \0 \\ \" \' \u{1F600}` and have no raw form.

A written type:

| Written | Means |
|---|---|
| `Point` | a struct, class or enum |
| `List<Int>` | a generic type with arguments |
| `(Int, Bool)` | a tuple |
| `(Int) -> Bool` | a function type |
| `Int?` | an `Int` or nothing; the same as `Option<Int>` |
| `Int \| String` | a union |

`typeof(x)` gives a value's type as a `String`: `typeof([1])` is `"List<Int>"`.

`Any` accepts every value, with a check at run time. It is not in scope until imported, and a union or generic is almost always better:

```mote,check
import { Any } from std.experimental.types

let anything: Any = 3
```

### Interpolation and printing

`${expression}` inside a string is replaced by the value's text; `\${` writes a literal one. `print(x)` and `println(x)` accept any value. `x.to_string()` gives the same text.

```mote
let items = 3
println("${items} items, ${items * 2} halves")
println([1, 2, 3])
println({"a": 1})
```

```output
3 items, 6 halves
[1, 2, 3]
{"a": 1}
```

A struct prints as `Point { x: 1, y: 2 }`, an enum variant as `Circle(2)`, and strings inside a collection are quoted.

## Operators

| Group | Operators |
|---|---|
| Arithmetic | `+ - * / %` |
| Comparison | `== != < <= > >=` |
| Logic | `&& \|\| !` |
| Bits | `& \| ^ ~ << >>` |
| Ranges | `a..b` excludes `b`, `a..=b` includes it; only in `for` and `match` patterns, not as a value |
| Choice | `c ? x : y` |
| Optionals | `a ?? b`, `a ??= b`, `a?.field` |
| Type test | `x is T` |

[Reference](reference.md#operators) has the precedence table.

## Control flow

`if`, `while`, `for` and `match` are statements and produce no value; use `c ? 1 : 2` to choose between two values. Conditions must be `Bool` and braces are required.

```mote
let temperature = 31
if temperature > 30 {
    println("hot")
} else if temperature > 15 {
    println("mild")
} else {
    println("cold")
}

var n = 1
while n < 100 {
    n = n * 3
}
println(n)

var total = 0
for price in [3, 5, 8] {
    total += price
}
println(total)
```

```output
hot
243
16
```

There is no `loop`; write `while true`. `for` runs over these:

| Written | Items |
|---|---|
| `for i in 0..3` | 0, 1, 2 |
| `for i in 0..=3` | 0, 1, 2, 3 |
| `for x in items` | each element of a list |
| `for k in map.keys()` | each key |
| `for m in rx` | each message of a `Receiver` until its channel closes |
| `for v in numbers()` | each value a [generator](functions.md#generators) yields |

The loop variable is read-only. `break`, `continue` and `return` leave the innermost loop, start its next pass, and leave the function.

## `match`

`match` runs the first arm whose pattern fits.

```mote
fn size(n: Int) -> String {
    match n {
        0 => { return "none" }
        1 | 2 | 3 => { return "few" }
        4..10 => { return "some" }
        x if x < 0 => { return "negative" }
        _ => { return "many" }
    }
}

println(size(0))
println(size(2))
println(size(7))
println(size(-4))
println(size(50))
```

```output
none
few
some
negative
many
```

| Pattern | Matches |
|---|---|
| `_` | anything |
| `x` | anything, named `x` in the arm |
| `42`, `"hi"`, `true`, `null` | that literal |
| `1 \| 2` | either |
| `4..10`, `4..=10` | an integer in the range |
| `x if x > 100` | the pattern, when the guard holds |
| `x @ 50` | the pattern, naming the value |
| `Circle(r)` | an enum variant and its payload |
| `(a, b)` | a tuple |
| `n: Int` | a value of that type |

A `match` on an enum must name every variant or end with `_`; on a `Bool` it needs both values; otherwise it needs a `_` arm. Struct-field patterns such as `Point { x, .. }` are not supported.

### Matching types

`name: T` matches a value of type `T` and gives it that type inside the arm. This takes apart a union, or an `Any`.

```mote
fn describe(x: Int | String | Bool) -> String {
    match x {
        n: Int if n < 0 => { return "negative" }
        n: Int => { return "number ${n + 1}" }
        s: String => { return "text of length ${s.len()}" }
        _: Bool => { return "flag" }
    }
}

println(describe(-1))
println(describe(4))
println(describe("hello"))
println(describe(true))
```

```output
negative
number 5
text of length 5
flag
```

On a union the arms must cover every member (a guarded arm covers nothing) or end with `_`. `if x is T { … }` is the one-arm form: inside the braces `x` has type `T`, and in `else` it has the remaining types.

```mote
fn double(x: Int | String) -> String {
    if x is Int {
        return "${x * 2}"
    } else {
        return x + x
    }
}

println(double(21))
println(double("ab"))
```

```output
42
abab
```
