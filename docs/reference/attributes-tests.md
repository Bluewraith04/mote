# Attributes, docs and tests

## Attributes

* `@name` or `@name(arg, …)` precedes an item. The set is closed: an unknown name is an error. <!-- test: tests_and_attributes, an_attribute_the_compiler_does_not_define_is_unknown_on_an_item_and_on_a_variant, attributes_are_a_closed_set, the_attributes_the_compiler_defines_are_these_and_no_others -->
<!-- table: attributes -->
* The attributes are `@blocking`, `@stable` and `@ignore`.
<!-- /table -->

<!-- test: extern_blocks_record_their_link, blocking_belongs_on_extern_functions, attributes_are_a_closed_set -->
| Attribute | Effect |
|---|---|
| `@blocking` | on an `extern` fn: marks a call that may wait |
| `@stable` | marks an item of `std`; an error elsewhere |
| `@ignore` | on a `test` block: skips it |

* An argument is a bare name, a string, a number or `true`/`false`, alone or as `key = value`. <!-- test: tests_and_attributes -->
* `@blocking` goes on an `extern` fn only, `@ignore` on a `test` only, and `@stable` in `std` only. <!-- block -->

```mote
extern "c" {
    fn puts(text: String) -> Int
}

@ignore
fn plain() {} // error: `@ignore` goes on a test
```

* Before a field or a method an attribute is a syntax error. <!-- block -->

```mote
struct Point {
    @blocking // error: expected a name
    x: Int
}
```

* There is no `@derive`; list the trait instead. See [derive.md](derive.md). <!-- block -->

```mote
@derive(Debug) // error: unknown attribute `@derive`
struct Point {
    x: Int
}
```

## Doc comments

* `///` alone on a line documents the next struct, class, enum, variant, field, method, trait or function. <!-- test: docs_attach_to_the_item_below, docs_attach_to_fields_variants_and_methods -->

```mote
/// A point on the grid.
struct Point {
    /// Distance from the left edge.
    x: Int
    y: Int
}
```

* `//`, `/* */`, a line of `////` or more and a `///` after code on a line are plain comments. <!-- test: comments_are_kept_outside_the_tokens -->
* A `///` before anything else is a warning: it documents nothing. <!-- test: docs_attach_to_the_item_below, a_doc_before_a_statement_is_a_warning -->

## Field defaults

* `name: Type = expr` fills the field when a literal omits it. The expression is checked against the type and evaluated each time a literal uses it. <!-- block -->

```mote
class Point {
    x: Int = 0
    y: Int = 2 * 3
    var tags: List<String> = []
}

fn main() {
    let a = Point { x: 9 }
    var b = Point {}
    b.tags.push("t")
    println("${a.x} ${a.y} ${a.tags.len()} ${b.tags.len()}")
}
```

```output
9 6 0 1
```

* A default sees the names of its module, not `self` or the other fields. <!-- block -->

```mote
struct Other {
    w: Int = 1
    h: Int = self.w // error: `self` is only inside a method
}

let o = Other {}
```

* A name in a default that is a field's name is the module's name, not the field. <!-- block -->

```mote
let w = 5

struct Size {
    w: Int = 1
    h: Int = w
}

fn main() {
    println(Size { w: 3 }.h)
}
```

```output
5
```

## `test`

* `test "name" { … }` at the top level is a test, checked like a function. <!-- test: test_blocks_are_checked_like_functions, differential_functions_a_test_block_compiled_gives_its_verdict_and_output -->
* `mote test` runs every test in the package and reports each. <!-- test: run_tests_lists_one_line_per_block_with_ignored_ones_skipped_and_a_count -->

```mote
import assert_eq from std.test

test "addition" {
    assert_eq(1 + 1, 2)
}
```

* `@ignore` skips one: the report says `skipped`. <!-- test: differential_functions_a_test_block_compiled_gives_its_verdict_and_output, run_tests_lists_one_line_per_block_with_ignored_ones_skipped_and_a_count -->

```mote
@ignore
test "slow" { }
```

## Reports

* `mote test` prints one line per test, `test NAME (file:line) ... ok`, `FAILED` or `skipped`, and a last line `test result: ok. 1 passed; 0 failed; 0 skipped`. <!-- test: run_tests_lists_one_line_per_block_with_ignored_ones_skipped_and_a_count, run_tests_a_run_where_every_block_passes_has_code_0 -->
* A failing test is listed after the lines with what it printed; a passing test's output is not shown. <!-- test: run_tests_prints_what_a_failing_block_printed_and_not_what_a_passing_one_did -->
* The exit code is `0` when no test failed and `1` otherwise; a package with no tests is a run of zero with code `0`. <!-- test: run_tests_a_run_where_every_block_passes_has_code_0, run_tests_lists_one_line_per_block_with_ignored_ones_skipped_and_a_count, run_tests_with_no_blocks_is_a_run_of_zero_with_code_0 -->
* What start-up prints comes first, and a fault in start-up runs no test and has code `1`. <!-- test: run_tests_start_up_output_comes_first, run_tests_a_fault_while_starting_up_runs_no_block_and_has_code_1, differential_functions_a_test_block_compiled_gives_its_verdict_and_output -->
* Each test has its own output. <!-- test: differential_functions_a_test_block_compiled_gives_its_verdict_and_output -->

## Assertions

<!-- test: assertions_are_calls_in_a_test_block, iter_string_math_and_test_modules_work_together, differential_functions_a_test_block_compiled_gives_its_verdict_and_output -->
| Call | Records a failure when |
|---|---|
| `assert(cond)` `assert(cond, msg)` | `cond` is false |
| `assert_eq(a, b)` `assert_ne(a, b)` | the values are unequal or equal; both are the same type |
| `fail(msg)` | always |

* The assertions come from `std.test`. <!-- test: iter_string_math_and_test_modules_work_together -->
* A failing assertion ends that test only. A fault inside a test fails it. <!-- test: differential_functions_a_test_block_compiled_gives_its_verdict_and_output, run_tests_a_block_that_faults_fails_only_itself -->
