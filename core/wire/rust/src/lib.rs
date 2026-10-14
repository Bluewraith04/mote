//! Sends HTTP requests through the bytes-in, bytes-out C convention.

mod ws;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) kind: &'static str,
    message: String,
}

pub(crate) fn failure(kind: &'static str, message: impl ToString) -> Failure {
    Failure { kind, message: message.to_string() }
}

fn kind_of(e: &ureq::Error) -> &'static str {
    match e {
        ureq::Error::Timeout(_) => "TimedOut",
        ureq::Error::HostNotFound => "NotFound",
        ureq::Error::BadUri(_) | ureq::Error::Http(_) | ureq::Error::Protocol(_) | ureq::Error::BodyExceedsLimit(_) => "InvalidData",
        ureq::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::NotFound => "NotFound",
            std::io::ErrorKind::PermissionDenied => "PermissionDenied",
            std::io::ErrorKind::TimedOut => "TimedOut",
            _ => "Other",
        },
        _ => "Other",
    }
}

fn transport(e: ureq::Error) -> Failure {
    failure(kind_of(&e), e)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn line(&mut self) -> Result<&'a str, Failure> {
        let rest = &self.bytes[self.at..];
        let end = rest.iter().position(|b| *b == b'\n').ok_or_else(|| failure("InvalidData", "the request is cut short"))?;
        self.at += end + 1;
        std::str::from_utf8(&rest[..end]).map_err(|_| failure("InvalidData", "the request is malformed"))
    }

    fn frame(&mut self) -> Result<&'a [u8], Failure> {
        let n: usize = self.line()?.parse().map_err(|_| failure("InvalidData", "a frame length is malformed"))?;
        let end = self.at.checked_add(n).filter(|e| *e <= self.bytes.len()).ok_or_else(|| failure("InvalidData", "the request is cut short"))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn text(&mut self) -> Result<&'a str, Failure> {
        std::str::from_utf8(self.frame()?).map_err(|_| failure("InvalidData", "the request holds text that is not UTF-8"))
    }
}

fn write_frame(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(format!("{}\n", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
}

static PENDING: LazyLock<Mutex<HashMap<i64, Vec<u8>>>> = LazyLock::new(Mutex::default);

static NEXT: AtomicI64 = AtomicI64::new(1);

struct Call {
    timeout: Duration,
    max_redirects: u32,
    max_body: u64,
    request: ureq::http::Request<Vec<u8>>,
}

fn parse(input: &[u8]) -> Result<Call, Failure> {
    let mut r = Reader { bytes: input, at: 0 };
    let head: Vec<u64> = r.line()?.split(' ').map(|n| n.parse().map_err(|_| failure("InvalidData", "the request is malformed"))).collect::<Result<_, _>>()?;
    let [timeout_millis, max_redirects, max_body, header_count] = head[..] else { return Err(failure("InvalidData", "the request is malformed")) };
    let method = r.text()?;
    let url = r.text()?;
    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for _ in 0..header_count {
        let name = r.text()?;
        let value = r.text()?;
        builder = builder.header(name, value);
    }
    let body = r.frame()?.to_vec();
    let request = builder.body(body).map_err(|e| failure("InvalidData", e))?;
    Ok(Call { timeout: Duration::from_millis(timeout_millis.max(1)), max_redirects: max_redirects as u32, max_body, request })
}

/// Encodes the status and headers of a response.
fn head_of<B>(response: &ureq::http::Response<B>) -> Vec<u8> {
    let mut head = format!("{} {}\n", response.status().as_u16(), response.headers().len()).into_bytes();
    for (name, value) in response.headers() {
        write_frame(&mut head, name.as_str().as_bytes());
        write_frame(&mut head, String::from_utf8_lossy(value.as_bytes()).as_bytes());
    }
    head
}

fn keep(answer: Vec<u8>) -> (i64, usize) {
    let ticket = NEXT.fetch_add(1, Ordering::SeqCst);
    let size = answer.len();
    PENDING.lock().unwrap().insert(ticket, answer);
    (ticket, size)
}

/// The shared agent that keeps connections for later calls to the same host.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .max_redirects_will_error(true)
            .http_status_as_error(false)
            .max_idle_connections(512)
            .max_idle_connections_per_host(256)
            .build()
            .into()
    })
}

