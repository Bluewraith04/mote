# wire

Networking for Mote: a `Server<S>` application and an HTTP and HTTPS client. The client is over the `ureq` crate, WebSocket framing is the `tungstenite` crate's, and both are carried as a library in `native/<triple>/`. Server-sent events, WebSocket rooms and a WebSocket client are not included.

| Step | Line |
|---|---|
| add it | nothing: it ships with Mote, and its library is copied beside a built program |
| use it | `import wire` |
| build the library | `tools/build-core-natives.sh wire` in the Mote repository, which puts it in `native/<triple>/` for this machine |

The example is an application whose handlers read its state, call a plain function for a dependency, and are tried with no socket and then over HTTP.

```mote
import wire

struct Deps {
    greeting: String
    hits: Shared<Int>
}

fn greet(req: wire.Request<Deps>) -> Result<wire.Reply, Error> {
    var hits = req.state.hits.clone()
    hits.update(|n| { n += 1 })
    return Ok(wire.Reply.text(200, "${req.state.greeting} #${req.state.hits.get()}"))
}

fn current_user(req: wire.Request<Deps>) -> Result<String, Error> {
    match req.header("x-user") {
        Some(name) => { return Ok(name) }
        None => { return Err(Error.new(ErrorKind.PermissionDenied, "sign in first")) }
    }
}

fn me(req: wire.Request<Deps>) -> Result<wire.Reply, Error> {
    let name = current_user(req)?
    return Ok(wire.Reply.text(200, "${req.state.greeting}, ${name}"))
}

fn show(req: wire.Request<Deps>) -> Result<wire.Reply, Error> {
    let id = req.param_int("id")?
    return Ok(wire.Reply.text(200, "user ${id}"))
}

fn main() {
    var app = wire.Server(Deps { greeting: "hello", hits: Shared(0) })
    app.get("/greet", greet)
    app.get("/me", me)
    app.get("/users/:id", show)

    let a = app.test().get("/greet").send().unwrap()
    println("${a.status} ${a.text().unwrap()}")
    let b = app.test().get("/me").header("x-user", "ann").send().unwrap()
    println("${b.status} ${b.text().unwrap()}")
    let c = app.test().get("/me").send().unwrap()
    println("${c.status} ${c.text().unwrap()}")
    let d = app.test().get("/users/abc").send().unwrap()
    println("${d.status} ${d.text().unwrap()}")

    let running = app.start("127.0.0.1", 0).unwrap()
    let over_http = wire.get("http://127.0.0.1:${running.port()}/greet").unwrap()
    println("${over_http.status} ${over_http.text().unwrap()}")
    running.stop()
    println("stopped: ${running.wait().is_ok()}")
}
```

```output
200 hello #1
200 hello, ann
403 sign in first
400 id is not a number: abc
200 hello #2
stopped: true
```

## Server

`wire.Server(state)` is the application. `state` is one value the handlers read as `req.state`: a struct of the dependencies the program defines, held in a `Shared`, so it holds no function; a mutable part of it is a `Shared` itself, as `hits` above. `wire.Server(null)` has no state, and its handlers take `wire.Request<Null>`. The methods change the server in place and answer nothing: call them one per line on a `var`. A pattern that is invalid, or the same method and pattern twice, is a fault at the line that adds it. `start`, `run` and `test` serve a frozen copy, so a route added afterwards is not served.

| Item | Does |
|---|---|
| `get`, `post`, `put`, `patch`, `delete` `(pattern, handler)` | add a route |
| `route(method, pattern, handler)` | add a route for any method |
| `fallback(handler)` | answer paths that match nothing |
| `wrap(middleware)` | wrap every route added so far, and the fallback, in a middleware |
| `include(prefix, router)` | add the routes and sockets of a `wire.Router` under a prefix |
| `files(prefix, files)` | serve a `wire.Files` under `prefix` |
| `socket(pattern, handler)` | upgrade `GET pattern` to a WebSocket that `handler` runs on |
| `max_body(bytes)`, `idle_timeout(seconds)` | the limits of the HTTP server underneath |
| `grace(seconds)` | how long `stop` lets requests in flight finish (10) |
| `start(host, port)` | listen (port 0 picks a free one) and serve on tasks of their own; answers `Result<Running, Error>` |
| `run(host, port)` | `start`, then wait: for a `main` that only serves |
| `test()` | a `TestClient` that sends requests through the application with no socket |

