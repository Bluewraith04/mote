# Prelude

These names are in scope in every module with no import.

| Name | What |
|---|---|
| `print(v)`, `println(v)` | write any value to standard output |
| `typeof(v)` | the run-time type name as a `String` |
| `T?`, `Some`, `None` | the optional type, also spelled `Option<T>` |
| `Result<T, E>` | `Ok(T)` or `Err(E)` |
| `Error`, `ErrorKind` | the error type of every fallible std operation ([error](error.md)) |
| `Display`, `Debug`, `Eq`, `Ord`, `Closeable` | the std traits |

Every value has `.to_string()`, which `${…}` and `print` use. `.clone()` gives a mutable deep copy; strings, channels, tasks and `Shared` cells are shared instead.

| Trait | Required method |
|---|---|
| `Display` | `fn to_string(self) -> String` |
| `Debug` | `fn debug(self) -> String` |
| `Eq` | `fn eq(self, other: Self) -> Bool` |
| `Ord` | `fn compare(self, other: Self) -> Int`, giving `-1`, `0` or `1` |
| `Closeable` | `fn close(self) -> Result<Null, Error>`, used by `with` |

## Optional and Result methods

| Method | On | Result |
|---|---|---|
| `is_some()`, `is_none()` | optional | `Bool` |
| `is_ok()`, `is_err()` | `Result` | `Bool` |
| `unwrap()` | both | the payload; a fault on `None` or `Err`; the same as `x!` |
| `unwrap_or(d)`, `unwrap_or_else(f)` | both | the payload, else `d` or `f()` |
| `map(f)` | both | `f` applied to the payload |
| `and_then(f)` | both | `f(payload)`, itself an optional or `Result` |
| `ok_or(e)` | optional | `Ok(payload)` or `Err(e)` |
| `or(other)`, `or_else(f)` | `Result` | `self` if `Ok`, else `other` or `f()` |

A `Some` around a `None` stays distinct from `None`: `T??` is legal.

## Built-in type methods

| Type | Methods |
|---|---|
| `String` | `len is_empty concat replace slice to_upper to_lower trim repeat contains starts_with ends_with split bytes find rfind` |
| `List<T>` | `len is_empty push get get_or set contains pop first last clear iter stream` |
| `Map<K, V>` | `len is_empty get get_or set contains_key remove clear keys values entries` |
| `Set<T>` | `len is_empty add contains remove clear items` |
| `Bytes` | `len is_empty get get_or set push contains clear extend slice decode` |
| `Sender<T>` | `send close clone` |
| `Receiver<T>` | `recv iter stream clone` |
| `Stream<T>` | `next iter` |
| `Task<T>` | `join cancel is_ready` |
| `Shared<T>` | `get update set wait_until` |

`String.len` and `slice` count bytes. `find` and `rfind` answer a byte offset as an `Int?`. `List.get` and `Map.get` fault on a miss; use `get_or`. Methods that change a value are rejected on a `Shared` snapshot.
