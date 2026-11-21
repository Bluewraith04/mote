# Collections and strings

## Kinds

<!-- block -->
| Type | Holds | Literal |
|---|---|---|
| `List<T>` | growable sequence | `[1, 2, 3]` |
| `[T]` | array, length fixed when made | `[1, 2, 3]` with annotation, `[0; n]` |
| `[T; N]` | array, length in the type | `[0; 4]` |
| `Map<K, V>` | values by key, in insertion order | `{"a": 1}`, `Map<K, V>()` |
| `Set<T>` | distinct members | `Set<T>()`, `set_from(xs)` |
| `String` | immutable UTF-8 | `"text"` |

```mote
import { Set, set_from } from std.collections

fn main() {
    let list = [1, 2, 3]
    let array: [Int] = [1, 2, 3]
    let filled = [0; 4]
    let sized: [Int; 4] = [0; 4]
    let map = {"b": 2, "a": 1}
    let made = Map<String, Int>()
    let empty = Set<Int>()
    let set = set_from([1, 1, 2])
    println(typeof(list))
    println(typeof(array))
    println(typeof(sized))
    println(typeof(map))
    println(map.keys())
    println(made.len())
    println(empty.len())
    println(set.len())
    println(filled.len())
    println("text".len())
}
```

```output
List<Int>
[Int]
[Int; 4]
Map<String, Int>
["b", "a"]
0
0
2
4
4
```

* An empty literal needs its type from context. <!-- block -->

```mote
fn main() {
    let xs: List<Int> = []
    let m: Map<String, Int> = {}
    println(xs.len() + m.len())
}
```

```output
0
```

```mote
let xs = [] // error
```

* The elements of a literal fit one type, and a `null` makes it optional; a union is written, as in `List<Int | String>`. <!-- block -->

```mote
fn main() {
    let a = [1, 2, null]
    let b: List<Int | String> = [1, "a"]
    println(typeof(a))
    println(typeof(b))
}
```

```output
List<Int?>
List<String | Int>
```

```mote
let c = [1, "a"] // error
```

* `Set`, `Stream` and `Task` need imports: `import { Set } from std.collections`. <!-- block -->

```mote
import { Set, Stream } from std.collections
import { Task } from std.task

fn count() -> Stream<Int> {
    yield 1
}

fn main() {
    let s = Set<Int>()
    let t = spawn { return 1 }
    let u: Task<Int> = t
    println(s.len())
    println(u.await())
    println(count().next())
}
```

```output
0
Ok(1)
1
```

```mote
let s = Set<Int>() // error
```

```mote
fn count() -> Stream<Int> { // error
    yield 1
}
```

```mote
fn run(t: Task<Int>) {} // error
```

## List

<!-- block -->
| Method | Result |
|---|---|
| `len()` `is_empty()` | size |
| `push(v)` | appends |
| `pop()` `first()` `last()` | `T?` |
| `get(i)` | the element; faults out of range |
| `get_or(i, d)` | the element, else `d` |
| `set(i, v)` | writes |
| `contains(v)` | `Bool`, by `==` |
| `slice(a, b)` `set_slice(a, b, v)` | see [operators.md](operators.md) |
| `clear()` | empties |
| `to_array()` | a `[T]` of the elements; a copy |
| `stream()` | a lazy form |

```mote
fn main() {
    var xs: List<Int> = [3, 1, 2]
    xs.push(4)
    println(xs.len())
    println(xs.is_empty())
    println(xs.first())
    println(xs.last())
    println(xs.pop())
    println(xs.get(1))
    println(xs.get_or(9, -1))
    xs.set(0, 7)
    println(xs)
    println(xs.contains(2))
    println(xs.slice(1, 3))
    xs.set_slice(0, 2, [8, 9])
    println(xs)
    println(xs.stream().next())
    println(xs.to_array())
    xs.clear()
    println(xs.is_empty())
    println(xs.pop() == null)
}
```

```output
4
false
3
4
4
1
-1
[7, 1, 2]
true
[1, 2]
[8, 9, 2]
8
[8, 9, 2]
true
true
```

