# Mote Reference

Dense language reference. Each page is sections of statements and snippets. Load one page for one topic, or all of them.

<!-- test: reference_readme_lists_the_pages_on_disk -->
| Page | Covers |
|---|---|
| [lexical.md](lexical.md) | comments, names, keywords, newlines, literals, operators as tokens |
| [types.md](types.md) | primitive and sized numbers, arrays, tuples, nullable, union, function types, aliases, assignability |
| [numbers.md](numbers.md) | suffixes, casts, overflow, `from` conversions |
| [bindings.md](bindings.md) | `let`, `var`, destructuring, assignment, globals |
| [operators.md](operators.md) | precedence, arithmetic, equality, logic, `is`, `as`, `?`, `!`, `??`, `?.`, indexing, slicing |
| [control-flow.md](control-flow.md) | `if`, `while`, `for`, `match`, value `if`/`match`, `if let`, `with`, `return`, `break` |
| [patterns.md](patterns.md) | every pattern form, guards, exhaustiveness |
| [functions.md](functions.md) | parameters, defaults, named and variadic arguments, `var` parameters, lambdas, generators, nested `fn` |
| [structs-classes.md](structs-classes.md) | `struct`, `class`, literals, methods, construction, field defaults |
| [enums.md](enums.md) | variants, payloads, methods, enum or union |
| [generics-traits.md](generics-traits.md) | type parameters, bounds, traits, derived methods |
| [derive.md](derive.md) | `std.reflect`, `@type_info`, `comptime`, writing a derived trait |
| [optionals-errors.md](optionals-errors.md) | `T?`, `Result`, narrowing, `Error`, methods |
| [collections.md](collections.md) | `List`, `Map`, `Set`, arrays, strings, `std.iter` |
| [modules.md](modules.md) | `import`, `pub`, visibility, packages, globals, `extern` |
| [concurrency.md](concurrency.md) | `spawn`, `scope`, channels, `Mutex`, `Once`, atomics, Sendable |
| [attributes-tests.md](attributes-tests.md) | `@` attributes, doc comments, `test` blocks |
| [contract.md](contract.md) | universal builtins, compiler intrinsics and fault kinds |
| [command.md](command.md) | the `mote` command |

## Conventions

* A `mote` block is built as a program of its own, or after the blocks before it on its page that only declare, and run when it has a `main`. `integration/fragments.txt` lists the blocks that are neither; `integration/broken.txt` lists those that run and disagree with the page. <!-- test: docs_every_block_is_run_rejected_as_marked_or_listed_as_a_fragment, docs_harness_runs_a_block_after_the_declarations_before_it_on_its_page -->
* `// error` on a line says the block is rejected, and `// error: text` that an error says `text`. `// faults Kind` says the block ends in a fault of that kind, such as `IntegerOverflow`. An `output` block right after a `mote` block is exactly what it prints. <!-- test: docs_harness_wants_an_error_mark_to_be_rejected_with_the_message_it_names, docs_harness_wants_the_kind_a_fault_mark_names, docs_harness_pairs_an_output_block_with_the_mote_block_before_it, docs_harness_compares_what_a_block_prints_with_its_output_block -->
* A fault is a run-time stop with a located message: `fault: index out of range` and the file and line. <!-- test: run_tests_a_fault_in_main_ends_with_code_1_and_the_line -->
* An error is a compile error, printed with its source line. <!-- test: pipeline_prints_a_diagnostic_of_the_checker_with_its_source_line -->
* A statement ends at a newline, and a `;` may separate two statements on one line. <!-- block -->

```mote
fn main() {
    let a = 1; let b = 2
    println(a + b)
}
```

```output
3
```

* There is no implicit last-line result. <!-- block -->

```mote
fn one() -> Int {
    1 // error
}
```

* There is no `impl`. <!-- block -->

```mote
struct Point {
    x: Int
}

impl Point {} // error
```

* There is no inheritance: a class lists traits, not a class. <!-- block -->

```mote
class Animal {}
class Dog: (Animal) {} // error
```

* There are no exceptions. <!-- block -->

```mote
fn main() {
    throw Error(ErrorKind.Other, "no") // error
}
```

## Decisions

* `Int64` and `Float64` are other names for `Int` and `Float`. <!-- block -->

```mote
fn main() {
    let a: Int64 = 5
    let b: Int = a
    let c: Float64 = 1.5
    let d: Float = c
    println(b)
    println(d)
}
```

```output
5
1.5
```

* A block lambda has a value only through `return`. <!-- block -->

```mote
fn main() {
    let f = |x: Int| { return x + 1 }
    println(f(1))
}
```

```output
2
```

* A block lambda without `return` has no value. <!-- block -->

```mote
let g = |x: Int| { x + 1 }
let n: Int = g(1) // error: expected Int, found Null
```

* `deep_clone` is a method of every value. <!-- block -->

```mote
fn main() {
    var a = [[1], [2]]
    let b = a.deep_clone()
    a[0].push(9)
    println(a)
    println(b)
}
```

```output
[[1, 9], [2]]
[[1], [2]]
```

* An array and a list do not convert by assignment. <!-- block -->

```mote
let l: List<Int> = [1, 2]
let r: [Int] = l // error: expected [Int], found List<Int>
```

* A suffixed literal that does not fit its type is an error. <!-- block -->

```mote
let z = 300u8 // error: 300 does not fit UInt8
```

* A global that nothing reads is still initialized. <!-- block -->

```mote
fn note() -> Int {
    println("initialized")
    return 1
}

let unused = note()

fn main() {
    println("main")
}
```

```output
initialized
main
```

* A `spawn` that start-up runs cannot read a `var` global. <!-- block -->

```mote
var counter = 0

fn read() -> Int {
    return counter
}

let t = spawn read() // error: a task started at start-up cannot read the `var` global `counter`
```
