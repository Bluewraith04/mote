# Derivation and static reflection

## Principle

* A derived trait is a trait with default methods that inspect the type at compile time via compiler builtins (`@type_info`, `@fields`, `@variants`) and `std.reflect`. A type that lists it needs no staging, no generated text and no attribute. <!-- test: derive_listing_display_debug_and_clone_needs_no_method, differential_every_block_of_std_tests_passes -->
* A type that lists the trait and writes none of its methods gets the defaults. A method it writes is kept. See [generics-traits.md](generics-traits.md#implementing-a-trait). <!-- test: a_listed_trait_is_checked_and_gives_its_defaults, differential_every_block_of_std_tests_passes -->
* The attribute set is closed and a library cannot add one. See [attributes-tests.md](attributes-tests.md#attributes). <!-- test: the_attributes_the_compiler_defines_are_these_and_no_others -->

## Describing a type

* `@type_info(T)` returns a `TypeInfo`, `@fields(T)` returns `[FieldInfo; N]`, and `@variants(T)` returns `[VariantInfo; M]` at compile time. In a generic function they describe the concrete type `T` the function is instantiated with. <!-- test: differential_every_block_of_std_tests_passes -->

<!-- block -->
| Builtin / Field | Answers |
|---|---|
| `@type_name(T)` | the name as `typeof` writes it, such as `List<Int>` |
| `@type_info(T).name` | the full type name, such as `docs.main.Pair<Int, String>` or `List<Int>` |
| `@type_info(T).kind` | a `Kind`; `Handle` for `Task`, `Sender`, `Receiver`, `Mutex`, `Once`, `AtomicInt`, `AtomicBool` and `PortEnd` |
| `@type_info(T).doc` | the `///` lines before the declaration, joined with a line break |
| `@type_info(T).field_count` / `variant_count` | number of fields on a struct/class/tuple or variants on an enum/union |
| `@fields(T)` | each field of a struct, class or tuple, with `name`, `type_name`, `doc`, `index`, `is_var`, `is_pub`, `has_default` |
| `@variants(T)` | each variant of an enum, with `name`, `shape` (`Unit`, `Tuple`, `Named`), `field_count`, `index` |
| `@FieldType(T, name)` / `@TypeArg(T, i)` | the compile-time type of a field or generic/tuple/union argument |
| `@size_of(T)` / `@align_of(T)` | layout size and alignment in bytes |
| `@typeof(x)` | the static type of the expression `x`, usable wherever a type is: `@type_info(@typeof(x))`, `@fields(@typeof(x))`, `let y: @typeof(x) = ...`, or `const t = @typeof(x)`. It is a type, not a value, so it cannot be printed; `typeof(x)` is the String the runtime gives, and for a union or `Any` it names the member held |

```mote
import { Kind, Shape } from std.reflect
import Display from std.traits

/// A point on the plane.
struct Point: (Display) {
    pub var x: Int
    y: Int = 0
}

enum Level {
    Low = 1
    High(Int)
}

struct Pair<A, B> {
    first: A
    second: B
}

fn word(kind: Kind) -> String {
    return match kind {
        Kind.Null => "Null"
        Kind.Bool => "Bool"
        Kind.Int => "Int"
        Kind.Float => "Float"
        Kind.String => "String"
        Kind.Tuple => "Tuple"
        Kind.Array => "Array"
        Kind.List => "List"
        Kind.Map => "Map"
        Kind.Set => "Set"
        Kind.Optional => "Optional"
        Kind.Union => "Union"
        Kind.Struct => "Struct"
        Kind.Class => "Class"
        Kind.Enum => "Enum"
        Kind.Function => "Function"
        Kind.Handle => "Handle"
        Kind.FixedArray => "FixedArray"
        Kind.Any => "Any"
    }
}

fn main() {
    const pair = @type_info(Pair<Int, String>)
    println("${@type_name(List<Int>)} ${pair.name} ${word(pair.kind)}")
    println(@type_info(Point).doc)
    println(@type_name(@TypeArg(Pair<Int, String>, 1)))
    const variants = @variants(Level)
    println("${variants[0].name} ${variants[0].index}")
    println(variants[1].shape == Shape.Tuple)
    const fields = @fields(Point)
    println("${fields[0].name} ${fields[0].is_var} ${fields[0].is_pub} ${fields[0].has_default}")
    println("${fields[1].name} ${fields[1].is_var} ${fields[1].is_pub} ${fields[1].has_default}")
    println(word(@type_info(AtomicInt).kind))
}
```

```output
List<Int> docs.main.Pair<Int, String> Struct
A point on the plane.
String
Low 0
true
x true true false
y false false true
Handle
```

## Reading and writing fields statically

* With `comptime for f in @fields(T)`, `@field(value, f.name)` reads a field and `@set_field(value, f.name, new_value)` mutates a `var` binding at compile time with zero runtime reflection overhead. <!-- block -->

```mote
struct Point {
    var x: Int
    var y: Int
}

fn shift<T>(var value: T, delta: Int) {
    comptime for f in @fields(T) {
        @set_field(value, f.name, @field(value, f.name) + delta)
    }
}

fn main() {
    var p = Point { x: 1, y: 2 }
    shift(p, 10)
    println("${p.x} ${p.y}")
}
```

```output
11 12
```

## Writing a derived trait

* A trait writes its default methods using `std.reflect` and `@compile_error` / `comptime if`. <!-- block -->

```mote
import { Kind, show } from std.reflect

trait Labeled {
    fn label(self) -> String {
        comptime if @type_info(Self).kind != Kind.Struct {
            @compile_error("needs a struct")
        }
        return "<" + show(self) + ">"
    }
}

struct Point: (Labeled) {
    x: Int
    y: Int
}

fn main() {
    println(Point { x: 1, y: 2 }.label())
}
```

```output
<Point(x: 1, y: 2)>
```

<!-- block -->
| In `std.reflect` | Answers |
|---|---|
| `show(value)` | what `Display` writes: `Point(x: 1, y: 2)` |
| `debug_text(value)` | what `Debug` writes: `Point { x: 1, y: 2 }` |
| `duplicate(value)` | what `Clone` writes: a new struct, class, tuple or enum value with the same parts |

```mote
import { show, debug_text, duplicate } from std.reflect

struct Point {
    x: Int
    y: Int
}

fn main() {
    let p = Point { x: 1, y: 2 }
    println(show(p))
    println(debug_text(p))
    let q = duplicate(p)
    println(show(q))
}
```

```output
Point(x: 1, y: 2)
Point { x: 1, y: 2 }
Point(x: 1, y: 2)
```

## Limits

* A trait declares the methods it derives. A type that lists it gets those names and no others; a required method with no default is still an error. <!-- test: derive_a_trait_without_a_default_still_needs_its_method, a_listed_trait_is_checked_and_gives_its_defaults -->
* A derived `Display` and `Debug` show a field by its fields, whatever methods the field's type writes. <!-- test: differential_every_block_of_std_tests_passes -->
