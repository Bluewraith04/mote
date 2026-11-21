# Operators

## Precedence

* Loosest to tightest. All binary levels are left-associative except `??` (right), comparison and equality (non-chaining) and `..` (non-associative). <!-- test: arithmetic_follows_the_precedence_table, coalesce_is_the_loosest_and_right_associative, comparisons_and_ranges_do_not_chain, grammar_every_case_gives_the_result_it_records -->

<!-- test: arithmetic_follows_the_precedence_table, coalesce_is_the_loosest_and_right_associative, casts_sit_between_ranges_and_prefix_operators, prefix_binds_tighter_than_binary_and_looser_than_postfix, postfix_operators_chain_left_to_right, type_tests_and_ranges, grammar_every_case_gives_the_result_it_records -->
| Level | Operators |
|---|---|
| 1 | `??` |
| 2 | `\|\|` |
| 3 | `&&` |
| 4 | `\|` |
| 5 | `^` |
| 6 | `&` |
| 7 | `==` `!=` |
| 8 | `<` `<=` `>` `>=` |
| 9 | `is` |
| 10 | `<<` `>>` |
| 11 | `..` `..=` |
| 12 | `+` `-` |
| 13 | `*` `/` `%` |
| 14 | `as` |
| 15 | prefix `-` `!` `~` |
| 16 | postfix `.` `?.` `( )` `[ ]` `?` `!` |

* Bitwise operators bind looser than comparison, as in C: `a & 1 == 0` is `a & (1 == 0)`, so write `(a & 1) == 0`. <!-- block -->

```mote
fn main() {
    let n = 6
    let even = (n & 1) == 0
    println(even)
}
```

```output
true
```

```mote
let n = 6
let even = n & 1 == 0 // error
```

* `a < b < c` and `a == b == c` are errors. <!-- block -->

```mote
let c = 1 < 2 < 3 // error
```

```mote
let c = 1 == 1 == true // error
```

* `..` binds looser than `+`, so `0..n + 1` is `0..(n + 1)`. <!-- block -->

```mote
fn main() {
    let n = 1
    for i in 0..n + 1 {
        println(i)
    }
}
```

```output
0
1
```

* Assignment is a statement, so `a = b = c` does not parse. <!-- block -->

```mote
var a = 1
var b = 2
var c = 3
a = b = c // error
```

## Arithmetic

* `+ - * / %` need both operands of one type. `+` also joins two `String`s. <!-- block -->

```mote
fn main() {
    println(7 + 2)
    println(7 - 2)
    println(7 * 2)
    println(7 / 2)
    println(7 % 2)
    println(7.0 / 2.0)
    println("a" + "b")
}
```

```output
9
5
14
3
1
3.5
ab
```

```mote
let a = 1 + 1.5 // error
```

* `-` negates; `~` is bitwise not on integers. <!-- block -->

```mote
fn main() {
    let n = 5
    println(-n)
    println(~n)
    println(~5u8)
}
```

```output
-5
-6
250
```

Overflow and division by zero fault. See [numbers.md](numbers.md).

## Equality

* `==` and `!=` compare by value for scalars, strings, structs (field by field), tuples, arrays, lists (element by element at any depth), `Map`s and `Set`s (any order), optionals and enum variants (variant, then payload). <!-- block -->

```mote
import { Set, set_from } from std.collections

struct P {
    x: Int
    y: Int
}

enum Shape {
    Circle(Int)
    Square(Int)
    Empty
}

fn main() {
    println(1 == 1)
    println(1.5 != 1.5)
    println("a" == "a")
    println(P { x: 1, y: 2 } == P { x: 1, y: 2 })
    println((1, "a") == (1, "a"))
    let a: [Int] = [1, 2]
    let b: [Int] = [1, 2]
    println(a == b)
    println([[1], [2]] == [[1], [2]])
    println([1, 2] == [2, 1])
    println({"a": 1, "b": 2} == {"b": 2, "a": 1})
    println(set_from([1, 2]) == set_from([2, 1]))
    let o: Int? = 1
    println(o == 1)
    println(Shape.Circle(1) == Shape.Circle(1))
    println(Shape.Circle(1) == Shape.Circle(2))
    println(Shape.Circle(1) == Shape.Square(1))
    println(Shape.Empty == Shape.Empty)
}
```

