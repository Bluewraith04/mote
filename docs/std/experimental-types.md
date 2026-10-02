# std.experimental.types

```mote
import { Any } from std.experimental.types
```

`Any` is a value of any type, checked only at run time. It is not in scope until imported in each file that names it.

Reach for a better type first:

| You have | Write |
|---|---|
| a value that is one of several types | a union: `Map<String, Int \| String>` |
| a value of any one type, chosen by the caller | a generic: `fn first<T>(xs: List<T>) -> T?` |
| a value that must do something | a bound: `fn show<T: Display>(x: T)` |

A value out of an `Any` is checked where it goes into a typed place. The standard library names `Any` nowhere in its signatures.