```mote
let xs = [1]
let n = xs.get(3) // faults IndexOutOfRange
```

* `xs[i]` is `xs.get(i)`; `xs[i] = v` is `xs.set(i, v)`. <!-- block -->

```mote
fn main() {
    var xs = [5, 6]
    println(xs[1] == xs.get(1))
    xs[0] = 9
    xs.set(1, 8)
    println(xs)
}
```

```output
true
[9, 8]
```

```mote
let xs = [1]
let n = xs[3] // faults IndexOutOfRange
```

## Arrays

* `[T]` and `[T; N]` index, iterate and compare like a `List`, with no `push` or `pop`. `[v; n]` fills. <!-- block -->

```mote
fn main() {
    var grid: [Int] = [0; 100]
    grid[5] = 1
    println(grid.len())
    println(grid[5])
    var total = 0
    for x in grid {
        total += x
    }
    println(total)
    let a: [Int; 2] = [1, 2]
    println(a[1])
    println(a == [1, 2])
    println(grid == [0; 100])
    for x in a {
        println(x)
    }
}
```

```output
100
1
1
2
true
false
1
2
```

* `to_list()` is a `List` of the elements, a copy; `to_array()` of a list is the way back. <!-- block -->

```mote
fn main() {
    let a: [Int] = [4, 5]
    var xs = a.to_list()
    xs.push(6)
    println(xs)
    println(a)
    println(xs.to_array() == [4, 5, 6])
}
```

```output
[4, 5, 6]
[4, 5]
true
```

```mote
let a: [Int] = [1, 2]
a.push(3) // error
```

```mote
var a: [Int] = [1, 2]
let x = a.pop() // error
```

## Bytes

* `Bytes` in `std.bytes` is a growable buffer of `UInt8`; it is a class, so a copy shares it unless `clone()` is called. <!-- block -->

<!-- block -->
| Method | Result |
|---|---|
| `Bytes.new()` `Bytes.from_array(a)` `Bytes.from_string(s)` | a new buffer |
| `len()` `is_empty()` | size |
| `get(i)` `set(i, b)` | one byte; faults out of range |
| `push(b)` `pop()` | appends; removes the last as `UInt8?` |
| `extend(a)` `append(other)` | appends a `[UInt8]` or another buffer |
| `slice(a, b)` `clone()` | a new buffer |
| `clear()` | empties |
| `to_array()` | a `[UInt8]` copy |
| `to_string()` | `String?`, `null` when the bytes are not UTF-8 |

```mote
import { Bytes } from std.bytes

fn main() {
    var b = Bytes.from_string("ab")
    b.push(99)
    b.extend("de".bytes())
    println(b.len())
    println(b.get(2))
    println(b.slice(1, 4).to_string())
    println(b.to_string())
    println(b.to_array())
    let copy = b.clone()
    b.clear()
    println(b.is_empty())
    println(copy.len())
    var bad = Bytes.new()
    bad.push(255)
    println(bad.to_string() == null)
}
```

```output
5
99
bcd
abcde
[97, 98, 99, 100, 101]
true
5
true
```

## Map

<!-- block -->
| Method | Result |
|---|---|
| `len()` `is_empty()` `clear()` | size, emptying |
| `get(k)` | the value; faults on a missing key |
| `get_or(k, d)` | the value, else `d` |
| `set(k, v)` | inserts or replaces |
| `contains_key(k)` | `Bool` |
| `remove(k)` | removes |
| `keys()` `values()` `items()` | lists |

```mote
fn main() {
    var m: Map<String, Int> = {"a": 1}
    m["b"] = 2
    m.set("c", 3)
    m.set("a", 10)
    println(m.len())
    println(m.is_empty())
    println(m.get("a"))
    println(m.get_or("z", -1))
    println(m.contains_key("b"))
    m.remove("b")
    println(m.contains_key("b"))
    println(m.keys())
    println(m.values())
    println(m.items())
    for (k, v) in m.items() {
        println("${k}=${v}")
    }
    m.clear()
    println(m.is_empty())
}
```