```output
true
false
true
true
true
true
true
false
true
true
true
true
false
false
true
```

* Class instances compare by identity. <!-- block -->

```mote
class Box {
    var n: Int = 0
}

fn main() {
    let a = Box()
    let b = Box()
    let c = a
    println(a == b)
    println(a == c)
}
```

```output
false
true
```

* `==` is built in: an `eq` method, written or listed through `Eq`, never changes it. `T: Eq` allows `==` on `T`. <!-- block -->

```mote
import Eq from std.traits

struct P {
    x: Int

    fn eq(self, other: P) -> Bool { return true }
}

fn same<T: Eq>(a: T, b: T) -> Bool {
    return a == b
}

fn main() {
    let a = P { x: 1 }
    let b = P { x: 2 }
    println(a == b)
    println(same(a, b))
    println(a.eq(b))
}
```

```output
false
false
true
```

* A different type on each side is an error, except an optional `T?` against a `T` or `null`. <!-- block -->

```mote
let a = 1 == "a" // error
```

```mote
let a = 3 == null // error
```

```mote
fn main() {
    let o: Int? = 3
    println(o == 3)
    println(o == null)
    println(3 == o)
}
```

```output
true
false
true
```

* A name narrowed from an optional can still be compared with `null`. <!-- block -->

```mote
fn main() {
    let o: Int? = 3
    if let x = o {
        println(x == null)
    }
}
```

```output
false
```

## Ordering

* `< <= > >=` work on numbers and, byte by byte, on `String`s. <!-- block -->

```mote
fn main() {
    println(1 < 2)
    println(2.5 >= 2.5)
    println("ab" < "b")
    println("Z" < "a")
    println("é" > "z")
}
```

```output
true
true
true
true
true
```

## Logic and bits

<!-- block -->
| Operator | On | Meaning |
|---|---|---|
| `&&` `\|\|` | `Bool` | and, or; short-circuit |
| `!` | `Bool` | not |
| `&` `\|` `^` | integers | bitwise and, or, xor |
| `~` | integers | bitwise not |
| `<<` `>>` | integers | shifts; `>>` on signed types is arithmetic |

```mote
fn loud(b: Bool) -> Bool {
    println("loud")
    return b
}

fn main() {
    println(false && loud(true))
    println(true || loud(true))
    println(true && loud(false))
    println(!true)
    println(6 & 3)
    println(6 | 3)
    println(6 ^ 3)
    println(~0)
    println(1 << 4)
    println(-8 >> 1)
    let u: UInt8 = 128
    println(u >> 1)
}
```

```output
false
true
loud
false
false
2
7
5
-1
16
-4
64
```

## `is`

* `e is T` is `true` when the run-time type of `e` is `T`, compared whole. A `T` that `e` can never be is an error. <!-- block -->

```mote
fn main() {
    let x: Int | List<Int> = [1]
    println(x is List<Int>)
    println(x is Int)
}
```

```output
true
false
```

```mote
let x: Int | List<Int> = [1]
let b = x is List<String> // error
```

* `T` may be a union (in parentheses), `T?` or a type parameter. A variant is not a type: `s is Shape.Circle` is an error; use `match`. <!-- block -->

```mote
fn kind(x: Int | String | Null) -> Bool {
    return x is (Int | String)
}

fn is_t<T>(x: T | String) -> Bool {
    return x is T
}

fn main() {
    let o: Int? = 4
    println(kind(null))
    println(kind(1))
    println(o is Int?)
    println(is_t<Int>(1))
    println(is_t<Int>("a"))
}
```

```output
false
true
true
true
false
```

```mote
enum Shape {
    Circle(Int)
    Empty
}

let s = Shape.Empty
let b = s is Shape.Circle // error
```

* On a `let` name, `is` narrows `x` to `T` in the branch it guards. See [optionals-errors.md](optionals-errors.md). <!-- block -->

```mote
fn main() {
    let x: Int | String = 4
    if x is Int {
        println(x + 1)
    }
}
```

```output
5
```

## `as`

* `e as T` is a scalar cast. See [numbers.md](numbers.md). <!-- block -->

