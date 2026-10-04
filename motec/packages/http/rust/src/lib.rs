//! An HTTP client over `ureq`, behind the bytes-in, bytes-out C convention of `std.dev.libtools`.
//!
//! `mote_http_request` keeps the answer under a ticket until `mote_http_take` fetches it, so a request is never sent twice.
//! A request is `<timeout ms> <max redirects> <max body> <header count>\n`, then frames (`<len>\n<bytes>`) for the method, the url, each header's name and value, and the body.
//! An answer is `<status> <header count>\n`, a frame for each header's name and value, then the body.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

#[derive(Debug)]
struct Failure {
    kind: &'static str,
    message: String,
}

fn failure(kind: &'static str, message: impl ToString) -> Failure {
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

fn request(input: &[u8]) -> Result<Vec<u8>, Failure> {
    let mut r = Reader { bytes: input, at: 0 };
    let head: Vec<u64> = r.line()?.split(' ').map(|n| n.parse().map_err(|_| failure("InvalidData", "the request is malformed"))).collect::<Result<_, _>>()?;
    let [timeout_millis, max_redirects, max_body, header_count] = head[..] else { return Err(failure("InvalidData", "the request is malformed")) };
    let method = r.text()?;
    let url = r.text()?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(timeout_millis.max(1))))
        .max_redirects(max_redirects as u32)
        .max_redirects_will_error(true)
        .http_status_as_error(false)
        .build()
        .into();
    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for _ in 0..header_count {
        let name = r.text()?;
        let value = r.text()?;
        builder = builder.header(name, value);
    }
    let body = r.frame()?.to_vec();
    let request = builder.body(body).map_err(|e| failure("InvalidData", e))?;
    let mut response = agent.run(request).map_err(transport)?;
    let mut answer = format!("{} {}\n", response.status().as_u16(), response.headers().len()).into_bytes();
    for (name, value) in response.headers() {
        write_frame(&mut answer, name.as_str().as_bytes());
        write_frame(&mut answer, String::from_utf8_lossy(value.as_bytes()).as_bytes());
    }
    let body = response.body_mut().with_config().limit(max_body.saturating_add(1)).read_to_vec().map_err(transport)?;
    if body.len() as u64 > max_body {
        return Err(failure("InvalidData", format!("the body is over {max_body} bytes")));
    }
    answer.extend_from_slice(&body);
    let ticket = NEXT.fetch_add(1, Ordering::SeqCst);
    let size = answer.len();
    PENDING.lock().unwrap().insert(ticket, answer);
    Ok(format!("{ticket} {size}\n").into_bytes())
}

/// Copies `data` to `out` when it fits and answers its length; a failure answers the negated length of `<kind> <message>`, written to `out` (cut to `cap`).
fn finish(answer: Result<Vec<u8>, Failure>, out: *mut u8, cap: i64) -> i64 {
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

/// A request in, `<ticket> <size>\n` out: `mote_http_take` fetches the answer.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_http_request(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    // SAFETY: the caller promises `input` is readable for `len` bytes.
    let bytes: &[u8] = if len <= 0 || input.is_null() { &[] } else { unsafe { slice::from_raw_parts(input, len as usize) } };
    let answer = catch_unwind(AssertUnwindSafe(|| request(bytes))).unwrap_or_else(|_| Err(failure("Other", "the library failed")));
    finish(answer, out, cap)
}

/// Writes the answer kept under `ticket` to `out`, which must hold the `<size>` that `mote_http_request` gave.
///
/// # Safety
/// `out` must be writable for `cap` bytes.
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
    fn a_refused_connection_is_an_error() {
        let url = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/", l.local_addr().unwrap())
        };
        assert!(ask(&url, 10).is_err());
    }
}
