# std.meta

What a compile-time modifier reads and answers. A modifier is a `pub fn` marked `@modifier` that takes a `Decl` and returns `Result<Additions, Diagnostic>`. A trait can name its modifier with `@generate`, and the compiler runs it for each type that lists the trait. A modifier is also an ordinary function you can call and test.

```mote,skip
import std.meta
```

## Reading a declaration

A `Decl` describes one `struct`, `class` or `enum` as written. Types are not resolved: `List<Int>` is a `Generic` named `List` with one argument.

| Type | Holds |
|---|---|
| `Decl` | `kind` (`DeclKind`), `name`, `is_pub`, `doc`, `generics`, `traits`, `fields`, `variants`, `methods`, `attrs` |
| `Field` | `name`, `ty`, `is_pub`, `is_var`, `doc`, `default` |
| `Variant` | `name`, `shape` (`Shape.Unit`, `Tuple` or `Struct`), `types`, `fields`, `discriminant`, `doc`, `attrs` |

`doc` is the `///` lines before the item, joined by a line break, `""` when there are none. `Field.default` is the source text after `=`, `""` when there is none.
| `Method` | `name`, `is_pub`, `has_self`, `params`, `returns`: the signature of a method already there |
| `GenericParam` | `name`, `bounds` |
| `Attr` | `name`, `args`, `values` (the `key = "value"` pairs) |
| `TypeRef` | `kind` (`TypeKind`), `name`, `args` |

`TypeKind` is `Named`, `Optional`, `Generic`, `Tuple`, `Function`, `Array`, `Union`, `SelfType` or `Var`. `Optional`, `Array` and `Var` keep their element in `args`; `Function` keeps its parameters and then its result.

`render(t)` writes a `TypeRef` back as source.

## Answering

| Type | Holds |
|---|---|
| `Additions` | `public` and `private`: source text of `fn`s that become methods of the declaration |
| `Diagnostic` | `message`: why the modifier refuses the declaration |

`public` methods are callable from other modules. `private` ones are helpers, named `__…` by convention.

Every type has a `new` that builds it: `Additions.new(public, private = "")`, `Diagnostic.new(message)`, `Decl.new(kind, name, …)`, `Field.new(name, ty, …)`, and so on.

## A modifier

```mote
import std.meta as meta
import std.string as str

@modifier
pub fn describe(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {
    var parts: List<String> = []
    for f in d.fields {
        parts.push(f.name + ": " + meta.render(f.ty))
    }
    let body = "return \"" + d.name + "(" + str.join(parts, ", ") + ")\""
    return Ok(meta.Additions.new("fn describe(self) -> String { " + body + " }"))
}

fn main() {
    let x = meta.Field.new("x", meta.TypeRef.new(meta.TypeKind.Named, "Int"))
    let item = meta.TypeRef.new(meta.TypeKind.Named, "String")
    let tags = meta.Field.new("tags", meta.TypeRef.new(meta.TypeKind.Generic, "List", [item]))
    let decl = meta.Decl.new(meta.DeclKind.Struct, "Point", fields = [x, tags])
    match describe(decl) {
        Ok(a) => println(a.public)
        Err(d) => println(d.message)
    }
}
```

```output
fn describe(self) -> String { return "Point(x: Int, tags: List<String>)" }
```

The signature is checked where the function is written: a `@modifier` must be `pub`, take one `Decl` and return `Result<Additions, Diagnostic>`.

## Testing a modifier

A modifier is a function, so a `test` block builds a `Decl`, calls it and compares the text it answers.

```mote,skip
test "describe lists the fields" {
    let x = meta.Field.new("x", meta.TypeRef.new(meta.TypeKind.Named, "Int"))
    let decl = meta.Decl.new(meta.DeclKind.Struct, "P", fields = [x])
    let out = describe(decl)!
    assert_eq(out.public, "fn describe(self) -> String { return \"P(x: Int)\" }")
}
```

`parse_decl(text)` reads the JSON form of a declaration, the one the compiler hands a modifier.

## Supplying a trait

`@generate(path)` on a trait names the modifier that writes its methods. A bare name is a function in the trait's own module; a dotted path names a function in another module.

```mote,skip
@generate(label_modifier)
pub trait Labeled {
    fn label(self) -> String
}

struct Dog: (Labeled) {
}
```

| Rule | Effect |
|---|---|
| a type lists the trait and writes none of its methods | the modifier runs and its methods are added |
| a type writes the method itself | the modifier does not run |
| a member carries `@arg(…)` | an error, unless a modifier the type asked for says `@modifier(reads = "arg")` |
| the modifier answers `Err` | the error is reported where the trait is listed |
| `mote expand FILE` | shows the methods each trait wrote |

`Display`, `Debug`, `JsonCodec` (`std.json`) and `Args` (`std.args`) work this way: each names a function in `std.derive`. There is no `@derive`: listing the trait is the only way. Several names go in one string: `reads = "arg, other"`.

A modifier runs at compile time with no access to files, the network or the clock. It may import only `std.meta`, `std.string`, `std.iter`, `std.collections` and `std.math`, and it ends with an error after 20 million steps, 64 MiB of memory or 1 MiB of output. A module that holds a modifier cannot define `main` and cannot list a trait that has a modifier. Answers are cached in the build cache.
