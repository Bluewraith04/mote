# std.error

`Error` and `ErrorKind` are in the prelude.

```mote,skip
pub enum ErrorKind {
    NotFound  PermissionDenied  InvalidData  Interrupted
    AlreadyExists  WouldBlock  TimedOut  Unsupported  Other
}

pub class Error {
    pub kind: ErrorKind
    pub message: String

    pub fn new(kind: ErrorKind, message: String) -> Error { … }
}
```

Build one with `Error.new(ErrorKind.InvalidData, "bad input")`. Every fallible operation in `std.sys`, `std.date`, `std.data` and `std.regex` answers a `Result` whose `Err` is an `Error`: read `kind` to decide, `message` to report.

```mote
import { read_file } from std.sys.io

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

## Per-domain enums

`io_error_of(e)` and `parse_error_of(e)` turn an `Error` into a narrower enum for matching:

| Function | Enum |
|---|---|
| `io_error_of(e) -> IoError` | `NotFound PermissionDenied AlreadyExists Interrupted WouldBlock TimedOut Other` |
| `parse_error_of(e) -> ParseError` | `InvalidSyntax Other` |

```mote,skip
import { parse_error_of, ParseError } from std.error
```
