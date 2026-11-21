# Control flow

## `if`

* `if cond { … } else if cond { … } else { … }`. The condition is a `Bool`. Braces are required. <!-- block -->

```mote
fn describe(n: Int) {
    if n > 0 {
        println("positive")
    } else if n < 0 {
        println("negative")
    } else {
        println("zero")
    }
}

fn main() {
    describe(1)
    describe(-1)
    describe(0)
}
```

```output
positive
negative
zero
```

```mote
if 1 { // error
    println("one")
}
```

```mote
if true println("one") // error
```

## Value `if`

* An `if` in expression position holds one expression per branch and needs `else`. <!-- block -->

```mote
fn main() {
    let n = -4
    let sign = if n < 0 { -1 } else { 1 }
    println(sign)
}
```

```output
-1
```

```mote
let n = 1
let sign = if n < 0 { -1 } // error
```

```mote
let n = 1
let sign = if n < 0 { println("x") 1 } else { 2 } // error
```

* Position decides the form: a statement `if` runs statements, a value `if` yields one value. Branches fit a common type; a `null` branch makes it optional. <!-- block -->

```mote
fn main() {
    let n = 5
    let label = if n > 0 { "pos" } else if n < 0 { "neg" } else { "zero" }
    let maybe = if n > 0 { n } else { null }
    println(label)
    println(typeof(maybe))
    println(maybe)
}
```

```output
pos
Int?
5
```

```mote
let n = 1
let x = if n > 0 { 1 } else { "a" } // error
```

## `if let` and `while let`

* The condition may match a pattern. The names it binds live in the body. <!-- block -->

```mote
fn main() {
    let xs = [4, 5]
    if let Some(x) = xs.first() {
        println(x)
    }
    var queue = [1, 2, 3]
    while let Some(job) = queue.pop() {
        println(job)
    }
}
```

```output
4
3
2
1
```

```mote
fn main() {
    let o: Int? = 1
    if let Some(x) = o {
        println(x)
    }
    println(x) // error
}
```

* The value `if` takes `if let` too. <!-- block -->

```mote
fn main() {
    let o: Int? = 7
    let n = if let Some(x) = o { x } else { 0 }
    println(n)
}
```

```output
7
```

## `while`

* `while cond { … }` repeats while the condition holds. `break` leaves, `continue` skips to the next iteration. <!-- block -->

```mote
fn main() {
    var i = 0
    var total = 0
    while i < 10 {
        i += 1
        if i % 2 == 0 {
            continue
        }
        if i > 7 {
            break
        }
        total += i
    }
    println(i)
    println(total)
}
```

```output
9
16
```

## `for`

* `for pattern in iterable { … }`. The names are read-only. The pattern must always match. <!-- block -->

```mote
struct Point {
    x: Int
    y: Int
}

fn main() {
    for (a, b) in [(1, 2), (3, 4)] {
        println(a + b)
    }
    for Point { x, y } in [Point { x: 5, y: 6 }] {
        println(x + y)
    }
}
```

```output
3
7
11
```

```mote
for i in 0..3 {
    i = 5 // error
}
```

```mote
fn main() {
    let xs: List<Int?> = [1, null]
    for Some(x) in xs { // error
        println(x)
    }
}
```

<!-- block -->
| Iterable | Yields |
|---|---|
| `0..3` | 0, 1, 2 |
| `0..=3` | 0, 1, 2, 3 |
| a `List` or array | each element |
| a call returning a `List` | `m.keys()`, `m.values()`, `m.items()` |
| a `Receiver` | each message until the channel closes |
| a `Stream` | each yielded value |

```mote
import { Stream } from std.collections
import { Channel } from std.sync

fn count(n: Int) -> Stream<Int> {
    for i in 0..n {
        yield i * 10
    }
}

fn main() {
    for i in 0..3 {
        println(i)
    }
    for i in 0..=3 {
        println(i)
    }
    for x in [7, 8] {
        println(x)
    }
    let a: [Int; 2] = [9, 10]
    for x in a {
        println(x)
    }
    let m = {"k": 1}
    for k in m.keys() {
        println(k)
    }
    for v in m.values() {
        println(v)
    }
    for (k, v) in m.items() {
        println("${k}=${v}")
    }
    let (tx, rx) = Channel<Int>(2)
    tx.send(1)
    tx.send(2)
    tx.close()
    for v in rx {
        println(v)
    }
    for v in count(3) {
        println(v)
    }
}
```

