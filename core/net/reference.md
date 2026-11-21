# net: reference

TCP, UDP, TLS, HTTP/1.1 and WebSocket in one library. Native: Zig (`net.socket`) and a Rust crate (`net.tls`, rustls with bundled public roots). HTTP and WebSocket are Mote.

| Import | Gives |
|---|---|
| `import { TcpListener, TcpStream, UdpSocket, Datagram } from net.socket` | sockets |
| `import { TlsAcceptor, TlsStream } from net.tls` | TLS 1.2 and 1.3 |
| `import { Server, Request, Response, Options, Url, get, post, send, connect_websocket, parse_url, url_encode, url_decode } from net.http` | HTTP |
| `import { WebSocket, Message } from net.websocket` | WebSocket frames |

`import net` also works: `net.socket.TcpStream`, `net.http.get`, ...

## Run it yourself

```bash
mote run
```

Run it in a package whose `src/main.mote` imports `net`. An installed `mote` carries the natives and unpacks them into `~/.mote/core/` the first time; nothing else to do. When editing the library in the repository, use `mote run --core <repo>/core` after `tools/build-core-natives.sh net json`. `tools/test-core.sh net` runs the library's own tests (`-- NAME` filters by test name).

A server and a client in one program (verified):

```mote
import { Mutex } from std.sync
import { TcpListener } from net.socket
import { Request, Response, Server, get } from net.http

fn app(state: Mutex<Int>, request: Request) -> Response {
    let user = request.route("GET", "/hello/:name")
    if user != null { return Response.text(200, "hello ${user[0]}") }
    return Response.text(404, "no such path")
}

fn main() -> Result<Null, Error> {
    let listener = TcpListener.bind("127.0.0.1", 0)?
    let port = listener.port()?
    let server = Server.new(0, app)
    let running = spawn {
        server.serve(listener)!
        return 0
    }
    let reply = get("http://127.0.0.1:${port}/hello/mote")?
    println("${reply.status} ${reply.body_text()!}")
    server.stop()
    running.await()!
    return Ok(null)
}
```

```output
200 hello mote
```

To browse your own server, bind `"127.0.0.1", 8080`, keep `main` waiting (for example `running.await()!` without `server.stop()`), and open `http://127.0.0.1:8080/hello/you`.

## Behaviour that holds for every socket

* Every call that can wait is `@blocking`: it parks the calling task and runs on a helper thread, so other tasks keep running. Cost: one thread per blocked call (not planned: a netpoller).
* A descriptor and its time limit are atomics: `close()` is `Closeable`, a second `close()` is an `Err`, and a closed socket's `descriptor()` is -1.
* A time limit is `set_timeout(ms)` on a stream, a listener (applies to `accept`) or a UDP socket (applies to `recv_from`); a negative time waits for ever (the default). A time that runs out is `ErrorKind.TimedOut`.
* Error kinds: `TimedOut` (limit or errno 110), `NotFound` (a name that does not resolve), `PermissionDenied` (13), `AlreadyExists` (98, address in use), `InvalidData` (22), `Interrupted` (4), otherwise `Other`. The message is `what: reason` (`connection refused`, `connection reset`, `broken pipe`, `network unreachable`, ...).
* A host is a name or an address; IPv6 in text is `::1` (in a URL `[::1]`). An empty host to `bind` is every address; port 0 picks a free one (`port()` reads it).

## net.socket

| Item | Signature | Notes |
|---|---|---|
| `TcpStream.connect` | `(host: String, port: Int) -> Result<TcpStream, Error>` | waits as long as the system does |
| `TcpStream.connect_within` | `(host, port, timeout_ms: Int)` | the limit is per address tried; sets `TCP_NODELAY` |
| `TcpStream.from_fd` | `(fd: Int) -> TcpStream` | owns the descriptor |
| `.descriptor()` | `-> Int` | -1 once closed |
| `.set_timeout(ms)` | | each read and write |
| `.read_into(buffer: [UInt8])` | `-> Result<Int, Error>` | bytes read, as soon as some arrive; 0 at the end of the stream |
| `.read(count)` | `-> Result<[UInt8], Error>` | up to `count`; empty at the end |
| `.read_all()` | `-> Result<[UInt8], Error>` | until the peer closes its side |
| `.write(bytes)` / `.write_string(text)` | `-> Result<Null, Error>` | writes all of it |
| `.shutdown_write()` | | no more bytes come from this side; reading still works |
| `.peer_addr()` / `.local_addr()` | `-> Result<String, Error>` | `127.0.0.1:80`; IPv6 `[::1]:80` |
| `.close()` | | |
| `TcpListener.bind` | `(host, port) -> Result<TcpListener, Error>` | |
| `.port()`, `.set_timeout(ms)` | | |
| `.accept()` | `-> Result<TcpStream, Error>` | `TimedOut` when the limit passes |
| `UdpSocket.bind` | `(host, port)` | |
| `.send_to(host, port, bytes)` | `-> Result<Int, Error>` | bytes sent |
| `.recv_from(buffer)` | `-> Result<Datagram, Error>` | `Datagram { size: Int, sender: String }`; longer than the buffer is cut |