```output
3
false
10
-1
true
false
["a", "c"]
[10, 3]
[("a", 10), ("c", 3)]
a=10
c=3
true
```

* A `Map` or `Set` is not iterable directly. <!-- block -->

```mote
import { Set, set_from } from std.collections

let m = {"a": 1}
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

* A missing key in `get(k)` or `m[k]` faults with `missing key`. <!-- block -->

```mote
let m = {"a": 1}
let x = m.get("b") // faults Raised
```

```mote
let m = {"a": 1}
let x = m["b"] // faults Raised
```

### Keys

<!-- block -->
| Key type | Allowed |
|---|---|
| `Int`, `Bool`, `String`, `Null` | yes |
| tuples, `[T; N]`, structs, enums, optionals and unions of key types | yes, by content |
| classes, tasks, channel ends, function values | yes, by identity |
| `Float`, or any type holding one | no |
| `List`, `[T]`, `Map`, `Set`, or any type holding one | no |

```mote
import { Channel, Sender } from std.sync
import { Task } from std.task

struct P {
    x: Int
    y: Int
}

enum E {
    A(Int)
    B
}

class C {
    var n: Int = 0
}

fn main() {
    var ints: Map<Int, Int> = {}
    ints[1] = 1
    var bools: Map<Bool, Int> = {}
    bools[true] = 1
    var strings: Map<String, Int> = {}
    strings["a"] = 1
    var nulls: Map<Null, Int> = {}
    nulls[null] = 1
    var tuples: Map<(Int, String), Int> = {}
    tuples[(1, "a")] = 1
    var sized: Map<[Int; 2], Int> = {}
    let k: [Int; 2] = [1, 2]
    sized[k] = 1
    var structs: Map<P, Int> = {}
    structs[P { x: 1, y: 2 }] = 1
    structs[P { x: 1, y: 2 }] = 2
    println(structs.len())
    var enums: Map<E, Int> = {}
    enums[E.A(1)] = 1
    enums[E.B] = 2
    var optionals: Map<Int?, Int> = {}
    optionals[null] = 1
    optionals[3] = 2
    var unions: Map<Int | String, Int> = {}
    unions[1] = 1
    unions["a"] = 2
    var classes: Map<C, Int> = {}
    let c = C()
    classes[c] = 1
    classes[C()] = 2
    println(classes.len())
    println(classes[c])
    var funcs: Map<() -> Int, Int> = {}
    let f = || 1
    funcs[f] = 1
    let (tx, rx) = Channel<Int>(1)
    var ends: Map<Sender<Int>, Int> = {}
    ends[tx] = 1
    let t = spawn { return 1 }
    var tasks: Map<Task<Int>, Int> = {}
    tasks[t] = 1
    println(ints.len() + bools.len() + strings.len() + nulls.len() + tuples.len() + sized.len())
    println(enums.len() + optionals.len() + unions.len() + funcs.len() + ends.len() + tasks.len())
}
```

```output
1
2
1
6
9
```

```mote
let m: Map<Float, Int> = {} // error
```

```mote
let m = {1.5: 1} // error
```

```mote
struct F {
    x: Float
}

let m: Map<F, Int> = {} // error
```

```mote
let m: Map<List<Int>, Int> = {} // error
```

```mote
let m: Map<[Int], Int> = {} // error
```

```mote
let m: Map<Map<Int, Int>, Int> = {} // error
```

```mote
import { Set } from std.collections

let m: Map<Set<Int>, Int> = {} // error
```

```mote
struct H {
    xs: List<Int>
}

let m: Map<H, Int> = {} // error
```

* The rule is the marker trait `Key`, which the compiler checks and a type does not list. `Map<K: Key, V>` and `Set<T: Key>` check it at each use, and a type parameter used as a key is written `K: Key`. <!-- block -->

```mote
import { Key } from std.traits

fn count<K: Key>(keys: List<K>) -> Int {
    var m: Map<K, Int> = {}
    for k in keys {
        m[k] = 1
    }
    return m.len()
}