A handler is a `Send (wire.Request<S>) -> Result<wire.Reply, Error>`: a named function, or a lambda with typed parameters. `?` works inside it. A dependency is a plain function that takes the request, like `current_user` above, and a handler passes its `Err` up with `?`.

A pattern is `/`-separated segments: a literal, `:name` for one segment, or a final `*name` for the rest of the path. Routes are tried in the order added; the first match wins. A trailing `/` is ignored.

| Request | Reply |
|---|---|
| a path and method match | the handler's |
| a path matches, no route has the method | 405 with an `allow` header |
| no path matches | the fallback, else 404 `not found` |
| `HEAD` | runs the `GET` handler, without the body |

An `Err` from a handler is answered by its kind, with the message as the body:

| `ErrorKind` | Status |
|---|---|
| `NotFound` | 404 |
| `InvalidData` | 400 |
| `PermissionDenied` | 403 |
| `AlreadyExists`, `Conflict` | 409 |
| `Unauthenticated` | 401 |
| `RateLimited` | 429 |
| `TimedOut` | 504 |
| `Unavailable` | 503 |
| anything else | 500 |

A handler that faults (an index out of range, an `unwrap` of `None`) is answered with a 500 `internal error` on a running server, its message goes to standard error, and the server keeps serving. Under `test()` the fault stops the caller.

### Request and Reply

| `wire.Request<S>` | Answers |
|---|---|
| `method`, `target`, `path`, `body`, `state` | the request line, the body as `Bytes`, and the application's state |
| `param(name)` | the `:name` or `*name` part of the path; `""` if the route has none |
| `param_int(name)` | `Result<Int, Error>`; not a number is `InvalidData`, so a 400 |
| `header(name)` | the first header of that name (any case), if any |
| `query()` | the query string as `Map<String, String>` |
| `text()`, `json()`, `form()` | the body as `Result` of `String`, `Json`, or `Map<String, String>` |

| `wire.Reply` | Does |
|---|---|
| `Reply.text(status, s)`, `Reply.json(status, value)`, `Reply.new(status, bytes)`, `Reply.empty(status)` | a reply with a body |
| `Reply.stream(status, chunks)` | a body from a `Stream<Bytes>`, chunked, or as exactly `content-length` bytes when that header is set |
| `reply.header(name, value)` | sets a header, replacing one of the same name |

### Typed JSON

A handler that takes and answers types needs no JSON of its own. `wire.typed(h)` makes a handler from a function `(Request<S>) -> Result<R, Error>`, and `wire.typed_body(h)` from `(Request<S>, B) -> Result<R, Error>` that also gets the request body read as a `B`. `B` and `R` list `JsonCodec` (`struct Item: (JsonCodec)`), or are a `List`, `Set`, `Map<String, _>`, optional or scalar of such types. The reply status is 200, or the `status` argument.

```mote
import wire
import { JsonCodec } from std.json

type Req = wire.Request<Null>

struct Item: (JsonCodec) {
    name: String
    qty: Int
}

struct Health: (JsonCodec) {
    status: String
}

fn health(req: Req) -> Result<Health, Error> {
    return Ok(Health { status: "ok" })
}

fn create(req: Req, item: Item) -> Result<Item, Error> {
    if item.qty < 0 {
        return Err(Error.new(ErrorKind.InvalidData, "qty must not be negative").with_code("negative_qty"))
    }
    return Ok(item)
}

fn show(r: Result<wire.Response, Error>) {
    let got = r.unwrap()
    println("${got.status} ${got.text().unwrap()}")
}

fn main() {
    var app = wire.Server(null)
    app.get("/health", wire.typed(health))
    app.post("/items", wire.typed_body(create, status = 201))

    show(app.test().get("/health").send())
    show(app.test().post("/items").text("{\"name\":\"bolt\",\"qty\":3}").send())
    show(app.test().post("/items").text("{\"name\":\"bolt\"}").send())
    show(app.test().post("/items").text("{\"name\":\"bolt\",\"qty\":\"x\"}").send())
    show(app.test().post("/items").text("not json").send())
    show(app.test().post("/items").text("{\"name\":\"bolt\",\"qty\":-1}").send())
}
```