Echo over TCP and UDP:

```mote
import { TcpListener, TcpStream, UdpSocket } from net.socket

fn main() -> Result<Null, Error> {
    let listener = TcpListener.bind("127.0.0.1", 0)?
    let port = listener.port()?
    let echo = spawn {
        let peer = listener.accept()!
        let bytes = peer.read(64)!
        peer.write(bytes)!
        peer.close()!
        return 0
    }
    let stream = TcpStream.connect("127.0.0.1", port)?
    stream.write_string("ping")?
    println(String.from_bytes(stream.read(64)?)!)
    stream.close()?
    echo.await()!

    let server = UdpSocket.bind("127.0.0.1", 0)?
    let client = UdpSocket.bind("127.0.0.1", 0)?
    client.send_to("127.0.0.1", server.port()?, "hello".bytes())?
    let buffer: [UInt8] = [0; 32]
    let got = server.recv_from(buffer)?
    println("${got.size} bytes from ${got.sender}")
    return Ok(null)
}
```

## net.tls

| Item | Signature | Notes |
|---|---|---|
| `TlsStream.connect` | `(host, port) -> Result<TlsStream, Error>` | handshake; trusts the bundled public roots; the certificate is checked against `host` |
| `TlsStream.connect_trusting` | `(host, port, roots: String)` | also trusts the PEM certificates in `roots` (a private CA) |
| `TlsStream.client` | `(tcp: TcpStream, host, roots)` | handshake over a connected stream; closes the stream if it fails |
| `.set_timeout(ms)` | | handshake, each read and each write |
| `.read_into`, `.read`, `.read_all`, `.write`, `.write_string`, `.peer_addr`, `.local_addr` | as `TcpStream` | a peer that closes without a close notice reads as the end of the stream |
| `.close()` | | sends the close notice, closes the socket |
| `TlsAcceptor.new` | `(chain: String, key: String) -> Result<TlsAcceptor, Error>` | PEM chain and private key; checked at once |
| `.accept(tcp)` | `-> Result<TlsStream, Error>` | server handshake |

One task at a time uses a `TlsStream`.

## net.http

### Client

| Item | Signature | Notes |
|---|---|---|
| `get(address, options = Options())` | `-> Result<Response, Error>` | |
| `post(address, body: [UInt8], content_type: String, options = Options())` | `-> Result<Response, Error>` | |
| `send(method, address, headers: [(String, String)] = [], body: [UInt8] = [], options = Options())` | `-> Result<Response, Error>` | `headers` are `(name, value)` pairs, as in `[("x-token", "abc")]`; reads the whole response |
| `Options { timeout_ms, max_redirects, roots, max_body }` | struct | every field has a default, so `Options()` and `Options { max_redirects: 5 }` work: `timeout_ms: 30000, max_redirects: 0, roots: "", max_body: 67108864`; `roots` is extra trusted PEM text |
| `connect_websocket(address, options = Options())` | `-> Result<WebSocket, Error>` | `ws://` or `wss://`; the socket then waits for ever |
| `parse_url(text)` | `-> Result<Url, Error>` | `Url { secure, host, port, target }`; schemes `http https ws wss`; port defaults 80 or 443; `target` is path and query, `#fragment` dropped; `Err` for another scheme, no scheme or a port above 65535 |
| `url_encode(text)` / `url_decode(text)` | `-> String` | everything but letters, digits and `-._~` becomes `%XX`; a bad escape leaves the text as it is |

* The client sends `host`, `connection: close`, `user-agent: mote` and `accept: */*` unless you set them, and `content-length` for a body or POST, PUT and PATCH. A new connection for every request; no keep-alive, no cookies, no compression.
* Redirects: followed up to `max_redirects` (0 follows none); a 303, and a 301 or 302 after a POST, become a GET with no body. A relative `Location` is resolved against the request.
* A response body is read by `content-length`, by chunked encoding, or to the end of the connection; HEAD, 204 and 304 have none. More than `max_body` is an `Err`.

### Server