```mote
fn main() {
    let n = 300
    println(n as UInt8)
}
```

```output
44
```

## `?` propagate

* `e?` on an optional or `Result` returns the failure from the function; otherwise it yields the payload. <!-- block -->

```mote
import parse_int from std.string

fn first_length(xs: List<String>) -> Int? {
    let s = xs.first()?
    return s.len()
}

fn double(text: String) -> Result<Int, Error> {
    let n = parse_int(text)?
    return Ok(n * 2)
}

fn main() {
    println(first_length(["abc"]))
    println(first_length([]) == null)
    println(double("21"))
    println(double("x").is_err())
}
```

```output
3
true
Ok(42)
true
```

* `?` on an optional needs a function returning an optional. `?` on a `Result` needs a function returning a `Result` whose error type the error fits: the same type or a union holding it. Convert an optional with `.ok_or(e)?`, and any other error with a `match`. <!-- block -->

```mote
fn bad(xs: List<String>) -> Int {
    let s = xs.first()? // error
    return s.len()
}
```

```mote
fn good(xs: List<String>) -> Result<Int, String> {
    let s = xs.first().ok_or("empty")?
    return Ok(s.len())
}

fn main() {
    println(good(["ab"]))
    println(good([]))
}
```

```output
Ok(2)
Err("empty")
```

```mote
import parse_int from std.string

fn bad(text: String) -> Result<Int, String> {
    let n = parse_int(text)? // error
    return Ok(n)
}
```

## `!` unwrap

* `e!` yields the payload of an optional or `Result`, or faults. <!-- block -->

```mote
fn main() {
    let xs = [4]
    println(xs.first()!)
}
```

```output
4
```

```mote
let xs: List<Int> = []
let n = xs.first()! // faults UnwrapFailed
```

## `??` coalesce

* `a ?? b` is the payload of `a`, else `b`. `b` runs only when needed. The result is `T` when `b` is `T`, and `T?` when `b` is optional. <!-- block -->

```mote
fn fallback() -> Int {
    println("fallback")
    return 8080
}

fn main() {
    let a: Int? = 1
    let b: Int? = null
    println(a ?? fallback())
    println(b ?? fallback())
    let c: Int? = null
    println(typeof(b ?? 1))
    println(typeof(b ?? c))
}
```

```output
1
fallback
8080
Int
Int?
```

* `??` is right-associative and binds loosest: `a ?? 0 + 1` is `a ?? (0 + 1)`. <!-- block -->

```mote
fn main() {
    let a: Int? = 5
    let b: Int? = null
    println(a ?? 0 + 1)
    println(b ?? a ?? 7)
}
```

```output
5
5
```

* `a` must be optional. <!-- block -->

```mote
let a = 1
let b = a ?? 2 // error
```

## `?.` optional member

* `a?.f` and `a?.m(args)` are `None` if `a` is `None`, else the member as an optional. Arguments run only when `a` is present. A member already optional is not wrapped twice. <!-- block -->

```mote
class Node {
    value: Int
    next: Node?

    fn plus(self, n: Int) -> Int {
        return self.value + n
    }
}

fn arg() -> Int {
    println("arg")
    return 1
}

fn main() {
    let a: Node? = Node { value: 1, next: Node { value: 2, next: null } }
    let b: Node? = null
    println(a?.value)
    println(b?.value == null)
    println(a?.next?.value)
    let n: Node? = a?.next
    println(n?.value)
    println(b?.plus(arg()) == null)
    println(a?.plus(arg()))
}
```

```output
1
true
2
2
true
arg
2
```

* A `.` after `a?.b` applies to the optional `a?.b`, not to its payload, so `a?.b.c` is an error. <!-- block -->

```mote
class Node {
    value: Int
}

let a: Node? = Node { value: 1 }
let n = a?.value.to_string() // error: Int? has no method
```

## Indexing

* `xs[i]` reads a `List`, array or `Map`. `xs[i] = v` and `xs[i] op= v` write; `xs` and `i` are evaluated once. <!-- block -->

```mote
fn main() {
    var xs = [1, 2]
    var m = {"k": 1}
    var a: [Int] = [5, 6]
    xs[0] = 9
    m["k"] += 1
    a[1] -= 1
    println(xs[0])
    println(m["k"])
    println(a[1])
}
```