```output
200 {"status":"ok"}
201 {"name":"bolt","qty":3}
422 {"detail":"missing field \"qty\""}
422 {"detail":"qty: expected an int, found a string"}
400 {"detail":"body is not JSON"}
422 {"detail":"qty must not be negative","code":"negative_qty"}
```

| Case | Reply |
|---|---|
| the body is not JSON | 400 `{"detail":"body is not JSON"}` |
| the body is over `max_json` | 413 `{"detail":"body is too large"}` |
| the body does not fit `B` | 422 `{"detail": "qty: expected an int, found a string"}`, the path to the field first |
| the handler answers `Err` | `{"detail": message}`, and `"code"` when the error has one; the status of its kind, with `InvalidData` a 422 |
| the value is `null` | 204 with no body |
| a handler faults | 500 `{"detail":"internal error"}` as for any route |

`app.error_reply(f)` sets how typed routes write an error, for a house style: `f` is a `(Int, Error) -> wire.Reply` that gets the status the library chose. `app.max_json(bytes)` sets the largest JSON body (1 MiB). A route written by hand can answer a value with `wire.json_reply(value)`, or `wire.json_reply(value, status = 201)`, and keeps its plain-text errors.

### Test mode

`app.test()` answers a `TestClient`; `get`, `head`, `post`, `put`, `patch`, `delete` or `request(method, path)` choose the request, `header`, `body`, `text`, `json` and `form` fill it, and `send()` runs it through the routes and middleware in the calling task. The answer is a `wire.Response`, the same type the client gives, and a streamed reply is read to its end. It has no socket, so it cannot test the HTTP parsing or a WebSocket: `start` and the client do that.

### Static files and streaming

`files("/static", wire.Files("public"))` serves `GET` and `HEAD` under the prefix. `Files.max_age(seconds)` answers a changed copy that adds `cache-control: public, max-age=seconds` (none by default). A directory answers its `index.html`, else a 404.

| Rule | Detail |
|---|---|
| real paths | the root and the file are resolved with `fs.canonicalize`; a link inside the root to a file inside it works, one that leaves the root is a 404; `..` and `\` are a 404 before any lookup |
| headers | content type by extension, a weak `etag` (`if-none-match`), `last-modified` (`if-modified-since`), `accept-ranges`, and one `range` (`bytes=a-b`, `a-`, `-n`) as a 206, or a 416 past the end; `if-range` is honored; several ranges get the whole file |
| not found | a missing file, a hidden one (a segment that starts with `.`) or one outside the root is a 404 that never names the path |
| size | files up to 1 MiB are read at once; bigger ones are streamed from the open file, which is closed when the last piece is read |

A `Stream` is pulled on the connection's task, so a reply needs no producer task. When the client leaves, the stream is not pulled again. A stream that holds a file closes it when it ends; one abandoned half way waits for the collector.

```mote
import wire
import std.fs as fs
import { Stream } from std.stream

fn countdown() -> Stream<Bytes> {
    yield "3 ".bytes()
    yield "2 ".bytes()
    yield "1".bytes()
}

fn count(req: wire.Request<Null>) -> Result<wire.Reply, Error> {
    return Ok(wire.Reply.stream(200, countdown()))
}

