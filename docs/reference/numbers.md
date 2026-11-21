# Numbers

## Types

* `Int` and `Float` are the 64-bit defaults, also written `Int64` and `Float64`. `Int8 Int16 Int32 UInt8 UInt16 UInt32 UInt64` and `Float32` are the sized types. <!-- block -->
* An unsuffixed integer literal is `Int` unless context names another integer type; an unsuffixed float literal is `Float` unless context names `Float32`. <!-- block -->

```mote
fn main() {
    let a: UInt8 = 200
    let b: Float32 = 1.5
    let c = 7
    let d = 1.5
    let e: Int64 = c
    let f: Float64 = d
    println(typeof(a))
    println(typeof(b))
    println(typeof(c))
    println(typeof(d))
    println(typeof(e))
    println(typeof(f))
}
```

```output
UInt8
Float32
Int
Float
Int
Float
```

## Suffixes

* A suffix makes the literal that type without an annotation: `i8 i16 i32 i64 u8 u16 u32 u64 f32 f64`. <!-- block -->

```mote
fn main() {
    let a = 1i8
    let b = 1i16
    let c = 1i32
    let d = 1i64
    let e = 1u8
    let f = 1u16
    let g = 1u32
    let h = 1u64
    let i = -5i16
    let j = 2.5f32
    let k = 2.5f64
    println(typeof(a))
    println(typeof(b))
    println(typeof(c))
    println(typeof(d))
    println(typeof(e))
    println(typeof(f))
    println(typeof(g))
    println(typeof(h))
    println(typeof(i))
    println(typeof(j))
    println(typeof(k))
}
```

```output
Int8
Int16
Int32
Int
UInt8
UInt16
UInt32
UInt64
Int16
Float32
Float
```

* A literal that does not fit its type is an error. <!-- block -->

```mote
let n = 256u8 // error: 256 does not fit UInt8
```

```mote
let n: Int8 = 128 // error: 128 does not fit Int8
```

## No implicit conversion

* Operators need both operands of one type. `UInt8 + Int` and `Int + Float` are errors. <!-- block -->

```mote
let a = 1 + 1.5 // error
```

```mote
let a: UInt8 = 1
let n: Int = 2
let b = a + n // error
```

## Casts with `as`

* `e as T` converts between scalar types with these rules: int to int wraps; float to int saturates, `NaN` is `0`; int to float rounds to nearest; `Bool` to int is `0` or `1`. <!-- block -->

```mote
fn main() {
    println(300 as UInt8)
    println(-1 as UInt8)
    println(3.9 as Int)
    println(-1.0 as UInt8)
    println(1e30 as Int)
    println((0.0 / 0.0) as Int)
    println(9007199254740993 as Float)
    println(7 as Float)
    println(true as Int)
    println(false as Int)
}
```

```output
44
255
3
0
9223372036854775807
0
9007199254740992.0
7.0
1
0
```

* `as` binds tighter than `..` and looser than prefix operators: `-a as Int` is `(-a) as Int`. <!-- block -->

```mote
fn main() {
    let a = 1.0
    println(-a as UInt8)
    for i in 0..2.9 as Int {
        println(i)
    }
}
```

```output
0
0
1
```

* A `?` right after `as T` belongs to the type: `a as Int?` is a cast to `Int?`, which is an error because only scalar types are cast to. <!-- block -->

```mote
let a = 5.0
let b = a as Int? // error: cannot cast Float to Int?
```

* `as` also renames an import: `import std.string as text`. <!-- block -->

```mote
import std.string as text

fn main() {
    println(text.join(["a", "b"], "-"))
}
```

```output
a-b
```

## Richer conversions

* A conversion that is not a scalar cast is the target type's static `from`: `String.from(x)`, `Float.from(3)`, or a user type's own `from`. <!-- block -->

```mote
struct Meters {
    n: Int

    fn from(n: Int) -> Meters { return Meters { n: n } }
}

fn main() {
    println(String.from(42))
    println(Float.from(3))
    println(Meters.from(5).n)
}
```

```output
42
3.0
5
```

* A type may have several `from`, one per source type; the argument's type picks one. Only `from` may be overloaded. <!-- block -->

```mote
struct Meters {
    n: Int

    fn from(n: Int) -> Meters { return Meters { n: n } }
    fn from(text: String) -> Meters { return Meters { n: text.len() } }
}

fn main() {
    println(Meters.from(5).n)
    println(Meters.from("abc").n)
    println(Int.from(2.7))
}
```

