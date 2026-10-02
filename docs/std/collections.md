# std.collections

Set algebra and `Map` helpers. Each answers a new collection and leaves its arguments unchanged.

| Function | Answers |
|---|---|
| `set_from(xs: List<T>) -> Set<T>` | the elements of `xs`, duplicates collapsed |
| `union(a, b)`, `intersection(a, b)`, `difference(a, b)` | the set operations |
| `map_from(pairs: List<(K, V)>) -> Map<K, V>` | a map from `(key, value)` tuples |
| `merge(a, b)` | every entry of both; `b` wins on a shared key |
| `map_values(m, f)` | every value passed through `f` |
| `filter_keys(m, pred)` | the entries whose key satisfies `pred` |

```mote
import { set_from, union, intersection, map_from } from std.collections

let a = set_from([1, 2, 3])
let b = set_from([2, 3, 4])
println(union(a, b).len())
println(intersection(a, b).len())

let prices: Map<String, Int> = map_from([("bread", 3), ("milk", 2)])
println(prices.get("bread").to_string())
```

```output
4
2
3
```
