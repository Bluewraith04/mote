# std.sys.tls

TLS for sockets and the HTTP server, over rustls. `import std.sys.tls as tls`. A TLS stream is an ordinary `TcpStream` ([net](net.md)).

`net.connect_tls(host, port)` finishes the handshake before it answers, so a bad certificate is an `Err` there. The server is checked against the bundled root certificates and the host name.

```mote,skip
import std.sys.net as net

fn main() {
    let c = net.connect_tls("example.com", 443)!
    let sent = c.write_text("GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
    println(c.read(40)!.decode()!)
}
```

`net.listen_tls(host, port, identity)` and `http_server.listen_tls(host, port, identity)` serve TLS; the handshake happens on the first read or write, in the task serving the connection. An `Identity` is a PEM certificate chain and private key, checked to load and match when made:

| Function | Does |
|---|---|
| `identity_from_files(cert_path, key_path)` | reads a PEM chain file and a PEM key file |
| `identity_from_pem(cert, key)` | from PEM text |
| `self_signed(names)` | a new self-signed certificate for development |
| `identity.cert_pem()`, `identity.key_pem()` | the PEM text |

```mote,skip
import std.sys.http_server as server
import std.sys.tls as tls

fn main() {
    let id = tls.identity_from_files("cert.pem", "key.pem")!
    let s = server.listen_tls("0.0.0.0", 8443, id)!
    println(s.local_port()!)
}
```

TLS 1.2 and 1.3. `close` sends `close_notify`. A TLS socket does one read or write at a time. A self-signed certificate is trusted by no client until the client is told to. Not included: custom CA roots, client certificates, several certificates by host name, ALPN, certificate reload.
