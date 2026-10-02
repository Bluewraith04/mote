# std.sys.net

TCP and UDP sockets. `import std.sys.net as net`. Every operation answers a `Result` whose `Err` is an `Error`. A blocking call parks only the calling task, and a task waiting for data or a connection holds no thread. Sockets have `close()`, so `with s = net.connect(…)! { … }` closes one on every exit.

| Function | Answers |
|---|---|
| `connect(host, port)` | a `TcpStream` |
| `listen(host, port)` | a `TcpListener`; port `0` picks a free one |
| `bind(host, port)` | a `UdpSocket` |
| `connect_tls`, `listen_tls` | the same over [TLS](tls.md) |

| Type | Methods |
|---|---|
| `TcpListener` | `accept()`, `local_port()`, `close()` |
| `TcpStream` | `read(max)`, `read_to_end()`, `write(b)`, `write_text(s)`, `shutdown_write()`, `local_port()`, `peer_addr()`, `close()` |
| `UdpSocket` | `send_to(host, port, b)`, `recv_from(max)` (a `Datagram` with `bytes` and `sender`), `local_port()`, `close()` |

`read` answers up to `max` bytes, and empty once the peer finished sending.

```mote
import std.sys.net as net

let l = net.listen("127.0.0.1", 0)!
println(l.local_port()! > 0)
```

```output
true
```

An echo server runs one task per connection inside a `scope`.
