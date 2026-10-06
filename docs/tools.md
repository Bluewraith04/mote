# Tools

## `mote test`

A `test "name" { … }` block is a top-level item. Inside it, `assert(cond)` and `assert_eq(a, b)` fail the test.

```mote,skip
fn divide(a: Int, b: Int) -> Result<Int, String> {
    if b == 0 { return Err("division by zero") }
    return Ok(a / b)
}

test "addition" {
    assert_eq(1 + 1, 2)
}

test "division by zero is an error" {
    assert(divide(1, 0).is_err())
}

@ignore
test "not ready yet" {
    assert_eq(divide(8, 2)!, 5)
}
```

| Command | Runs |
|---|---|
| `mote test main.mote` | every test in the file |
| `mote test` | the tests of the current package: those in the modules its entry imports, and in the `.mote` files directly under its `tests/` directory |
| `mote test --filter zero` | tests whose name matches the [regex](std/regex.md) `zero` (unanchored; a bad pattern exits with `2`) |
| `mote test --schedule-seed 7` | the tests on one worker, with task switches chosen by the seed |

Each test runs as its own task and is joined before the next, so a fault aborts only that test. `mote run` and `mote check` compile a file with `test` blocks and skip them. `@ignore` reports a test as ignored; `@test` marks an existing parameterless function as a test.

```text
  ok       addition
  FAIL     division by zero is an error: assertion failed
  ignored  not ready yet
1 passed, 1 failed, 1 ignored
```

The exit code is `0` when nothing failed and `1` otherwise. A failing `--schedule-seed` run prints `schedule seed: n`; the same seed replays the same switches.

A library keeps its tests in `tests/`, where they do not ship with it. A test file reaches the package's own modules by a relative path (`import ..src.lib as shapes`) and its dependencies by name once `mote sync` has run. `mote test` in the package directory runs every file in `tests/` with the tests the entry imports, and grants a package that carries a native library to its own tests.

[`std.test`](std/test.md) has a `Suite` for tests written as ordinary code.

## `mote check --memory`

Type-checks a program and prints where each struct literal in your modules lives. See [Memory](memory.md#seeing-the-choice).

## `--allow-native`

`mote run --allow-native main.mote` lets [`std.dev.libtools`](std/libtools.md) load C libraries; without it `open` answers `Err`. A program built with `mote build` takes no flags, so set `MOTE_ALLOW_NATIVE=1`.
