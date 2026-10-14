//! Drives WebSocket frames from bytes through the bytes-in, bytes-out C convention.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Message, Role, WebSocket, WebSocketConfig};
use tungstenite::Error as WsError;

use crate::{failure, finish, Failure};

const ACCEPT_KEY: i64 = 0;
const OPEN: i64 = 1;
const FEED: i64 = 2;
const NEXT: i64 = 3;
const OUT: i64 = 4;
const SEND_TEXT: i64 = 5;
const SEND_BINARY: i64 = 6;
const SEND_CLOSE: i64 = 7;
const FREE: i64 = 8;

/// The socket the library reads from and writes to.
#[derive(Default)]
struct Duplex {
    received: VecDeque<u8>,
    queued: Vec<u8>,
}

impl Read for Duplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.received.is_empty() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = buf.len().min(self.received.len());
        for (slot, byte) in buf.iter_mut().zip(self.received.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }
}

impl Write for Duplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.queued.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Session {
    ws: WebSocket<Duplex>,
    message: Option<Vec<u8>>,
}

static SESSIONS: LazyLock<Mutex<HashMap<i64, Arc<Mutex<Session>>>>> = LazyLock::new(Mutex::default);

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn session(id: i64) -> Result<Arc<Mutex<Session>>, Failure> {
    SESSIONS.lock().unwrap().get(&id).cloned().ok_or_else(|| failure("Other", "the WebSocket is closed"))
}

fn open(limit: usize) -> i64 {
    let config = WebSocketConfig::default().max_message_size(Some(limit)).max_frame_size(Some(limit));
    let ws = WebSocket::from_raw_socket(Duplex::default(), Role::Server, Some(config));
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    SESSIONS.lock().unwrap().insert(id, Arc::new(Mutex::new(Session { ws, message: None })));
    id
}

fn closed(e: WsError) -> Failure {
    match e {
        WsError::ConnectionClosed | WsError::AlreadyClosed => failure("Other", "the WebSocket is closed"),
        other => failure("InvalidData", other),
    }
}