fn request(input: &[u8]) -> Result<Vec<u8>, Failure> {
    let call = parse(input)?;
    let agent = agent();
    let request = agent.configure_request(call.request).timeout_global(Some(call.timeout)).max_redirects(call.max_redirects).build();
    let mut response = agent.run(request).map_err(transport)?;
    let mut answer = head_of(&response);
    let body = response.body_mut().with_config().limit(call.max_body.saturating_add(1)).read_to_vec().map_err(transport)?;
    if body.len() as u64 > call.max_body {
        return Err(failure("InvalidData", format!("the body is over {} bytes", call.max_body)));
    }
    answer.extend_from_slice(&body);
    let (ticket, size) = keep(answer);
    Ok(format!("{ticket} {size}\n").into_bytes())
}

/// Writes the answer or the failure message to `out` and returns its signed length.
pub(crate) fn finish(answer: Result<Vec<u8>, Failure>, out: *mut u8, cap: i64) -> i64 {
    let (data, sign) = match answer {
        Ok(data) => (data, 1),
        Err(f) => (format!("{} {}", f.kind, f.message).into_bytes(), -1),
    };
    if (sign > 0 && data.len() as i64 <= cap || sign < 0) && cap > 0 && !out.is_null() {
        let n = data.len().min(cap as usize);
        // SAFETY: the caller promises `out` is writable for `cap` bytes and `n <= cap`.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, n) };
    }
    sign * data.len() as i64
}

/// Sends a request and returns the ticket and size of its answer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_http_request(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    // SAFETY: the caller promises `input` is readable for `len` bytes.
    let bytes: &[u8] = if len <= 0 || input.is_null() { &[] } else { unsafe { slice::from_raw_parts(input, len as usize) } };
    let answer = catch_unwind(AssertUnwindSafe(|| request(bytes))).unwrap_or_else(|_| Err(failure("Other", "the library failed")));
    finish(answer, out, cap)
}

/// Writes the answer kept under a ticket to `out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_http_take(ticket: i64, out: *mut u8, cap: i64) -> i64 {
    let kept = PENDING.lock().unwrap().remove(&ticket);
    finish(kept.ok_or_else(|| failure("Other", "the answer was already taken")), out, cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve(reply: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).unwrap();
            s.write_all(reply).unwrap();
        });
        url
    }

    fn ask(url: &str, max_body: u64) -> Result<Vec<u8>, Failure> {
        let mut input = format!("5000 10 {max_body} 1\n").into_bytes();
        for part in ["GET", url, "x-a", "1"] {
            write_frame(&mut input, part.as_bytes());
        }
        write_frame(&mut input, b"");
        let head = String::from_utf8(request(&input)?).unwrap();
        let ticket: i64 = head.split(' ').next().unwrap().parse().unwrap();
        Ok(PENDING.lock().unwrap().remove(&ticket).unwrap())
    }

    #[test]
    fn an_answer_carries_status_headers_and_body() {
        let url = serve(b"HTTP/1.1 404 Not Found\r\ncontent-length: 2\r\nx-b: yes\r\nconnection: close\r\n\r\nhi");
        let got = ask(&url, 1024).unwrap();
        let text = String::from_utf8(got).unwrap();
        assert!(text.starts_with("404 "), "{text}");
        assert!(text.contains("3\nx-b3\nyes"), "{text}");
        assert!(text.ends_with("hi"), "{text}");
    }

    #[test]
    fn a_body_past_the_limit_is_an_error() {
        let url = serve(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello");
        assert_eq!(ask(&url, 3).unwrap_err().kind, "InvalidData");
    }

    #[test]
    fn calls_to_one_host_share_a_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            for _ in 0..3 {
                let mut seen = Vec::new();
                let mut buf = [0u8; 4096];
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = s.read(&mut buf).unwrap();
                    assert!(n > 0, "the client closed the connection");
                    seen.extend_from_slice(&buf[..n]);
                }
                s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi").unwrap();
            }
        });
        for _ in 0..3 {
            let text = String::from_utf8(ask(&url, 1024).unwrap()).unwrap();
            assert!(text.starts_with("200 "), "{text}");
        }
        server.join().unwrap();
    }

    #[test]
    fn a_refused_connection_is_an_error() {
        let url = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/", l.local_addr().unwrap())
        };
        assert!(ask(&url, 10).is_err());
    }
}