```output
5
3
2
```

```mote
struct Meters {
    n: Int

    fn make(n: Int) -> Meters { return Meters { n: n } }
    fn make(text: String) -> Meters { return Meters { n: 0 } } // error
}
```

* An overloaded `from` is only called as `Meters.from(value)`: it is not called on a value and is not a value. <!-- block -->

```mote
struct Meters {
    n: Int

    fn from(n: Int) -> Meters { return Meters { n: n } }
    fn from(text: String) -> Meters { return Meters { n: text.len() } }
}

fn main() {
    let m = Meters.from(5)
    println(m.from(3).n) // error
    let g = Meters.from // error
}
```

* A conversion that can fail has another name and returns a `Result`: `std.string.parse_int`. <!-- block -->

```mote
import { parse_int } from std.string

fn main() {
    println(parse_int("42"))
    println(parse_int("x").is_err())
}
```

```output
Ok(42)
true
```

## Overflow and division

* Integer overflow, division or remainder by zero, `Int.MIN / -1` and a shift past the width fault in every build. <!-- block -->

```mote
let a = 255u8 + 1 // faults IntegerOverflow
```

```mote
let a = 9223372036854775807 + 1 // faults IntegerOverflow
```

```mote
let z = 0
let a = 1 / z // faults DivideByZero
```

```mote
let z = 0
let a = 1 % z // faults DivideByZero
```

```mote
let m = -9223372036854775807 - 1
let n = -1
let a = m / n // faults IntegerOverflow
```

```mote
let s: UInt8 = 9
let a = 1u8 << s // faults ShiftOutOfRange
```

* Explicit forms by method name: `wrapping_add`, `checked_add`, `saturating_add`, `overflowing_add`, and the same for `sub mul neg shl shr`. <!-- block -->

```mote
fn main() {
    println(255u8.wrapping_add(1))
    println(255u8.checked_add(1) == null)
    println(255u8.saturating_add(1))
    println(255u8.overflowing_add(1))
    println(0u8.wrapping_sub(1))
    println(0u8.checked_sub(1) == null)
    println(0u8.saturating_sub(1))
    println(0u8.overflowing_sub(1))
    println(16u8.wrapping_mul(16))
    println(16u8.checked_mul(16) == null)
    println(16u8.saturating_mul(16))
    println(16u8.overflowing_mul(16))
    println(1u8.wrapping_neg())
    println(1u8.checked_neg() == null)
    println(1u8.saturating_neg())
    println(1u8.overflowing_neg())
    println(1u8.wrapping_shl(1))
    println(1u8.checked_shl(1))
    println(1u8.saturating_shl(1))
    println(1u8.overflowing_shl(1))
    println(2u8.wrapping_shr(1))
    println(2u8.checked_shr(1))
    println(2u8.saturating_shr(1))
    println(2u8.overflowing_shr(1))
}
```

```output
0
true
255
(0, true)
255
true
0
(255, true)
0
true
255
(0, true)
255
true
0
(255, true)
2
2
2
(2, false)
1
1
1
(1, false)
```

## Division and remainder

* Integer `/` truncates toward zero; `%` takes the sign of the left operand. <!-- block -->

```mote
fn main() {
    println(-7 / 2)
    println(-7 % 3)
    println(7 % -3)
}
```

```output
-3
-1
1
```

* `Float` follows IEEE 754: division by zero gives infinity or `NaN`, not a fault. <!-- block -->
* A `Float` prints as the shortest text that reads back, with `.0` when it is whole, and `NaN`, `inf` or `-inf`. <!-- block -->

```mote
println(1.0)
println(0.1 + 0.2)
println(1.0 / 0.0)
println(-1.0 / 0.0)
println(0.0 / 0.0)
```

```output
1.0
0.30000000000000004
inf
-inf
NaN
```

## Shifts by name

* `wrapping_shl` and `wrapping_shr` take the amount modulo the width. `checked_shl`, `checked_shr` and the `overflowing_` shifts report only an amount outside the width, not bits shifted out. `saturating_shl` gives the largest value when the result does not fit. <!-- block -->

```mote
println(1u8.wrapping_shl(9))
println(1u8.checked_shl(9))
println(1u8.checked_shl(3))
println(200u8.saturating_shl(1))
println(1u8.overflowing_shl(9))
```

```output
2
null
8
255
(2, true)
```