fn main() {
    println(count([1, 2, 2]))
    println(count(["a"]))
}
```

```output
2
1
```

```mote
fn count<K>(keys: List<K>) -> Int {
    var m: Map<K, Int> = {} // error
    return m.len()
}
```

```mote
import { Key } from std.traits

fn count<K: Key>(keys: List<K>) -> Int {
    return keys.len()
}

let n = count([1.5]) // error
```

## Set

<!-- block -->
| Method | Result |
|---|---|
| `len()` `is_empty()` `clear()` | size, emptying |
| `add(v)` `remove(v)` `contains(v)` | membership |
| `items()` | a list |

```mote
import { Set, set_from, union } from std.collections

fn main() {
    var s = Set<Int>()
    s.add(1)
    s.add(1)
    s.add(2)
    println(s.len())
    println(s.is_empty())
    println(s.contains(2))
    s.remove(2)
    println(s.contains(2))
    println(s.items())
    s.clear()
    println(s.is_empty())
    let a = set_from([1, 2, 3])
    let u = union(a, set_from([3, 4]))
    println(u.len())
}
```

```output
2
false
true
false
[1]
true
4
```

* `std.collections` also has `intersection`, `difference`, `map_from(pairs)`, `merge(a, b)`, `map_values(m, f)` and `filter_keys(m, pred)`; each returns a new value. <!-- block -->

```mote
import { set_from, intersection, difference, map_from, merge, map_values, filter_keys } from std.collections

fn main() {
    let a = set_from([1, 2, 3])
    let b = set_from([2, 3, 4])
    println(intersection(a, b).items())
    println(difference(a, b).items())
    let m = map_from([("a", 1), ("b", 2)])
    println(m.items())
    let n = merge(m, {"b": 20, "c": 30})
    println(n.items())
    println(m.items())
    println(map_values(m, |v| v * 2).items())
    println(filter_keys(m, |k| k == "a").items())
}
```

```output
[2, 3]
[1]
[("a", 1), ("b", 2)]
[("a", 1), ("b", 20), ("c", 30)]
[("a", 1), ("b", 2)]
[("a", 2), ("b", 4)]
[("a", 1)]
```

## Strings

* `len()` and `slice` count bytes. There is no indexing and no character type. <!-- block -->

```mote
fn main() {
    println("héllo".len())
    println("héllo".slice(0, 3))
}
```

```output
6
hé
```

```mote
let s = "abc"
let c = s[0] // error
```

<!-- block -->
| Method | Result |
|---|---|
| `len()` `is_empty()` | size in bytes |
| `concat(s)` `a + b` | joined |
| `replace(from, to)` | all occurrences replaced |
| `slice(a, b)` `s[a..b]` | substring |
| `to_upper()` `to_lower()` `trim()` | case and whitespace |
| `repeat(n)` | repeated |
| `contains(s)` `starts_with(s)` `ends_with(s)` | `Bool` |
| `split(sep)` | `List<String>` |
| `find(s)` `rfind(s)` | byte offset, `Int?` |
| `bytes()` | the UTF-8 bytes |
| `String.from_bytes(a)` | the string of `[UInt8]` bytes, or `null` |

```mote
fn main() {
    println("abc".len())
    println("".is_empty())
    println("a".concat("b"))
    println("a" + "b")
    println("a-b-c".replace("-", "+"))
    println("hello".slice(1, 3))
    println("hello"[1..3])
    println("Mote".to_upper())
    println("Mote".to_lower())
    println("  hi ".trim())
    println("ab".repeat(3))
    println("hello".contains("ell"))
    println("hello".starts_with("he"))
    println("hello".ends_with("lo"))
    println("a,b,c".split(","))
    println("hello".find("l"))
    println("hello".rfind("l"))
    println("hello".find("z") == null)
    println("AB".bytes())
    println(String.from_bytes("AB".bytes()))
    println(String.from_bytes([255, 254]) == null)
}
```

```output
3
true
ab
ab
a+b+c
el
el
MOTE
mote
hi
ababab
true
true
true
["a", "b", "c"]
2
3
true
[65, 66]
AB
true
```

* `String.from_bytes(a)` is the string of a `[UInt8]`, or `null` when it is not UTF-8. <!-- block -->

* `std.string` adds `join(parts, sep)`, `lines(s)`, `is_blank(s)`, `pad_start(s, width, pad)`, `parse_int(s)` and `parse_float(s)`; the parsers return `Result`. <!-- block -->

```mote
import { join, lines, is_blank, pad_start, parse_int, parse_float } from std.string

