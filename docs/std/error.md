# std.error

`Error` and `ErrorKind` are in the prelude.

```mote,skip
pub enum ErrorKind {
    NotFound  PermissionDenied  InvalidData  Interrupted
    AlreadyExists  WouldBlock  TimedOut  Unsupported  Other
    Unauthenticated  Conflict  RateLimited  Unavailable
}

pub class Error {
    pub kind: ErrorKind
    pub message: String
    pub code: String

    pub fn new(kind: ErrorKind, message: String) -> Error { … }
    pub fn with_code(self, code: String) -> Error { … }
}
```

Build one with `Error.new(ErrorKind.InvalidData, "bad input")`. Every fallible operation in `std.fs`, `std.date`, `std.json` and `std.regex` answers a `Result` whose `Err` is an `Error`: read `kind` to decide, `message` to report.

```mote
import { read_file } from std.io

match read_file("missing.txt") {
    Ok(text) => println(text)
    Err(e) => {
        match e.kind {
            NotFound => println("no such file")
            _ => println(e.message)
        }
    }
}
```

```output
no such file
```

## Context

`e.context(what)` answers the same error with `what` in front of its message; the kind is unchanged. `context(r, what)` from `std.error` does that to the `Err` of a `Result` and passes an `Ok` through.

```mote
import { context } from std.error
import { read_file } from std.io

fn load(path: String) -> Result<String, Error> {
    return Ok(context(read_file(path), "loading ${path}")?)
}

match load("missing.txt") {
    Ok(text) => println(text)
    Err(e) => println(e.message)
}
```

```output
loading missing.txt: read_file: missing.txt
```

## Codes

`code` is `""` unless set. `e.with_code("name_taken")` answers the same error with a code a program can branch on; `context` keeps it. `message` is for a person.

```mote
let e = Error.new(ErrorKind.Conflict, "taken").with_code("name_taken").context("signup")
println(e.message)
println(e.code)
```

```output
signup: taken
name_taken
```

## Your own errors

A type that implements `Failure` becomes an `Error` at `?` in a function that returns `Result<_, Error>`.

| Member | Does |
|---|---|
| `fn into_error(self) -> Error` | the `Error` a `?` returns for this value |

```mote
import { Failure } from std.error

enum AuthError: (Failure) {
    Missing
    Expired(String)

    pub fn into_error(self) -> Error {
        match self {
            Missing => { return Error.new(ErrorKind.Unauthenticated, "no token").with_code("auth_missing") }
            Expired(who) => { return Error.new(ErrorKind.Unauthenticated, "expired for " + who).with_code("auth_expired") }
        }
    }
}

fn check(n: Int) -> Result<Int, AuthError> {
    if n == 0 { return Err(AuthError.Missing) }
    return Ok(n)
}

fn run(n: Int) -> Result<Int, Error> {
    let v = check(n)?
    return Ok(v + 1)
}

match run(0) {
    Ok(v) => println(v)
    Err(e) => println("${e.message} ${e.code}")
}
```

```output
no token auth_missing
```

## Per-domain enums

`io_error_of(e)` and `parse_error_of(e)` turn an `Error` into a narrower enum for matching:

| Function | Enum |
|---|---|
| `io_error_of(e) -> IoError` | `NotFound PermissionDenied AlreadyExists Interrupted WouldBlock TimedOut Other` |
| `parse_error_of(e) -> ParseError` | `InvalidSyntax Other` |

```mote,skip
import { parse_error_of, ParseError } from std.error
```
