# std.math

`import std.math as math`. `Int` and `Float` do not overload, so the `Float` forms of `abs`, `min` and `max` are `fabs`, `fmin` and `fmax`.

| Group | Functions |
|---|---|
| constants | `pi()`, `e()`, `tau()`, `inf()`, `nan()` |
| one argument | `sqrt cbrt exp ln log2 log10 sin cos tan asin acos atan floor ceil round trunc fabs` |
| two arguments | `pow(x, y)`, `log(x, base)`, `atan2(y, x)`, `hypot(x, y)`, `copysign(x, sign)`, `fmin`, `fmax` |
| predicates | `is_nan`, `is_infinite`, `is_finite` |
| conversion | `to_float(n)`; `to_int(x)` truncates toward zero and faults on `NaN` or infinity |
| `Int` | `abs`, `sign`, `min`, `max`, `clamp(x, lo, hi)`, `gcd`, `lcm`, `ipow(base, exp)` |
| wrapping and saturating | `wrapping_add/sub/mul`, `saturating_add/sub/mul` |

```mote
import std.math as math

println(math.sqrt(16.0))
println(math.clamp(15, 0, 10))
println(math.gcd(12, 18))
```

```output
4.0
10
6
```
