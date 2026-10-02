# Memory

A Mote program never frees memory by hand. There is no lifetime syntax and no borrow checker; the compiler picks where each value lives.

## Owners

| Owner | Freed when |
|---|---|
| Registers | the function returns |
| A block's region | the block ends, by any path |
| The heap | the collector finds the value unreachable |

The heap is the owner of last resort, so a placement the compiler cannot prove costs speed, never correctness. Lists, maps, classes, strings and closures are always on the heap; only struct values get the cheaper owners.

A small struct lives in registers when it has at most 8 fields, none of them a struct, is built by a function with a `let` or `var`, and is only read and written through its fields. A struct literal bound by `let` or `var` lives in a region when every use leaves it behind: field access, copying, comparing, printing, or passing it to a function whose body does not keep it. A function keeps a parameter by storing it in a list, capturing it in a lambda, passing it to a call that keeps it, or reassigning it.

A value may point to a value of its own owner or an outer one, never an inner one. The compiler keeps region values from escaping, and the runtime checks each store as a second line. A value sent to another task is copied onto the heap first.

## Seeing the choice

```mote
struct Point {
    var x: Int
    var y: Int
}

fn keep(p: Point, var into: List<Point>) {
    into.push(p)
}

fn main() {
    var list: List<Point> = []
    var a = Point { x: 1, y: 2 }
    a.x += 1
    println(a.x)
    var b = Point { x: 3, y: 4 }
    keep(b, list)
    println(Point { x: 5, y: 6 }.x)
}
```

```output
2
5
```

`mote check --memory main.mote` prints one line per struct literal in your own modules:

```text
main.mote:12:13  Point { .. }  registers, no memory
main.mote:15:13  Point { .. }  heap: `b` is passed at 16:10 to a call that may keep it
main.mote:17:13  Point { .. }  heap: not the initializer of a let or var
3 struct literal(s): 1 in registers, 0 in regions, 2 on the heap.
```

A struct goes on the heap when the name is stored whole, passed to a call that may keep it (including through a function value, a generic function or a built-in), used in a lambda, declared twice in a function, or built in place (`f(T { .. })`, `T { .. }.x`) or at module level.

## The collector

The heap is collected by a full mark-sweep pass, started once the program has allocated as much as the live heap holds, and at least 4 MiB. `mote run --gc nogc` turns collection off.

## Limits

A program that outgrows a limit stops with a fault; the operating system does not kill it.

| Limit | Default | Change |
|---|---|---|
| heap | half of physical memory, or of the container limit | `--max-heap 2GiB`, `MOTE_MAX_HEAP=2GiB`, or `[run]` `max-heap` in `mote.toml`; `unlimited` removes it |
| stack, per task | 64 MiB | `MOTE_MAX_STACK=16MiB` |

The flag wins over the environment, which wins over `mote.toml`. Heap exhaustion faults with `out of memory: heap limit 2.0 GiB reached (live 2.0 GiB)` and runaway recursion with `stack overflow: N calls deep`. A collection that falls to a quarter full shrinks, and a 1 MiB slab that stays free across two collections goes back to the operating system. `mote run --mem-stats` prints what a run used.
