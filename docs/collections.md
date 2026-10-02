# Collections and strings

| Type | Holds | Literal or constructor |
|---|---|---|
| `List<T>` | an ordered, growable sequence | `[1, 2, 3]` |
| `Map<K, V>` | values by key, in insertion order | `{"a": 1}` or `Map()` |
| `Set<T>` | distinct values | `Set()` |
| `Bytes` | a growable sequence of `0` to `255` | `Bytes()` or `Bytes(n)` |
| `String` | immutable UTF-8 text | `"text"` |

The methods named here need no import; the [prelude](std/prelude.md) lists them all. Element types are checked: `xs.push("nope")` on a `List<Int>` is an error.

## Lists

```mote
var xs = [1, 2, 3]
xs.push(4)
xs[1] = 99
xs[0] += 10
println(xs)
println(xs.len())
println(xs.pop())
println(xs)
```

```output
[11, 99, 3, 4]
4
4
[11, 99, 3]
```

| Call | Result |
|---|---|
| `xs[i]`, `xs.get(i)` | the element; a fault when out of range |
| `xs.get_or(i, d)` | the element, or `d` |
| `xs[i] = v`, `xs.set(i, v)` | replaces an element |
| `xs.push(v)`, `xs.pop()` | appends; removes the last as a `T?` |
| `xs.first()`, `xs.last()` | the end elements as a `T?` |
| `xs.contains(v)` | whether some element `==` `v` |
| `xs.len()`, `xs.is_empty()`, `xs.clear()` | size, emptying |

There is no insert, remove-by-index, slice or reverse; sorting and reversing are in [`std.iter`](std/iter.md).

## Maps and sets

A map keeps insertion order for `keys()`, `values()`, `entries()` and printing. Setting an existing key keeps its place.

```mote
var m = {"a": 1, "b": 2}
m["c"] = 3
println(m)
println(m.get_or("zzz", 0))
println(m.remove("a"))
println(m.keys())

var s: Set<Int> = Set()
s.add(1)
s.add(1)
s.add(2)
println(s.len())
println(s.contains(2))
```

```output
{"a": 1, "b": 2, "c": 3}
0
true
["b", "c"]
2
true
```

| Call | Result |
|---|---|
| `m[k]`, `m.get(k)` | the value; a fault when missing |
| `m.get_or(k, d)` | the value, or `d` |
| `m[k] = v`, `m.set(k, v)` | adds or replaces |
| `m.contains_key(k)`, `m.remove(k)` | `Bool`; whether the key was there |
| `m.keys()`, `m.values()`, `m.entries()` | lists |
| `s.add(v)`, `s.contains(v)`, `s.remove(v)`, `s.items()` | set operations |

`for x in map` and `for x in set` are errors; loop over `keys()` or `items()`. Keys and set elements match by `==`: by content for strings, numbers, tuples, structs and enum variants, by identity for class instances. A type holding a `List`, `Map` or `Set` cannot be a key; use a tuple. Set algebra is in [`std.collections`](std/collections.md).

## Bytes

```mote
var b = Bytes()
b.push(72)
b.push(105)
println(b[0])
println(b.len())
println(b.decode())
```

```output
72
2
Hi
```

`Bytes` also has `get`, `set`, `extend(other)`, `slice(start, end)`, `contains`, `clear`. `push` of a value outside 0 to 255 is a fault, and `decode()` gives a `String?`, `None` for invalid UTF-8.

## Strings

A string cannot be indexed with `s[i]`; `len` and `slice` count bytes, and `slice` faults off a character boundary.

| Call | Result |
|---|---|
| `a + b`, `a.concat(b)` | the joined string |
| `s.len()`, `s.is_empty()` | size in bytes |
| `s.slice(start, end)` | the part between two byte offsets |
| `s.find(sub)`, `s.rfind(sub)` | the byte offset as an `Int?` |
| `s.contains(sub)`, `s.starts_with(sub)`, `s.ends_with(sub)` | `Bool` |
| `s.replace(from, to)` | every match replaced |
| `s.to_upper()`, `s.to_lower()` | ASCII letters only |
| `s.trim()`, `s.split(sep)`, `s.repeat(n)`, `s.bytes()` | as named |

```mote
let line = "  Hello, Mote  "
let t = line.trim()
println(t.to_upper())
println(t.find("Mote"))
println(t.split(", "))
```

```output
HELLO, MOTE
7
["Hello", "Mote"]
```

There is no `format` with width or alignment. [`std.string`](std/string.md) has `join`, `lines`, `pad_start`, `parse_int` and `parse_float`.

## Iterating with `std.iter`

`std.iter` functions take a list and return a new one, eagerly.

```mote
import std.iter as it

let evens = it.filter(it.range(10), |x| x % 2 == 0)
println(evens)
println(it.sum(evens))
println(it.sort_by([3, 1, 2], |a, b| a > b))
```

```output
[0, 2, 4, 6, 8]
20
[3, 2, 1]
```