fn main() {
    println(join(["a", "b"], ","))
    println(lines("a\nb\r\nc"))
    println(is_blank("  "))
    println(pad_start("7", 3, "0"))
    println(parse_int("42")!)
    println(parse_float("2.5").unwrap())
}
```

```output
a,b
["a", "b", "c"]
true
007
42
2.5
```

* `parse_int` takes an optional sign and digits; `parse_float` takes digits with an optional fraction and exponent, `.5`, `inf` and `NaN`. Anything else, spaces included, is an `Err` of kind `InvalidData`. <!-- block -->

```mote
import { parse_int, parse_float } from std.string

println(parse_int("+7"))
println(parse_int(" 7"))
println(parse_float("1e3"))
println(parse_float("1,5"))
```

```output
Ok(7)
Err(Error(kind: InvalidData, message: "not an integer:  7"))
Ok(1000.0)
Err(Error(kind: InvalidData, message: "not a number: 1,5"))
```

* `split("")` gives the characters, `replace("", to)` returns the text, and `find`, `rfind` and `len` count bytes. <!-- block -->

```mote
println("abc".split(""))
println("abc".replace("", "x"))
println("héllo".len())
println("héllo".find("l"))
println("héllo".rfind("l"))
```

```output
["a", "b", "c"]
abc
6
3
4
```

* `slice` faults when an end is not on a character boundary, and `repeat` faults when the result passes the memory limit. <!-- block -->

```mote
let part = "é".slice(0, 1) // faults IndexOutOfRange
```

```mote
let huge = "ab".repeat(100000000) // faults MemoryLimit
```

## Interpolation

* `"${expr}"` inserts `expr.to_string()`; a number, `Bool` or `String` needs no method, and a type parameter needs a `Display` bound. <!-- block -->

```mote
import { Display } from std.traits

struct P: (Display) {
    x: Int

    fn to_string(self) -> String {
        return "P${self.x}"
    }
}

fn show<T: Display>(x: T) -> String {
    return "<${x}>"
}

fn main() {
    let x = 1
    let a = 2
    let b = 3
    println("x is ${x}, sum ${a + b}")
    println("${true} ${1.5} ${"s"}")
    println("${P { x: 4 }}")
    println(show(5))
    println(show(P { x: 6 }))
}
```

```output
x is 1, sum 5
true 1.5 s
P4
<5>
<P6>
```

```mote
fn show<T>(x: T) -> String {
    return "<${x}>" // error
}
```

## `std.iter`

* Functions over lists. Each returns a new list unless noted. <!-- block -->

<!-- block -->
| Function | Result |
|---|---|
| `range(n)` | `[0, …, n-1]` |
| `map(xs, f)` `flat_map(xs, f)` | transformed |
| `filter(xs, pred)` | kept elements |
| `find(xs, pred)` `find_index(xs, pred)` | `T?`, `Int?` |
| `any(xs, pred)` `all(xs, pred)` | `Bool` |
| `fold(xs, init, f)` | left fold |
| `sum(xs)` | `Int` sum |
| `reverse(xs)` `take(xs, n)` `enumerate(xs)` | reordered, shortened, `(Int, T)` pairs |
| `contains(xs, v)` | `Bool`, `T: Eq` |
| `sort(xs)` `sort_by(xs, less)` | stable sort; `sort` needs `T: Ord` |

```mote
import { range, map, flat_map, filter, find, find_index, any, all, fold, sum, reverse, take, enumerate, contains, sort, sort_by } from std.iter

