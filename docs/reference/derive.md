# Derivation

## Principle

* A derived trait is a trait with default methods that read the type through `std.meta`. A type that lists it needs no staging, no generated text and no attribute. <!-- test: derive_listing_display_debug_and_clone_needs_no_method, differential_every_block_of_std_tests_passes -->
* A type that lists the trait and writes none of its methods gets the defaults. A method it writes is kept. See [generics-traits.md](generics-traits.md#implementing-a-trait). <!-- test: a_listed_trait_is_checked_and_gives_its_defaults, differential_every_block_of_std_tests_passes -->
* The attribute set is closed and a library cannot add one. See [attributes-tests.md](attributes-tests.md#attributes). <!-- test: the_attributes_the_compiler_defines_are_these_and_no_others -->

## Describing a type

* `describe<T>()` answers a `Type` for the type `T`. In a generic function it describes the type the function was built with. <!-- test: differential_every_block_of_std_tests_passes -->
* A type that holds itself, such as a class with a field of its own type, is described once. <!-- test: differential_every_block_of_std_tests_passes -->

<!-- block -->
| `Type` method | Answers |
|---|---|
| `name()` | the name as `typeof` writes it, such as `List<Int>` |
| `short_name()` | the declared name without its module or type arguments, such as `Pair` or `List`; `name()` for a type with no declaration, such as a tuple |
| `kind()` | a `Kind`; `Handle` for `Task`, `Sender`, `Receiver`, `Mutex`, `Once`, `atomic.Int64`, `atomic.Bool` and the ends of a port |
| `doc()` | the `///` lines before the declaration, joined with a line break |
| `args()` | the type arguments, the element type, the members of a tuple or union, or a function's parameters then its result |
| `generics()` | the names of the type parameters |
| `traits()` | the traits the declaration lists |
| `methods()` | the methods it writes, each with `name`, `signature` (as source writes it), `is_pub`, `has_self`, `writes_self` |
| `attributes()` | the attributes before the declaration, each with `name` and `args`, a list of `key` (empty when bare) and `value` (as text) |
| `variants()` | each variant of an enum, with `name`, `doc`, `shape` (`Unit`, `Tuple`, `Named`), `fields` and `discriminant` |
| `fields()` | each field of a struct or class, with `name`, `ty`, `doc`, `is_var`, `is_pub`, `has_default` |

```mote
import { describe, Kind, Shape } from std.meta
import { Int64 } from std.atomic
import { Display } from std.traits

/// A point on the plane.
struct Point: (Display) {
    pub var x: Int
    y: Int = 0
}

class Counter {
    var count: Int

    pub fn bump(var self) { self.count += 1 }
    fn peek(self) -> Int { return self.count }
}

enum Level {
    Low = 1
    /// The top.
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
    }
}

fn main() {
    let pair = describe<Pair<Int, String>>()
    println("${describe<List<Int>>().name()} ${pair.short_name()} ${word(pair.kind())}")
    println(describe<List<Int>>().short_name())
    println(describe<(Int, String)>().short_name())
    println(describe<Point>().doc())
    println(pair.args()[1].name())
    println(pair.generics()[0])
    println(describe<Point>().traits()[0])
    let methods = describe<Counter>().methods()
    println("${methods[0].name} ${methods[0].is_pub} ${methods[0].has_self} ${methods[0].writes_self}")
    println("${methods[1].name} ${methods[1].is_pub} ${methods[1].writes_self}")
    println(describe<List<Int>>().attributes()[0].name)
    let variants = describe<Level>().variants()
    println("${variants[0].name} ${variants[0].discriminant!} ${variants[1].doc}")
    println(variants[1].shape == Shape.Tuple)
    let fields = describe<Point>().fields()
    println("${fields[0].name} ${fields[0].is_var} ${fields[0].is_pub} ${fields[0].has_default}")
    println("${fields[1].name} ${fields[1].is_var} ${fields[1].is_pub} ${fields[1].has_default}")
    println(word(describe<Int64>().kind()))
}
```

```output
List<Int> Pair Struct
List
(Int, String)
A point on the plane.
String
A
Display
bump true true true
peek false false
lang
Low 1 The top.
true
x true true false
y false false true
Handle
```

## Reading and writing values

* `Dyn` holds a value of any described type and reads and writes it by position. <!-- test: differential_every_block_of_std_tests_passes -->

<!-- test: differential_every_block_of_std_tests_passes -->
| Call | Does |
|---|---|
| `Dyn.of(value)` | holds the value |
| `downcast<T>(d)` | the value as a `T`, or null when it is another type |
| `d.ty()` | the `Type` of the value |
| `d.len()` | the number of parts: the fields of a struct, class or tuple, the payload of an enum value, the elements of a `List`, array or `Set`, a key then a value for each entry of a `Map`, the value of an optional that has one; 0 for any other type |
| `d.tag()` | the position of the variant an enum value holds; 0 for any other type |
| `d.get(i)` | part `i` as a `Dyn` |
| `d.set(i, part)` | replaces part `i` |
| `build<T>(tag, parts)` | a struct, class, tuple or enum value from a list of `Dyn`; `tag` is the variant |
| `build_dyn(ty, tag, parts)` | a `Dyn` of a `Type` known only at run time: as `build`, the elements of a `List`, `[T]` or `Set`, the keys and values of a `Map` in turn (an equal key later replaces an earlier one), or no part or one for an optional |

<!-- test: differential_meta_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results -->
| Fault | When |
|---|---|
| index out of range | `get` or `set` with `i` outside the parts; `build` of an enum with no variant `tag`; `build_dyn` of a `[T; N]` |
| a part of another type | `set`, `build` or `build_dyn` with a part whose type is not the part's |
| a key cannot be replaced | `set` on a key of a `Map` or a member of a `Set` |
| the wrong number of parts | `build` or `build_dyn` with more or fewer parts than the type has, or two for an optional |
| this type cannot be built | `build` of a type that is not a struct, class, tuple or enum; `build_dyn` of a type that is none of those, a `List`, a `[T]`, a `Map`, a `Set` or an optional |

* A struct, tuple, enum value or `[T; N]` is held as a copy: `set` changes the `Dyn`, not the value it came from. A class instance, a `List` and a `[T]` are shared: `set` changes the object. <!-- block -->

```mote
import { Dyn, build, downcast } from std.meta

struct Point {
    x: Int
    y: Int
}

class Counter {
    var count: Int
}

fn main() {
    let p = Point { x: 1, y: 2 }
    let d = Dyn.of(p)
    d.set(0, Dyn.of(10))
    let moved: Point = build(0, [d.get(0), d.get(1)])
    println(downcast<Int>(d.get(0))!)
    println("${p.x} ${moved.x}")

    let c = Counter { count: 1 }
    Dyn.of(c).set(0, Dyn.of(7))
    println(c.count)
}
```

```output
10
1 10
7
```

* `build_dyn` makes a value of a type that the code reads from a `Type`, such as a field type, where `build<T>` needs `T` in the source. <!-- block -->

```mote
import { Dyn, build_dyn, describe, downcast } from std.meta

class Team {
    name: String
    sizes: List<Int>
    lead: String?
}

fn main() {
    let team = describe<Team>()
    let fields = team.fields()
    let sizes = build_dyn(fields[1].ty, 0, [Dyn.of(3), Dyn.of(5)])
    let lead = build_dyn(fields[2].ty, 0, [Dyn.of("Ann")])
    let made = downcast<Team>(build_dyn(team, 0, [Dyn.of("core"), sizes, lead]))!
    println(made.name)
    println(made.sizes)
    println(made.lead)
}
```

```output
core
[3, 5]
Ann
```

## Writing a derived trait

* A trait writes its default methods over `std.meta`. Its module's imports are the ones the body sees; a type that lists the trait imports nothing from `std.meta`. <!-- block -->

```mote
import { describe, require, show, Kind } from std.meta

trait Labeled {
    fn label(self) -> String {
        require(describe<Self>().kind() == Kind.Struct, "needs a struct")
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
| In `std.meta` | Answers |
|---|---|
| `show(value)` | what `Display` writes: `Point(x: 1, y: 2)` |
| `debug_text(value)` | what `Debug` writes: `Point { x: 1, y: 2 }` |
| `duplicate(value)` | what `Clone` writes: a new struct, class, tuple or enum value with the same parts |

```mote
import { show, debug_text, duplicate } from std.meta

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

## `require`

* `require(condition, message)` stops the program with `message` unless `condition` holds. <!-- test: require_that_holds_in_a_plain_function_is_no_error_and_the_program_runs, require_that_reads_a_parameter_is_left_to_fault_when_it_runs, require_after_a_branch_is_left_to_fault_when_it_runs -->
* Before any branch of a function, with arguments that read no parameter and no global, it is decided before the program runs. Unmet, it is an error where a type lists the trait the default method comes from, or at the call elsewhere. <!-- test: require_that_holds_for_the_listing_type_is_no_error, require_that_fails_is_an_error_naming_the_type_the_trait_and_the_message_though_nothing_calls_the_method, require_points_at_the_listing_type, require_in_a_plain_function_is_an_error_with_only_its_message, require_with_a_message_built_by_a_call_reports_the_built_text -->
* A type with no type parameters is checked where it lists the trait, even when nothing calls the method. A type with parameters is checked where it is used. <!-- test: require_in_a_default_method_builds_it_for_each_type_without_parameters_that_lists_the_trait, require_that_fails_is_an_error_naming_the_type_the_trait_and_the_message_though_nothing_calls_the_method, require_in_a_type_with_parameters_is_checked_where_the_type_is_used -->
* Any other `require` faults with `message` when it runs. <!-- block -->

```mote
import { require } from std.meta

fn positive(n: Int) {
    require(n > 0, "needs a positive number") // faults Raised
}

fn main() {
    positive(0)
}
```

<!-- test: require_that_fails_is_an_error_naming_the_type_the_trait_and_the_message_though_nothing_calls_the_method, require_in_a_plain_function_is_an_error_with_only_its_message -->
| Error | Text |
|---|---|
| in a default method | `` `Shape` cannot list `Labeled`: needs a struct `` for an enum `Shape` that lists `Labeled` |
| elsewhere | the message |

## Limits

* A trait declares the methods it derives. A type that lists it gets those names and no others; a required method with no default is still an error. <!-- test: derive_a_trait_without_a_default_still_needs_its_method, a_listed_trait_is_checked_and_gives_its_defaults -->
* A derived `Display` and `Debug` show a field by its fields, whatever methods the field's type writes. <!-- test: differential_every_block_of_std_tests_passes -->