fn main() {
    fs.create_dir_all("public").unwrap()
    fs.write_text("public/hello.txt", "hello world").unwrap()
    var app = wire.Server(null)
    app.files("/static", wire.Files("public").max_age(60))
    app.get("/count", count)
    let running = app.start("127.0.0.1", 0).unwrap()
    let base = "http://127.0.0.1:${running.port()}"

    let a = wire.get("${base}/static/hello.txt").unwrap()
    let cache = a.header("cache-control").unwrap()
    println("${a.status} ${a.text().unwrap()} (${cache})")
    let b = wire.request("GET", "${base}/static/hello.txt").header("range", "bytes=6-").send().unwrap()
    let span = b.header("content-range").unwrap()
    println("${b.status} ${span} ${b.text().unwrap()}")
    let c = wire.get("${base}/static/missing.txt").unwrap()
    println("${c.status} ${c.text().unwrap()}")
    let d = wire.get("${base}/count").unwrap()
    println("${d.status} ${d.text().unwrap()}")

    running.stop()
    let ended = running.wait()
    fs.remove_dir_all("public").unwrap()
}
```

```output
200 hello world (public, max-age=60)
206 bytes 6-10/11 world
404 not found
200 3 2 1
```

### WebSocket

`socket(pattern, handler)` adds a `GET` route that answers the handshake (anything else is a 400) and runs the handler, a `Send (wire.Socket) -> Result<Null, Error>`, on the connection; the connection ends when the handler returns. Rooms and broadcast are the program's: keep the sockets in a `Shared`.

| `wire.Socket` | Does |
|---|---|
| `messages()` | a `Stream<Result<Message, Error>>`: `Message.Text(text)`, `Message.Binary(bytes)`, `Message.Close(code, reason)` (answered for you), then the end |
| `recv()` | the next message as `Result<Message?, Error>`, `None` at the end |
| `send_text(text)`, `send_binary(bytes)` | send a message; safe from another task, though messages from different tasks may be written in either order |
| `close(code, reason)` | start the closing handshake (1000 is a normal close); keep reading until the end |
| `end()` | free the framing state and close the connection, once the end has been read |
| `id` | names the socket |

A ping is answered by the library and not shown. A message is at most 16 MiB. `stop` closes every connected socket with 1001. There is no client socket: a program that needs one writes it over `std.socket`.

```mote,skip
import wire

fn echo(socket: wire.Socket) -> Result<Null, Error> {
    for got in socket.messages() {
        match got? {
            wire.Message.Text(text) => { socket.send_text("echo:${text}")? }
            wire.Message.Binary(bytes) => { }
            wire.Message.Close(code, reason) => { }
        }
    }
    return Ok(null)
}

fn main() {
    var app = wire.Server(null)
    app.socket("/echo", echo)
    let ended = app.run("127.0.0.1", 8080)
}
```

### Middleware, routers and stopping

A middleware is a `Send (wire.Handler<S>) -> wire.Handler<S>`: it gets the handler it wraps and answers a handler that calls it, or does not. `wrap` applies to what is in the server when it is called, so `get(...)`, `wrap(a)`, `wrap(b)` in that order runs `b`, then `a`, then the handler; a route added after `wrap` is not wrapped. For one route, call the middleware on the handler: `get("/x", needs_token(handler))`. An `Err` that a middleware returns, or passes up with `?`, is answered by its kind like a handler's, outside the middleware.

A `wire.Router<S>()` holds routes, middleware, files and sockets with no state or limits of its own, and has the same methods as a server except `fallback` and the limits. `include(prefix, router)` adds them to a server or to another router. The prefix is literals and `:name` segments, so the router's handlers can read the prefix's parameters; the state is the including server's. A router can be included in several servers.

`start` answers a `wire.Running`, which is safe to hand to any task:

| `wire.Running` | Does |
|---|---|
| `port()` | the port it listens on |
| `stop()` | graceful: no new connections, idle ones close, requests in flight finish for the grace, sockets close with 1001, and what is left is cancelled |
| `wait()` | waits until it has stopped; `Err` if it ended on a failure |

```mote
import wire

type Req = wire.Request<Null>