```output
0
1
2
0
1
2
3
7
8
9
10
k
1
k=1
1
2
0
10
20
```

* A `Map`, `Set`, struct or class is not iterable directly. <!-- block -->

```mote
let m = {"k": 1}
for x in m { // error
    println(x)
}
```

```mote
import { Set, set_from } from std.collections

let s = set_from([1])
for x in s { // error
    println(x)
}
```

```mote
struct P {
    x: Int
}

for x in P { x: 1 } { // error
    println(x)
}
```

```mote
class C {
    var n: Int = 0
}

for x in C() { // error
    println(x)
}
```

## `match`

* `match subject { pattern => stmt … }` tries arms in order. An arm is one statement or a block. `pattern if guard` applies only when the guard holds. <!-- block -->

```mote
enum Shape {
    Circle(Int)
    Rect(Int, Int)
    Dot
}

fn show(shape: Shape) {
    match shape {
        Circle(r) => println(r)
        Rect(w, h) if w == h => println("square")
        Rect(w, h) => { println(w * h) }
        _ => println("other")
    }
}

fn main() {
    show(Shape.Circle(3))
    show(Shape.Rect(2, 2))
    show(Shape.Rect(2, 5))
    show(Shape.Dot)
}
```

```output
3
square
10
other
```

* Arms need no separator. Names bound by a pattern live in that arm. <!-- block -->

```mote
fn main() {
    let o: Int? = 4
    match o {
        Some(x) => println(x)
        None => println("none")
    }
}
```

```output
4
```

```mote
fn main() {
    let o: Int? = 4
    match o {
        Some(x) => println(x)
        None => println(x) // error
    }
}
```

<!-- block -->
| Subject | Must be covered by |
|---|---|
| an enum | every variant, or a catch-all |
| a `Bool` | `true` and `false` |
| a union | every member (type patterns), or a catch-all |
| an optional | `Some` and `None`, or a catch-all |
| anything else | a catch-all arm: `_` or a bare name |

```mote
enum Level {
    Low
    High
}

fn main() {
    let level = Level.High
    match level {
        Low => println("low")
        High => println("high")
    }
    match true {
        true => println("yes")
        false => println("no")
    }
    let u: Int | String = "a"
    match u {
        _: Int => println("int")
        _: String => println("string")
    }
    let o: Int? = null
    match o {
        Some(x) => println(x)
        None => println("none")
    }
    match 5 {
        1 => println("one")
        n => println(n)
    }
    match "a" {
        "b" => println("b")
        _ => println("other")
    }
}
```

```output
high
yes
string
none
5
other
```

```mote
enum Level {
    Low
    High
}

let level = Level.High
match level { // error
    Low => println("low")
}
```

```mote
match true { // error
    true => println("yes")
}
```

```mote
let u: Int | String = "a"
match u { // error
    _: Int => println("int")
}
```

```mote
let o: Int? = null
match o { // error
    Some(x) => println(x)
}
```

```mote
match 5 { // error
    1 => println("one")
}
```

## Value `match`

* A `match` in expression position has one expression per arm and must cover its subject. <!-- block -->

```mote
enum Shape {
    Circle(Float)
    Rect(Float, Float)
}

fn main() {
    let shape = Shape.Rect(2.0, 3.0)
    let area = match shape {
        Circle(r) => 3.0 * r * r
        Rect(w, h) => w * h
    }
    println(area)
}
```

```output
6.0
```

```mote
let n = 1
let x = match n { // error
    1 => 10
}
```

* Arms fit a common type. <!-- block -->

```mote
let n = 1
let x = match n { // error
    1 => 10
    _ => "ten"
}
```

## `return`

* `return e` leaves the function with `e`; a bare `return` returns `null` and fits only a function returning `Null`. A function body has no implicit last-line value. <!-- block -->

```mote
fn double(n: Int) -> Int {
    return n * 2
}

fn nothing() {
    return
}

fn main() {
    println(double(4))
    println(nothing() == null)
}
```

```output
8
true
```

```mote
fn double(n: Int) -> Int {
    n * 2 // error
}
```

```mote
fn find(n: Int) -> Int? {
    return // error
}
```

## `break` and `continue`

* `break` leaves the innermost loop and takes no value; `continue` starts its next iteration. <!-- block -->