fn main() {
    let xs = [3, 1, 2]
    let doubled = map([1, 2, 3], |n| n * 2)
    println(range(3))
    println(doubled)
    println(flat_map([1, 2], |n| [n, n]))
    println(filter(xs, |n| n > 1))
    println(find(xs, |n| n < 3))
    println(find_index(xs, |n| n == 2))
    println(any(xs, |n| n == 1))
    println(all(xs, |n| n == 1))
    println(fold(doubled, 0, |a, n| a + n))
    println(sum(xs))
    println(reverse(xs))
    println(take(xs, 2))
    println(enumerate(["a", "b"]))
    println(contains(xs, 2))
    println(sort(xs))
    println(sort_by(xs, |a, b| a > b))
    println(xs)
}
```

```output
[0, 1, 2]
[2, 4, 6]
[1, 1, 2, 2]
[3, 2]
1
2
true
false
12
6
[2, 1, 3]
[3, 1]
[(0, "a"), (1, "b")]
true
[1, 2, 3]
[3, 2, 1]
[3, 1, 2]
```

## Equality and copies

* `==` on collections compares element by element at any depth. <!-- block -->

```mote
fn main() {
    println([[1, 2], [3]] == [[1, 2], [3]])
    println([[1, 2], [3]] == [[1, 2], [4]])
    println({"a": [1]} == {"a": [1]})
    println([(1, [2])] == [(1, [2])])
}
```

```output
true
false
true
true
```

* `.clone()` is shallow: a new list, map, set, array or object whose elements are the same values; structs inside are copied, objects inside are shared. <!-- block -->

```mote
class Counter {
    var n: Int = 0
}

fn main() {
    var a = [Counter(), Counter()]
    let b = a.clone()
    a[0].n = 5
    println(b[0].n)
    println(a == b)
    a.push(Counter())
    println(b.len())
}
```

```output
5
true
2
```

* `.deep_clone()` copies everything reachable; strings, channels, tasks, mutexes, onces and atomics are shared, not copied. <!-- block -->

```mote
class Counter {
    var n: Int = 0
}

fn main() {
    var a = [Counter(), Counter()]
    let c = a.deep_clone()
    a[0].n = 5
    println(c[0].n)
    println(a == c)
}
```

```output
0
false
```

## Display

* `print(v)` and `println(v)` write the display form of any value. A struct, class or enum that writes `to_string(self) -> String` prints that text; inside a list, map, tuple or other value the elements print in the structural form. <!-- block -->

```mote
struct P {
    x: Int

    fn to_string(self) -> String {
        return "custom"
    }
}

fn main() {
    println([1, 2])
    println(P { x: 1 })
    println([P { x: 1 }])
    println((P { x: 1 }, 2))
    print("a")
    print("b\n")
}
```

```output
[1, 2]
custom
[P(x: 1)]
(P(x: 1), 2)
ab
```

* `typeof(v)` is its type name as a `String`. A type the prelude does not export carries its module path, package first, so `std.atomic.Int64` and `Int` differ. The blocks on this page run as package `docs`. <!-- block -->

<!-- block -->
| `typeof` of | Text |
|---|---|
| `[1]` | `List<Int>` |
| `Some(1)` | `Int?` |
| a `Set<Int>` | `std.collections.Set<Int>` |
| `atomic.Int64(0)` | `std.atomic.Int64` |
| a struct `P` in module `main` of package `docs` | `docs.main.P` |

```mote
import { Set } from std.collections
import std.atomic as atomic

struct P {
    x: Int
}

fn main() {
    println(typeof([1]))
    println(typeof(Some(1)))
    println(typeof(Set<Int>()))
    println(typeof(atomic.Int64(0)))
    println(typeof(P { x: 1 }))
    println(typeof(1))
}
```

```output
List<Int>
Int?
std.collections.Set<Int>
std.atomic.Int64
docs.main.P
Int
```

* `to_string` of a tuple, union or function writes what `print` writes: `(1, "a")`, the held member, `<function>`. <!-- block -->

```mote
fn main() {
    let u: Int | String = "s"
    let f = |n: Int| n
    println((1, "a").to_string())
    println(u.to_string())
    println(f.to_string())
}
```

```output
(1, "a")
s
<function>
```