/// Reads the next message as a kind byte and payload.
fn read_message(s: &mut Session) -> Result<Vec<u8>, Failure> {
    loop {
        match s.ws.read() {
            Ok(Message::Text(t)) => return Ok([b"T".as_slice(), t.as_str().as_bytes()].concat()),
            Ok(Message::Binary(b)) => return Ok([b"B".as_slice(), &b[..]].concat()),
            Ok(Message::Close(frame)) => {
                let _ = s.ws.flush();
                let (code, reason) = frame.map(|f| (u16::from(f.code), f.reason.as_str().to_string())).unwrap_or((1005, String::new()));
                return Ok([b"C".as_slice(), &code.to_be_bytes(), reason.as_bytes()].concat());
            }
            Ok(_) => continue,
            Err(WsError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => return Ok(Vec::new()),
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(b"X".to_vec()),
            Err(e) => return Err(failure("InvalidData", e)),
        }
    }
}

fn call(op: i64, id: i64, input: &[u8], cap: usize) -> Result<Vec<u8>, Failure> {
    match op {
        ACCEPT_KEY => Ok(tungstenite::handshake::derive_accept_key(input).into_bytes()),
        OPEN => {
            let limit: usize = std::str::from_utf8(input).ok().and_then(|t| t.parse().ok()).ok_or_else(|| failure("InvalidData", "the message limit is malformed"))?;
            Ok(open(limit).to_string().into_bytes())
        }
        FREE => {
            SESSIONS.lock().unwrap().remove(&id);
            Ok(Vec::new())
        }
        _ => {
            let shared = session(id)?;
            let mut s = shared.lock().unwrap();
            match op {
                FEED => {
                    s.ws.get_mut().received.extend(input);
                    Ok(Vec::new())
                }
                NEXT => {
                    if s.message.is_none() {
                        let m = read_message(&mut s)?;
                        if m.is_empty() {
                            return Ok(m);
                        }
                        s.message = Some(m);
                    }
                    let held = s.message.as_ref().unwrap();
                    if held.len() <= cap { Ok(s.message.take().unwrap()) } else { Ok(held.clone()) }
                }
                OUT => {
                    let queued = &mut s.ws.get_mut().queued;
                    if queued.len() <= cap { Ok(std::mem::take(queued)) } else { Ok(queued.clone()) }
                }
                SEND_TEXT => {
                    let text = String::from_utf8(input.to_vec()).map_err(|_| failure("InvalidData", "the text is not UTF-8"))?;
                    s.ws.send(Message::text(text)).map_err(closed)?;
                    Ok(Vec::new())
                }
                SEND_BINARY => {
                    s.ws.send(Message::binary(input.to_vec())).map_err(closed)?;
                    Ok(Vec::new())
                }
                SEND_CLOSE => {
                    if input.len() < 2 {
                        return Err(failure("InvalidData", "a close needs a code"));
                    }
                    let code = u16::from_be_bytes([input[0], input[1]]);
                    let reason = String::from_utf8_lossy(&input[2..]).into_owned();
                    s.ws.close(Some(CloseFrame { code: CloseCode::from(code), reason: reason.into() })).map_err(closed)?;
                    Ok(Vec::new())
                }
                _ => Err(failure("Other", "unknown operation")),
            }
        }
    }
}

/// Runs one WebSocket operation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_ws(op: i64, id: i64, input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    // SAFETY: the caller promises `input` is readable for `len` bytes.
    let bytes: &[u8] = if len <= 0 || input.is_null() { &[] } else { unsafe { slice::from_raw_parts(input, len as usize) } };
    let answer = catch_unwind(AssertUnwindSafe(|| call(op, id, bytes, cap.max(0) as usize))).unwrap_or_else(|_| Err(failure("Other", "the library failed")));
    if op == OPEN {
        return match answer {
            Ok(id) => String::from_utf8(id).ok().and_then(|t| t.parse().ok()).unwrap_or(0),
            Err(f) => finish(Err(f), out, cap),
        };
    }
    finish(answer, out, cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(op: i64, id: i64, input: &[u8]) -> Vec<u8> {
        call(op, id, input, 1 << 20).unwrap()
    }

    fn opened() -> i64 {
        String::from_utf8(run(OPEN, 0, b"1048576")).unwrap().parse().unwrap()
    }

    #[test]
    fn the_accept_key_is_the_rfc_example() {
        assert_eq!(run(ACCEPT_KEY, 0, b"dGhlIHNhbXBsZSBub25jZQ=="), b"s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn a_masked_text_frame_is_read_and_a_reply_is_queued() {
        let id = opened();
        assert!(run(NEXT, id, b"").is_empty());
        run(FEED, id, &[0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]);
        assert_eq!(run(NEXT, id, b""), b"THello");
        run(SEND_TEXT, id, b"Hi");
        assert_eq!(run(OUT, id, b""), [0x81, 0x02, b'H', b'i']);
        assert!(run(OUT, id, b"").is_empty());
    }

    #[test]
    fn a_frame_in_two_pieces_waits_for_the_rest() {
        let id = opened();
        run(FEED, id, &[0x81, 0x85, 0x37, 0xfa]);
        assert!(run(NEXT, id, b"").is_empty());
        run(FEED, id, &[0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]);
        assert_eq!(run(NEXT, id, b""), b"THello");
    }

    #[test]
    fn a_ping_is_answered_with_a_pong() {
        let id = opened();
        run(FEED, id, &[0x89, 0x80, 1, 2, 3, 4]);
        assert!(run(NEXT, id, b"").is_empty());
        assert_eq!(run(OUT, id, b""), [0x8a, 0x00]);
    }

    #[test]
    fn a_close_is_reported_and_answered() {
        let id = opened();
        run(FEED, id, &[0x88, 0x82, 0, 0, 0, 0, 0x03, 0xe8]);
        assert_eq!(run(NEXT, id, b""), [b'C', 0x03, 0xe8]);
        assert_eq!(run(OUT, id, b"")[0], 0x88);
    }

    #[test]
    fn a_message_over_the_room_is_kept_until_asked_for_again() {
        let id = opened();
        run(FEED, id, &[0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]);
        assert_eq!(call(NEXT, id, b"", 3).unwrap().len(), 6);
        assert_eq!(call(NEXT, id, b"", 6).unwrap(), b"THello");
    }

    #[test]
    fn an_unmasked_frame_from_a_client_is_an_error() {
        let id = opened();
        run(FEED, id, &[0x81, 0x02, b'H', b'i']);
        assert_eq!(call(NEXT, id, b"", 100).unwrap_err().kind, "InvalidData");
    }

    #[test]
    fn a_freed_session_is_closed() {
        let id = opened();
        run(FREE, id, b"");
        assert!(call(NEXT, id, b"", 10).is_err());
    }
}