```output
9
2
5
```

* An index out of range or a missing key faults. There is no negative index. A `String` cannot be indexed. <!-- block -->

```mote
let xs = [1, 2]
let a = xs[2] // faults IndexOutOfRange
```

```mote
let xs = [1, 2]
let n = -1
let a = xs[n] // faults IndexOutOfRange
```

```mote
let m = {"k": 1}
let a = m["x"] // faults Raised
```

```mote
let s = "abc"
let c = s[0] // error
```

## Slicing

* A range inside `[ ]` makes a zero-copy view on a `List` or `[T]` (`Span<T>`), or on a `String` (`StrView`); only `[T; N]` makes a `[T]` copy. <!-- block -->

```mote
fn main() {
    var xs = [1, 2, 3]
    let s = xs[0..2]
    xs[0] = 9
    println(s)
    println("hello"[1..3])
    let a: [Int; 3] = [1, 2, 3]
    println(typeof(a[0..2]))
    let b: [Int] = [1, 2, 3]
    println(typeof(b[0..2]))
    println(typeof("hello"[1..3]))
}
```

```output
[9, 2]
el
[Int]
Span<Int>
StrView
```

<!-- block -->
| Written | Meaning |
|---|---|
| `xs[a..b]` | from `a` up to, not including, `b` |
| `xs[a..=b]` | through `b` |
| `xs[..b]` `xs[..=b]` | from 0 |
| `xs[a..]` | to the end |
| `xs[..]` | all of it |
| `xs[a..b..s]` | every `s`th item (`List` and arrays; `b` required, `s > 0`) |
| `xs[a..b] = v` | `List` only: one value for every slot, or a list of length `b - a` |

```mote
fn main() {
    var xs = [0, 1, 2, 3, 4, 5]
    println(xs[1..3])
    println(xs[1..=3])
    println(xs[..2])
    println(xs[..=2])
    println(xs[4..])
    println(xs[..])
    println(xs[0..6..2])
    xs[1..3] = [7, 8]
    println(xs)
    xs[3..5] = 0
    println(xs)
}
```

```output
[1, 2]
[1, 2, 3]
[0, 1]
[0, 1, 2]
[4, 5]
[0, 1, 2, 3, 4, 5]
[0, 2, 4]
[0, 7, 8, 3, 4, 5]
[0, 7, 8, 0, 0, 5]
```

* A bound out of range, or `b` before `a`, faults. `xs[a..b][i] = v` and `xs[a..b] += v` are errors. <!-- block -->

```mote
let xs = [1, 2, 3]
let a = xs[1..9] // faults IndexOutOfRange
```

```mote
let xs = [1, 2, 3]
let a = 2
let b = 1
let s = xs[a..b] // faults IndexOutOfRange
```

```mote
var xs = [1, 2, 3]
xs[0..2][0] = 5 // error
```

```mote
var xs = [1, 2, 3]
xs[0..2] += 5 // error
```

## Ranges

* `a..b` excludes `b`; `a..=b` includes it. Both ends are required outside `[ ]`. A range is a value used by `for`, `match` and indexing. <!-- block -->

```mote
fn main() {
    for i in 0..3 {
        println(i)
    }
    for i in 0..=1 {
        println(i)
    }
    match 5 {
        0..=4 => println("low")
        _ => println("high")
    }
}
```

```output
0
1
2
0
1
high
```

```mote
let r = 1.. // error
```

## Prefix and postfix summary

<!-- block -->
| Form | Meaning |
|---|---|
| `-e` `!e` `~e` | negate, not, bitwise not |
| `e.name` | field or method |
| `e.0` | tuple element |
| `e(args)` | call |
| `e[i]` | index |
| `e?` `e!` | propagate, unwrap |
| `e?.name` | optional member |

```mote
struct P {
    x: Int
}

fn first(xs: List<Int>) -> Int? {
    let a = xs.first()?
    return a
}

fn main() {
    let t = (4, 5)
    let o: P? = P { x: 6 }
    println(-t.0)
    println(!(t.1 == 5))
    println(~t.0)
    println(first([7])!)
    println([8, 9][1])
    println(o?.x)
}
```

```output
-4
false
-5
7
9
6
```