fn show(req: Req) -> Result<wire.Reply, Error> {
    let id = req.param_int("id")?
    let org = req.param("org")
    return Ok(wire.Reply.text(200, "${org} user ${id}"))
}

fn logged(next: wire.Handler<Null>) -> wire.Handler<Null> {
    return |req: Req| -> Result<wire.Reply, Error> {
        let reply = next(req)?
        println("${req.method} ${req.path}")
        return Ok(reply)
    }
}

fn ping(req: Req) -> Result<wire.Reply, Error> {
    return Ok(wire.Reply.text(200, "done"))
}

fn main() {
    var users = wire.Router<Null>()
    users.get("/users/:id", show)
    users.wrap(logged)
    var app = wire.Server(null)
    app.include("/orgs/:org", users)
    app.get("/ping", ping)
    app.grace(5)
    let a = app.test().get("/orgs/acme/users/7").send().unwrap()
    println("${a.status} ${a.text().unwrap()}")
    let b = app.test().get("/orgs/acme/users/x").send().unwrap()
    println("${b.status} ${b.text().unwrap()}")

    let running = app.start("127.0.0.1", 0).unwrap()
    let c = wire.get("http://127.0.0.1:${running.port()}/ping").unwrap()
    println("${c.status} ${c.text().unwrap()}")
    running.stop()
    println("stopped cleanly: ${running.wait().is_ok()}")
    let again = wire.get("http://127.0.0.1:${running.port()}/ping")
    println("refused: ${again.is_err()}")
}
```

```output
GET /orgs/acme/users/7
200 acme user 7
400 id is not a number: x
200 done
stopped cleanly: true
refused: true
```

## Client

Any status is an answer: a 404 or a 500 is `Ok(Response)`. Only a failure to get one is an `Err`. HTTPS, redirects and gzip are handled underneath, and a call parks only the calling task. Every call goes through one pool: an idle connection to a host is kept for the next call (up to 256 per host and 512 in all). The library carries its own TLS, so it adds about 1.7 MB to the files you ship.

`wire.HttpClient()` holds what every call shares; each method answers a changed copy.

| `HttpClient` | Does |
|---|---|
| `base(url)` | paths given to the call methods go under `url`; an absolute URL stands as it is |
| `header(name, value)`, `bearer(token)` | a header sent with every call, replacing one of the same name |
| `timeout(seconds)`, `max_redirects(n)`, `max_body(bytes)` | the settings of `ClientRequest` below |
| `get(path)`, `head(path)`, `delete(path)` | one call, `Result<Response, Error>` |
| `post(path, body)`, `put(path, body)`, `patch(path, body)` | the same with a `Bytes` body |
| `post_json(path, json)`, `post_form(path, fields)` | a JSON or form body with its content type |
| `request(method, path)` | a `ClientRequest` with the client's settings, to adjust, then `send()` |

`wire.get(url)` and `wire.post_json(url, value)` are the same calls on a client with no settings. `wire.request(method, url)` starts a `ClientRequest` and `wire.query(fields)` writes `a=1&b=2`, percent-encoded, in the map's order. A call that should be tried again is wrapped by the program.

Each `ClientRequest` method answers a changed copy; `send()` makes the call.

| Method | Sets |
|---|---|
| `header(name, value)` | a header; the same name replaces the earlier one |
| `body(bytes)` | the body |
| `text(s)` | a UTF-8 body with `text/plain` |
| `json(value)` | a JSON body with `application/json`; a value with no JSON text (`NaN`) makes `send()` an `Err` |
| `form(fields)` | a form body with `application/x-www-form-urlencoded` |
| `timeout(seconds)` | the time for the whole call (30) |
| `max_redirects(n)` | redirects to follow (10); more is an `Err`, and 0 answers the redirect itself |
| `max_body(bytes)` | the largest response body (64 MiB); more is an `Err` |
| `to(method, url)` | aims at another method and URL, keeping the rest |

### Typed JSON

A value that lists `JsonCodec` (or a `List`, `Map<String, _>`, optional or scalar of such) goes out and comes back without JSON written by hand:

| Call | Does |
|---|---|
| `wire.post_json(url, value)`, `client.post_json(path, json)` | POST `value` as JSON; the free function takes any `JsonCodec`, the client method a `Json` |
| `wire.with_json(request, value)` | the `ClientRequest` with `value` as its JSON body |
| `wire.json_as(response)` | the body read as an `R`; an `Err` when it is not JSON or does not fit, naming the field |
| `wire.test_json(app.test().post(path), value)` | the same body on a `TestClient` |

`json_as` returns only its type parameter, so the type comes from where the result goes: `let item: Item = wire.json_as(resp)?`, or `let r: Result<Item, Error> = wire.json_as(resp)`.

```mote
import wire
import { JsonCodec } from std.json