| Item | Signature | Notes |
|---|---|---|
| `Server.new(state: S, handler)` | `handler: Send (Mutex<S>, Request) -> Response` | `S` is shared through a `Mutex`; use `state.lock(|s| { ... })` |
| `.with_sockets(f)` | `f: Send (Mutex<S>, Request, var WebSocket) -> Null` | needed for upgrades |
| `.with_tls(acceptor)` | | every accepted connection speaks TLS |
| `.with_limits(timeout_ms, max_body)` | | defaults 30 s and 8 MiB |
| `.serve(listener)` | `-> Result<Null, Error>` | one task per connection; sets the listener limit to 100 ms to notice `stop`; returns on `stop` |
| `.stop()` | | `serve` returns at the next 100 ms tick; requests in progress finish |

The handler runs in the connection's task. A connection is kept alive unless the request (HTTP/1.1 default, or `keep-alive` on 1.0) or the response (`connection: close`) says otherwise. HEAD is answered by the GET handler with no body. `expect: 100-continue` is answered. The server writes `content-length` and `connection`; any `transfer-encoding` header you set is dropped. A malformed request gets 400, a body over the limit 413, too many headers 431, then the connection closes. No chunked responses, no request streaming.

`Request` (all `pub`): `method`, `target`, `path`, `query`, `version`, `headers: [(String, String)]` (names lower case), `body: [UInt8]`, `peer`.

| `Request` method | Gives |
|---|---|
| `header(name) -> String?` | by any case |
| `query_param(name) -> String?` | decoded; `a=1&b` gives `""` for `b` |
| `segments() -> [String]` | path split at `/`, each part decoded |
| `route(method, pattern) -> [String]?` | `route("GET", "/users/:id")` gives the `:name` parts in order, or null if the method or shape differs |
| `body_text() -> String?`, `body_json() -> Result<Json, Error>` | |

`Response` (`status`, `headers`, `body`, `upgrade`): `bytes(status, content_type, body)`, `text(status, text)`, `html(status, text)`, `json(status, value: Json)`, `empty(status)`, `redirect(location)` (302), `upgrade_to_websocket()`, `.with_header(name, value)`, `.header(name)`, `.body_text()`, `.body_json()`.

Routing pattern in the handler: `let user = request.route("GET", "/users/:id"); if user != null { ... user[0] ... }`; return `Response.text(404, ...)` last.

## net.websocket

| Item | Signature | Notes |
|---|---|---|
| `Message` | `Text(String)` or `Binary([UInt8])` | |
| `socket.recv()` | `-> Result<Message?, Error>` | null once the peer closed; pings are answered, pongs skipped |
| `socket.send_text(text)` / `.send_binary(bytes)` | `-> Result<Null, Error>` | |
| `socket.ping(bytes)` | | up to 125 bytes |
| `socket.close(code, reason)` | | 1000 is normal; closes the stream |
| `socket.set_timeout(ms)` | | |

Methods take `var self`, so a socket is a `var`. A message over 16 MiB, a frame not well formed or an incomplete message closes the socket with code 1009 or a protocol error and an `Err`. One task at a time per socket. Server side:

```mote
import { Mutex } from std.sync
import { Message, WebSocket } from net.websocket
import { Request, Response } from net.http

fn handler(state: Mutex<Int>, request: Request) -> Response {
    if request.route("GET", "/ws") != null { return Response.upgrade_to_websocket() }
    return Response.text(404, "no such path")
}

fn echo(state: Mutex<Int>, request: Request, var socket: WebSocket) -> Null {
    while true {
        let message = socket.recv()
        if message.is_err() { break }
        let got = message!
        if got == null { break }
        match got {
            Message.Text(text) => socket.send_text("echo:" + text)
            Message.Binary(bytes) => socket.send_binary(bytes)
        }
    }
    return null
}
```

Wire it with `Server.new(0, handler).with_sockets(echo)` and talk to it (verified; prints `echo:hi`):

```mote
fn main() -> Result<Null, Error> {
    let listener = TcpListener.bind("127.0.0.1", 0)?
    let port = listener.port()?
    let server = Server.new(0, handler).with_sockets(echo)
    let running = spawn {
        server.serve(listener)!
        return 0
    }
    var socket = connect_websocket("ws://127.0.0.1:${port}/ws")?
    socket.send_text("hi")?
    let reply = socket.recv()?
    match reply! {
        Message.Text(text) => println(text)
        Message.Binary(bytes) => println(bytes.len())
    }
    socket.close(1000, "done")?
    server.stop()
    running.await()!
    return Ok(null)
}
```

The imports are the ones above plus `TcpListener` from `net.socket` and `Server`, `connect_websocket` from `net.http`.

## Not there

Keep-alive and pooling in the client, cookies, HTTP/2, compression, chunked responses from the server, client certificates, a netpoller, unix sockets, multicast, WebSocket extensions. These are in `tasks.md` under "Not planned".

## Tests

`tools/test-core.sh net` (socket, tls, http, websocket, keys, modules); `cargo test -p mote-integration --test core_tools -- core_net`.
