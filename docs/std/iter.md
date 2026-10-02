# std.iter

Eager adapters over a `List`. Each answers a new list; the input is unchanged. `import std.iter as it`.

| Function | Answers |
|---|---|
| `range(n)` | `[0, 1, …, n-1]` |
| `map(xs, f)`, `flat_map(xs, f)` | `f` applied to every element, optionally flattened one level |
| `filter(xs, pred)` | the elements for which `pred` is true |
| `find(xs, pred)`, `find_index(xs, pred)` | the first match, or its index, as an optional |
| `any(xs, pred)`, `all(xs, pred)` | whether some, or every, element satisfies `pred` |
| `fold(xs, init, f)` | a left fold |
| `sum(xs: List<Int>)` | the sum |
| `reverse(xs)`, `take(xs, n)` | reversed; the first `n` |
| `enumerate(xs)` | `(index, element)` tuples |
| `contains(xs, target)` | whether any element `==` `target` |
| `sort(xs)`, `sort_by(xs, less)` | ascending by `compare` (`T: Ord`), or by `less(a, b)`; stable insertion sort, O(n²) |

```mote
import std.iter as it

let evens = it.filter(it.range(10), |x| x % 2 == 0)
println(evens)
println(it.sum(evens))
```

```output
[0, 2, 4, 6, 8]
20
```

Callbacks are typed: `it.map(xs, |x| x * 2)` on a `List<Int>` answers a `List<Int>`. [`std.stream`](stream.md) has the lazy equivalents.