type Req = wire.Request<Null>

struct Item: (JsonCodec) {
    name: String
    qty: Int
}

fn create(req: Req, item: Item) -> Result<Item, Error> {
    return Ok(Item { name: item.name, qty: item.qty + 1 })
}

fn all(req: Req) -> Result<List<Item>, Error> {
    return Ok([Item { name: "a", qty: 1 }, Item { name: "b", qty: 2 }])
}

fn run() -> Result<Null, Error> {
    var app = wire.Server(null)
    app.post("/items", wire.typed_body(create, status = 201))
    app.get("/items", wire.typed(all))
    let running = app.start("127.0.0.1", 0)?
    let base = "http://127.0.0.1:${running.port()}"

    let made = wire.post_json("${base}/items", Item { name: "bolt", qty: 3 })?
    let item: Item = wire.json_as(made)?
    println("${made.status} ${item.name} ${item.qty}")

    let api = wire.HttpClient().base(base)
    let got = api.get("/items")?
    let listed: List<Item> = wire.json_as(got)?
    println("${listed.len()} items, last ${listed.get(1).name}")

    let again = wire.with_json(api.request("POST", "/items"), Item { name: "nut", qty: 0 }).send()?
    println(again.text()?)

    let empty = wire.test_json(app.test().post("/items"), Item { name: "washer", qty: 7 }).send()?
    println("${empty.status} ${empty.text()?}")

    running.stop()
    return running.wait()
}

fn main() {
    println("${run().is_ok()}")
}
```

```output
201 bolt 4
2 items, last b
{"name":"nut","qty":1}
201 {"name":"washer","qty":8}
true
```

The whole response body is read before `send()` answers, up to `max_body`.

```mote,skip
import wire

fn main() {
    let r = wire.request("PUT", "https://api.example.com/items/7")
        .header("authorization", "Bearer token")
        .text("hello")
        .timeout(5)
        .send()
    match r {
        Ok(resp) => { println(resp.status) }
        Err(e) => { println("failed: ${e.message}") }
    }
}
```

### Response

| Member | Answers |
|---|---|
| `status` | the status code |
| `url` | the URL requested |
| `body` | the whole body as `Bytes` |
| `ok()` | whether the status is 200 to 299 |
| `header(name)` | the first header of that name (any case), if any |
| `text()` | `Result<String, Error>`; an `Err` if the body is not UTF-8 |
| `json()` | `Result<Json, Error>` |
| `error_for_status()` | the response when 2xx, else an `Err` (`NotFound` for 404, `PermissionDenied` for 401 and 403, else `Other`) |

### Failures

| Cause | `ErrorKind` |
|---|---|
| timeout | `TimedOut` |
| unknown host | `NotFound` |
| bad URL, unsupported scheme, too many redirects, body past the cap | `InvalidData` |
| an I/O error: not found, permission denied or timed out | `NotFound`, `PermissionDenied`, `TimedOut` |
| anything else, such as a refused or reset connection | `Other` |

```mote
import wire

fn main() {
    println(wire.query({"q": "a b", "x": "1&2"}))
}
```

```output
q=a+b&x=1%262
```

Not included: cookies, proxy and certificate settings.

## Tests

`mote test` in this directory runs `tests/`, which start servers on `127.0.0.1`.
