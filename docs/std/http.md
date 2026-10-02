# std.sys.http

An HTTP and HTTPS client. `import std.sys.http as http`. HTTPS, redirects and gzip are handled underneath; a call parks only the calling task. Any status is an answer: a 404 is `Ok(Response)`, and only a failure to get one is an `Err`.

```mote,skip
import std.sys.http as http

fn main() {
    let r = http.get("https://example.com/")!
    println(r.status)
    println(r.text()!)
}
```

| Function | Does |
|---|---|
| `get(url)`, `head(url)`, `delete(url)` | one call, `Result<Response, Error>` |
| `post(url, body)`, `put`, `patch` | the same with a `Bytes` body |
| `post_json(url, json)`, `post_form(url, fields)` | a JSON or form body |
| `request(method, url)` | a `Request` to adjust, then `send()` |
| `query(fields)` | `a=1&b=2`, percent-encoded |

Each `Request` method answers a changed copy: `header(name, value)`, `body(bytes)`, `text(s)`, `json(value)`, `form(fields)`, `timeout(seconds)` (30), `max_redirects(n)` (10), `max_body(bytes)` (64 MiB).

```mote,skip
let r = http.request("PUT", "https://api.example.com/items/7")
    .header("authorization", "Bearer token")
    .text("hello")
    .timeout(5)
    .send()
```

| `Response` member | Answers |
|---|---|
| `status`, `url`, `body` | the code, the URL requested, the body as `Bytes` |
| `ok()` | whether the status is 200 to 299 |
| `header(name)`, `header_map()` | one header, or all by lowercase name |
| `text()`, `json()` | `Result` of the body as text or `Json` |
| `error_for_status()` | the response when 2xx, else an `Err` |

A timeout is `TimedOut`, an unknown host `NotFound`, and a bad URL, too many redirects or a body past the cap `InvalidData`.

```mote
import std.sys.http as http

fn main() {
    println(http.query({"q": "a b", "x": "1&2"}))
}
```

```output
q=a+b&x=1%262
```

Not included: streaming bodies, auth helpers, proxy and certificate settings, cookies.
