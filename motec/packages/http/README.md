# http

An HTTP and HTTPS client over the `ureq` crate. The package carries the crate as a library in `native/<triple>/`, so add it with native access granted, then `import http as http`.

```toml
[dependencies]
http = { git = "github:user/http", tag = "v0.1.0", native = true }
```

Libraries are built for `x86_64-unknown-linux-gnu` here. To build one for another machine: `cargo build --release --manifest-path rust/Cargo.toml`, then copy `rust/target/release/libmote_http.so` (`.dylib`, `mote_http.dll`) into `native/<triple>/`. The functions are in the bytes-in, bytes-out shape of `std.dev.libtools`; the answer stays in the library under a ticket until `mote_http_take` fetches it, so a request is never sent twice. The library carries its own TLS, so it adds about 1.7 MB to the files you ship.

HTTPS, redirects and gzip are handled underneath. A call waits for the answer on a helper thread and parks only the calling task. There is no scripted stand-in for tests: point the code at a server on `127.0.0.1`.

Any status is an answer: a 404 or a 500 is `Ok(Response)`. Only a failure to get one is an `Err`.

```mote,skip
import http as http

fn main() {
    let r = http.get("https://example.com/").unwrap()
    println(r.status)
    println(r.text().unwrap())
}
```

## Calls

| Function | Does |
|---|---|
| `get(url)`, `head(url)`, `delete(url)` | one call, `Result<Response, Error>` |
| `post(url, body)`, `put(url, body)`, `patch(url, body)` | the same with a `Bytes` body |
| `post_json(url, json)` | the body is JSON, with a JSON content type |
| `post_form(url, fields)` | the body is a form (`Map<String, String>`) |
| `request(method, url)` | a `Request` to adjust, then `send()` |
| `query(fields)` | `a=1&b=2`, percent-encoded, in the map's order |

## Request

Each method answers a changed copy; `send()` makes the call.

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

```mote,skip
import http as http

fn main() {
    let r = http.request("PUT", "https://api.example.com/items/7")
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

## Response

| Member | Answers |
|---|---|
| `status` | the status code |
| `url` | the URL requested |
| `body` | the whole body as `Bytes` |
| `ok()` | whether the status is 200 to 299 |
| `header(name)` | the first header of that name (any case), if any |
| `header_map()` | every header by lowercase name; a repeated header's values joined with `, ` |
| `text()` | `Result<String, Error>`; an `Err` if the body is not UTF-8 |
| `json()` | `Result<Json, Error>` |
| `error_for_status()` | the response when 2xx, else an `Err` (`NotFound` for 404, `PermissionDenied` for 401 and 403, else `Other`) |

## Failures

| Cause | `ErrorKind` |
|---|---|
| timeout | `TimedOut` |
| unknown host | `NotFound` |
| bad URL, unsupported scheme, too many redirects, body past the cap | `InvalidData` |
| refused, reset | by the I/O error |

## Query strings

```mote,skip
import http as http

fn main() {
    println(http.query({"q": "a b", "x": "1&2"}))
}
```

```text
q=a+b&x=1%262
```

Not included: streaming bodies, auth helpers, proxy and certificate settings, cookies.
