# signal: reference

Receive Unix signals as values, and send them. Native: Zig.

`import { Signal, subscribe, send, pid } from signal`

| Item | Notes |
|---|---|
| `Signal` | `Hangup Interrupt Quit User1 User2 Alarm Terminate Child Continue WindowChange`; `.number()` is the Linux number |
| `subscribe(list) -> Result<Subscription, Error>` | start watching; `Err` for an empty list |
| `Subscription.signals` | a `Receiver<Signal>`, signals in the order they were caught |
| `.next() -> Signal?` | waits; null once closed and read |
| `.close()` | stop watching; signals already caught are still read, then the receiver ends; a second close is an `Err` |
| `pid() -> Int` | this process |
| `send(process: Int, signal: Signal) -> Result<Null, Error>` | to any process, this one included; a signal to this program is handled before `send` returns, so none is reordered or merged |

* A signal that arrives while 64 KiB of them are waiting is lost, and several of one kind from another process may merge (the kernel keeps one pending signal of a kind).
* The signals go back to their defaults when no subscription watches them; while one does, the default action (such as ending the program on `Terminate`) does not happen.

```mote
import { Signal, pid, send, subscribe } from signal

fn main() -> Result<Null, Error> {
    let watch = subscribe([Signal.User1])?
    send(pid(), Signal.User1)?
    println(watch.next()!)
    watch.close()?
    return Ok(null)
}
```

To test by hand: `subscribe([Signal.Interrupt, Signal.Terminate])` in a loop on `next()`, then press Ctrl+C or run `kill -TERM <pid>`.

Tests: `tools/test-core.sh signal`.
