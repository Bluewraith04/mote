# Reference

## Lexical rules

| Item | Rule |
|---|---|
| Comments | `// …` and nestable `/* … */`; `///` is an ordinary line comment |
| Identifiers | `[a-zA-Z_][a-zA-Z0-9_]*`; a lone `_` is the pattern wildcard |
| Statement end | a newline, when the previous token can end a statement; newlines inside `( )`, `[ ]`, `{ }` are ignored |

Keywords:

| Group | Keywords |
|---|---|
| declarations | `let` `var` `fn` `struct` `class` `enum` `trait` `type` `pub` |
| modules | `import` `from` `as` `super` |
| control | `if` `else` `match` `while` `for` `in` `break` `continue` `return` `yield` |
| concurrency | `scope` `spawn` |
| values | `self` `Self` `true` `false` `null` `is` |

`impl`, `mut`, `try` and `native` are also reserved. A keyword cannot be a name, which is why the standard library has `get_var` and not `var`. `new` is not a keyword; constructors are ordinary static methods.

Not supported: raw strings, the `\xHH` escape, uppercase radix prefixes (`0XFF`), and numeric type suffixes.

## Operators

From tightest to loosest:

| Level | Operators |
|---|---|
| postfix | `.` `?.` `( )` `[ ]` `!` `?` |
| prefix | `-` `!` `~` |
| range | `..` `..=` |
| multiplicative | `*` `/` `%` |
| additive | `+` `-` |
| shift | `<<` `>>` |
| type test | `is` |
| comparison | `<` `<=` `>` `>=` |
| equality | `==` `!=` |
| bit and, xor, or | `&`, `^`, `\|` |
| logical and, or | `&&`, `\|\|` |
| null-coalescing | `??` |
| conditional | `? :` |

| Rule | Detail |
|---|---|
| `Int` division | truncates toward zero; `-7 / 2` is `-3` |
| `%` | takes the sign of the left operand; `-7 % 3` is `-1` |
| Division by zero | a runtime error |
| `Float` | IEEE 754 |
| Operand types | must match; `1 + 1.5` is an error |
| Optional or union operand | must be narrowed first |
| `&&`, `\|\|` | short-circuit |
| Indexing | `xs[i]` is `xs.get(i)`; out of range or a missing key is a fault; no negative indexing |
| `c ? x : y` | `c` must be `Bool`; only the chosen branch runs; a `null` branch makes the result optional |

`==` compares scalars and strings by value, structs field by field, lists, tuples, maps and sets by content, optionals and enum variants by variant then payload, and class instances by identity.

`e is T` tests the run-time type as a whole: `[1] is List<Int>` is true and `[1] is List<Any>` is false. An enum variant is not a type.

There is no cast operator. Convert with the target type's `from`: `Float.from(3)`, `Int.from(2.7)`, `String.from(x)`.

```mote
let config_port: Int? = None
var cache: Int? = None
let port = config_port ?? 8080
cache ??= 7
println(port)
println(cache)
```

```output
8080
7
```

## Statements

| Statement | Meaning |
|---|---|
| `let x = e`, `var x = e` | a binding; the initializer is mandatory |
| `let (a, b) = t` | destructures a flat tuple |
| `place = e`, `place op= e` | assignment to a `var` name, a `var` field or an index |
| `if`, `while`, `for`, `match` | see [Basics](basics.md) |
| `return`, `break`, `continue`, `yield` | leave a function, leave a loop, next pass, produce a stream element |
| `scope { }` | wait for the tasks spawned inside |
| `with name = expr { }` | calls `.close()` on the resource on every exit from the block |

`with` takes any type with a `close(self)` method; `File`, `TcpStream`, `TcpListener` and `UdpSocket` qualify. It closes on falling off the end, `return`, `break`, `continue` and `?`.

```mote,skip
with f = fs.open("x.txt")! {
    println(f.read(64)!)
}
```

## Attributes

`@name` or `@name(arg, …)` precedes a declaration. The set is closed: an unknown name is a compile error.

| Attribute | Effect |
|---|---|
| `@derive(Display)`, `@derive(Debug)` | adds `to_string`, `debug` |
| `@derive(Json)` | adds `to_json` and `from_json` ([std.data.json](std/json.md#records)) |
| `@derive(Args)` | adds `parse`, `parse_or_exit`, `usage` ([std.dev.args](std/args.md)) |
| `@arg(…)` | describes a field or variant for `Args` |
| `@test`, `@ignore` | mark a function for `mote test`, skip a test |
| `@stable` | marks a `pub` item inside `std` as part of its checked API |

## Known gaps

These parse or look like they should work, but do not.

| Form | Behaviour |
|---|---|
| `if` or `match` as an expression | does not parse; use `c ? x : y` |
| `f(x: 1)` named arguments | do not parse |
| a nested `fn` inside a function | does not parse; use a lambda |
| nested tuple destructuring (`let (a, (b, c)) = t`), tuple destructuring at top level | not supported; a flat `let (a, b) = t` works inside a function |
| a pattern inside a variant's payload (`Ok(Some(x))`) | the payload takes names or `_`; nest a `match` |
| a bound on a method, or on a type's own parameters | compile error |
| `P { x: 1 }` where `type P = Point` | compile error; write `Point { x: 1 }` |
| a variadic function, or one with defaults, as a value | calling it through the value fails at run time; wrap it in a lambda |
| a range as a value (`let r = 1..3`) | not supported; ranges appear in `for` and patterns |
| `Type { x, y }` against a struct | parses, but codegen rejects it; match an enum, a tuple or a literal |
| a trait with type parameters, or extending another trait | compile or parse error |
| `s[i]` on a `String` | by design; use `.bytes()`, `.find`, `.slice` |
| trait objects, inheritance | none |
