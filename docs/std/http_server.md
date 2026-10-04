# std.sys.http_server

An HTTP/1.1 server over plain sockets. `import std.sys.http_server as server`. The program writes its own accept loop and starts a task per connection; a connection waiting for a request holds no thread.

```mote,skip
import std.sys.http_server as server

fn handle(conn: server.Connection) {
    var requests = conn.requests()
    while true {
        match requests.next() {
            Ok(next) => {
                match next {
                    Some(req) => {
                        let sent = req.respond(server.Response.text(200, "hello ${req.path}"))
                    }
                    None => { break }
                }
            }
            Err(e) => { break }
        }
    }
}

fn main() {
    let s = server.listen("0.0.0.0", 8080)!
    scope {
        while true {
            let conn = s.accept()!
            spawn { handle(conn) }
        }
    }
}
```

| Item | Does |
|---|---|
| `listen(host, port)`, `listen_tls(host, port, identity)` | `Result<Server, Error>` |
| `Server.accept()`, `local_port()`, `close()` | the next `Connection`; the bound port; stop listening |
| `Connection.requests()` | the request reader; make it in the task serving the connection |
| `Requests.next()` | `Result<Request?, Error>`; `None` when the client is done |

| `Request` member | Answers |
|---|---|
| `method`, `target`, `path` | as sent; the path is percent-decoded |
| `query()` | `Map<String, String>`, decoded; a repeated name keeps its last value |
| `header(name)`, `header_map()` | one header, or all by lowercase name |
| `body`, `text()`, `json()`, `form()` | the body as `Bytes`, text, `Json`, or a form map |
| `respond(reply)` | sends the reply |

A `Response` is built with `Response.new(status, body)`, `empty(status)`, `text(status, s)` or `json(status, value)`, and `reply.header(name, value)`. `respond` adds `date`, `content-length` and `connection`, and drops those headers if the program set them. A bad header, a status outside 100 to 999, or a `NaN` in JSON makes `respond` an `Err` before any byte is sent.

Limits are set on the server, so it must be a `var`: `set_max_header(bytes)` (64 KiB), `set_max_body(bytes)` (64 MiB), `set_idle_timeout(seconds)` (10). A request that cannot be served is answered with 400, 408, 413, 431 or 501, the connection closes, and `next()` answers an `Err`.

```mote
import std.sys.http_server as server
import std.sys.net as net

fn main() {
    let s = server.listen("127.0.0.1", 0)!
    let port = s.local_port()!
    scope {
        spawn {
            let c = net.connect("127.0.0.1", port)!
            let sent = c.write_text("GET /greet?name=zed HTTP/1.1\r\nconnection: close\r\n\r\n")
            let reply = c.read_to_end()!.decode().unwrap_or("")
            println(reply.split("\r\n\r\n").get(1))
        }
        let conn = s.accept()!
        var requests = conn.requests()
        match requests.next() {
            Ok(next) => {
                match next {
                    Some(req) => {
                        let who = req.query().get_or("name", "world")
                        let sent = req.respond(server.Response.text(200, "hello ${who}"))
                    }
                    None => {}
                }
            }
            Err(e) => {}
        }
    }
}
```

```output
hello zed
```

Not included: streaming bodies, a router, HTTP/2, WebSocket, static files, graceful shutdown.