```mote
fn main() {
    for x in [1, -2, 3, 200, 4] {
        if x < 0 {
            continue
        }
        if x > 100 {
            break
        }
        println(x)
    }
    for i in 0..2 {
        for j in 0..5 {
            if j == 1 {
                break
            }
            println("${i}${j}")
        }
    }
}
```

```output
1
3
00
10
```

```mote
fn main() {
    while true {
        break 1 // error
    }
}
```

## `yield`

* `yield e` produces the next element of a function declared `-> Stream<T>`. See [functions.md](functions.md). <!-- block -->

```mote
import { Stream } from std.collections

fn count(n: Int) -> Stream<Int> {
    for i in 0..n {
        yield i
    }
}

fn main() {
    for x in count(3) {
        println(x)
    }
}
```

```output
0
1
2
```

```mote
fn count(n: Int) -> Int {
    yield n // error
}
```

## `with`

* `with name = expr { … }` binds one resource and calls `.close()` once on every exit: falling off the end, `return`, `break`, `continue` and `?`. <!-- block -->

```mote
class Res {
    let label: String

    fn close(self) {
        println("close ${self.label}")
    }
}

fn early() {
    with r = Res { label: "return" } {
        println("body")
        return
    }
}

fn fail() -> Int? {
    with r = Res { label: "question" } {
        let n: Int? = null
        let m = n?
        println("not reached")
    }
    return 1
}

fn main() {
    with r = Res { label: "end" } {
        println("body")
    }
    early()
    for i in 0..2 {
        with r = Res { label: "loop ${i}" } {
            if i == 0 {
                continue
            }
            break
        }
    }
    println(fail() == null)
}
```

```output
body
close end
body
close return
close loop 0
close loop 1
close question
true
```

* The resource is any value with a visible `close(self)` method that takes no other parameter; `var self` does not qualify. The name is immutable and scoped to the block. One resource per `with`; nest for two. The `close()` result is discarded. <!-- block -->

```mote
class Res {
    let label: String

    fn close(self) -> Int {
        println("close ${self.label}")
        return 1
    }
}

fn main() {
    with a = Res { label: "a" } {
        with b = Res { label: "b" } {
            println("both")
        }
    }
}
```

```output
both
close b
close a
```

```mote
class Res {
    var n: Int = 0

    fn close(var self) {
        self.n = 1
    }
}

with r = Res() { // error
    println("body")
}
```

```mote
with r = 5 { // error
    println("body")
}
```

```mote
class Res {
    fn close(self) {}
}

with r = Res() {
    r = Res() // error
}
```

```mote
class Res {
    fn close(self) {}
}

with r = Res() {
    println("body")
}
println(r) // error
```

```mote
class Res {
    fn close(self) {}
}

with a = Res(), b = Res() { // error
    println("body")
}
```

## `scope` and `spawn`

* `scope { … }` ends only when every task spawned inside it has finished. See [concurrency.md](concurrency.md). <!-- block -->

```mote
import { Mutex } from std.sync

fn main() {
    let hits = Mutex(0)
    scope {
        spawn { hits.lock(|n| { n += 1 }) }
        spawn { hits.lock(|n| { n += 10 }) }
    }
    println(hits.lock(|n| { return n }))
}
```

```output
11
```

## Nested functions

* A `fn` may appear as a statement. It sees names declared before it. <!-- block -->

```mote
fn outer() -> Int {
    let base = 10
    fn inner(n: Int) -> Int {
        return n + base
    }
    return inner(1)
}

fn main() {
    println(outer())
}
```

```output
11
```

```mote
fn outer() -> Int {
    fn inner(n: Int) -> Int {
        return n + later // error
    }
    let later = 10
    return inner(1)
}
```

## Blocks and statement ends

* Blocks are `{ … }` and hold statements; a block is a statement only as the body of another statement. <!-- block -->

```mote
fn main() {
    {  // error
        println(1)
    }
}
```

* A line ends a statement by the rules in [lexical.md](lexical.md), except that a `return` takes the next expression even from the next line: a bare `return` is the last statement of its block. <!-- block -->

```mote
fn f(n: Int) -> Int {
    if n > 0 {
        return n
    }
    return 0
}

fn g(n: Int) {
    if n > 0 {
        return
    }
    println("not positive")
}

fn main() {
    println(f(2))
    g(1)
    g(0)
    for i in 0..3 {
        if i == 1 {
            break
        }
        println(i)
    }
}
```

```output
2
not positive
0
```

```mote
fn h() -> Int {
    return
        7
}

fn main() {
    println(h())
}
```

```output
7
```
