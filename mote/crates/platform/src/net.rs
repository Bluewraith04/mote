//! The real machine's sockets: a table of streams, TLS sessions, listeners and UDP sockets by id.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use contracts::{PlatformError, PlatformErrorKind, PlatformRequest, PlatformResponse};

use crate::ext::{missing, Secure, ServerSettings, Tls};
use crate::reactor::Pollable;
use crate::system::io_error;

const MAX_READ: usize = 1 << 20;

pub(crate) enum Readiness {
    Now(Result<PlatformResponse, PlatformError>),
    Wait(Pollable, Option<Duration>),
    Run,
}

#[derive(Clone)]
enum Socket {
    Stream(Arc<TcpStream>),
    Tls(Arc<dyn Secure>),
    Listener(Arc<TcpListener>),
    TlsListener(Arc<TcpListener>, ServerSettings),
    Udp(Arc<UdpSocket>),
}

#[derive(Clone)]
enum Stream {
    Plain(Arc<TcpStream>),
    Tls(Arc<dyn Secure>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(s) => s.tcp(),
        }
    }

    fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => (&**s).read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }

    fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => (&**s).write_all(bytes),
            Stream::Tls(s) => s.write_all(bytes),
        }
    }

    fn shutdown_write(&self) -> std::io::Result<()> {
        if let Stream::Tls(s) = self {
            s.send_close_notify();
        }
        self.tcp().shutdown(Shutdown::Write)
    }

    fn close(&self) {
        if let Stream::Tls(s) = self {
            s.send_close_notify();
        }
        let _ = self.tcp().shutdown(Shutdown::Both);
    }
}

pub(crate) struct SystemNet {
    sockets: Mutex<HashMap<i64, Socket>>,
    next_id: AtomicI64,
    tls: &'static dyn Tls,
}

impl SystemNet {
    pub(crate) fn new(tls: &'static dyn Tls) -> Self {
        SystemNet { sockets: Mutex::new(HashMap::new()), next_id: AtomicI64::new(1), tls }
    }

    fn tls(&self) -> Result<&'static dyn Tls, PlatformError> {
        if self.tls.present() { Ok(self.tls) } else { Err(missing("TLS")) }
    }

    fn insert(&self, socket: Socket) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.sockets.lock().unwrap().insert(id, socket);
        id
    }

    fn get(&self, id: i64) -> Result<Socket, PlatformError> {
        self.sockets.lock().unwrap().get(&id).cloned().ok_or_else(|| other("socket is closed"))
    }

    fn stream(&self, id: i64) -> Result<Stream, PlatformError> {
        match self.get(id)? {
            Socket::Stream(s) => Ok(Stream::Plain(s)),
            Socket::Tls(s) => Ok(Stream::Tls(s)),
            _ => Err(other("socket is not a TCP stream")),
        }
    }

    fn udp(&self, id: i64) -> Result<Arc<UdpSocket>, PlatformError> {
        match self.get(id)? {
            Socket::Udp(s) => Ok(s),
            _ => Err(other("socket is not a UDP socket")),
        }
    }

    /// `trust_pem` holds extra certificates to trust, or is empty.
    pub(crate) fn tls_connect(&self, host: &str, port: u16, trust_pem: &str) -> Result<i64, PlatformError> {
        let tls = self.tls()?;
        let stream = TcpStream::connect(resolve(host, port)?.as_slice()).map_err(io_error)?;
        let session = tls.client(Arc::new(stream), host, trust_pem)?;
        Ok(self.insert(Socket::Tls(session)))
    }

    pub(crate) fn readiness(&self, request: &PlatformRequest) -> Readiness {
        let (id, max, limit) = match *request {
            PlatformRequest::TcpAccept { id } => {
                return match self.get(id) {
                    Ok(Socket::Listener(l) | Socket::TlsListener(l, _)) => Readiness::Wait(Pollable::Listener(l), None),
                    _ => Readiness::Run,
                };
            }
            PlatformRequest::UdpRecvFrom { id, .. } => {
                return match self.udp(id) {
                    Ok(socket) => Readiness::Wait(Pollable::Udp(socket), None),
                    Err(_) => Readiness::Run,
                };
            }
            PlatformRequest::SocketRead { id, max } => (id, max, None),
            PlatformRequest::SocketReadFor { id, max, timeout_millis } => (id, max, Some(Duration::from_millis(timeout_millis.max(1)))),
            _ => return Readiness::Run,
        };
        match self.stream(id) {
            Ok(Stream::Plain(s)) => Readiness::Wait(Pollable::Stream(s), limit),
            Ok(Stream::Tls(session)) => match session.buffered(max.min(MAX_READ)) {
                Some(read) => Readiness::Now(read.map(PlatformResponse::Bytes).map_err(io_error)),
                None => Readiness::Wait(Pollable::Stream(session.tcp().clone()), limit),
            },
            Err(_) => Readiness::Run,
        }
    }

    /// Closes a socket; a listener or UDP socket answers its pollable, so the reactor can wake what waits on it.
    pub(crate) fn close(&self, id: i64) -> Result<Option<Pollable>, PlatformError> {
        let socket = self.sockets.lock().unwrap().remove(&id).ok_or_else(|| other("socket is closed"))?;
        Ok(match socket {
            Socket::Stream(stream) => {
                Stream::Plain(stream).close();
                None
            }
            Socket::Tls(session) => {
                Stream::Tls(session).close();
                None
            }
            Socket::Listener(l) | Socket::TlsListener(l, _) => Some(Pollable::Listener(l)),
            Socket::Udp(u) => Some(Pollable::Udp(u)),
        })
    }

    pub(crate) fn request(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        Ok(match request {
            PlatformRequest::TcpConnect { host, port } => {
                let stream = TcpStream::connect(resolve(&host, port)?.as_slice()).map_err(io_error)?;
                PlatformResponse::Int(self.insert(Socket::Stream(Arc::new(stream))))
            }
            PlatformRequest::TlsConnect { host, port } => PlatformResponse::Int(self.tls_connect(&host, port, "")?),
            PlatformRequest::TcpListen { host, port } => {
                let listener = TcpListener::bind(resolve(&host, port)?.as_slice()).map_err(io_error)?;
                PlatformResponse::Int(self.insert(Socket::Listener(Arc::new(listener))))
            }
            PlatformRequest::TlsListen { host, port, cert_pem, key_pem } => {
                let config = self.tls()?.server_settings(&cert_pem, &key_pem)?;
                let listener = TcpListener::bind(resolve(&host, port)?.as_slice()).map_err(io_error)?;
                PlatformResponse::Int(self.insert(Socket::TlsListener(Arc::new(listener), config)))
            }
            PlatformRequest::TcpAccept { id } => match self.get(id)? {
                Socket::Listener(listener) => {
                    let (stream, _) = listener.accept().map_err(io_error)?;
                    PlatformResponse::Int(self.insert(Socket::Stream(Arc::new(stream))))
                }
                Socket::TlsListener(listener, config) => {
                    let (stream, _) = listener.accept().map_err(io_error)?;
                    let session = self.tls()?.server(Arc::new(stream), &config)?;
                    PlatformResponse::Int(self.insert(Socket::Tls(session)))
                }
                _ => return Err(other("socket is not a listener")),
            },
            PlatformRequest::SocketRead { id, max } => {
                let stream = self.stream(id)?;
                let mut buf = vec![0u8; max.min(MAX_READ)];
                let n = stream.read(&mut buf).map_err(io_error)?;
                buf.truncate(n);
                PlatformResponse::Bytes(buf)
            }
            PlatformRequest::SocketReadFor { id, max, timeout_millis } => {
                let stream = self.stream(id)?;
                stream.tcp().set_read_timeout(Some(Duration::from_millis(timeout_millis.max(1)))).map_err(io_error)?;
                let mut buf = vec![0u8; max.min(MAX_READ)];
                let read = stream.read(&mut buf);
                let _ = stream.tcp().set_read_timeout(None);
                let n = read.map_err(|e| match e.kind() {
                    std::io::ErrorKind::WouldBlock => PlatformError { kind: PlatformErrorKind::TimedOut, message: "read timed out".to_string() },
                    _ => io_error(e),
                })?;
                buf.truncate(n);
                PlatformResponse::Bytes(buf)
            }
            PlatformRequest::SocketWrite { id, bytes } => {
                self.stream(id)?.write_all(&bytes).map_err(io_error)?;
                PlatformResponse::Unit
            }
            PlatformRequest::SocketShutdownWrite { id } => {
                self.stream(id)?.shutdown_write().map_err(io_error)?;
                PlatformResponse::Unit
            }
            PlatformRequest::SocketClose { id } => {
                self.close(id)?;
                PlatformResponse::Unit
            }
            PlatformRequest::LocalPort { id } => {
                let addr = match self.get(id)? {
                    Socket::Stream(s) => s.local_addr(),
                    Socket::Tls(s) => s.tcp().local_addr(),
                    Socket::Listener(s) | Socket::TlsListener(s, _) => s.local_addr(),
                    Socket::Udp(s) => s.local_addr(),
                };
                PlatformResponse::Int(i64::from(addr.map_err(io_error)?.port()))
            }
            PlatformRequest::PeerAddr { id } => {
                PlatformResponse::Text(self.stream(id)?.tcp().peer_addr().map_err(io_error)?.to_string())
            }
            PlatformRequest::UdpBind { host, port } => {
                let socket = UdpSocket::bind(resolve(&host, port)?.as_slice()).map_err(io_error)?;
                PlatformResponse::Int(self.insert(Socket::Udp(Arc::new(socket))))
            }
            PlatformRequest::UdpSendTo { id, host, port, bytes } => {
                let sent = self.udp(id)?.send_to(&bytes, resolve(&host, port)?.as_slice()).map_err(io_error)?;
                PlatformResponse::Int(sent as i64)
            }
            PlatformRequest::UdpRecvFrom { id, max } => {
                let mut buf = vec![0u8; max.min(MAX_READ)];
                let (n, from) = self.udp(id)?.recv_from(&mut buf).map_err(io_error)?;
                buf.truncate(n);
                PlatformResponse::Datagram { bytes: buf, from: from.to_string() }
            }
            _ => unreachable!("not a socket request"),
        })
    }
}

fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, PlatformError> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| PlatformError { kind: PlatformErrorKind::InvalidData, message: format!("{host}: {e}") })?
        .collect();
    if addrs.is_empty() {
        return Err(PlatformError { kind: PlatformErrorKind::InvalidData, message: format!("{host}: no address") });
    }
    Ok(addrs)
}

fn other(message: &str) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::Other, message: message.to_string() }
}

pub(crate) fn is_socket_request(request: &PlatformRequest) -> bool {
    matches!(
        request,
        PlatformRequest::TcpConnect { .. }
            | PlatformRequest::TlsConnect { .. }
            | PlatformRequest::TcpListen { .. }
            | PlatformRequest::TlsListen { .. }
            | PlatformRequest::TcpAccept { .. }
            | PlatformRequest::SocketRead { .. }
            | PlatformRequest::SocketReadFor { .. }
            | PlatformRequest::SocketWrite { .. }
            | PlatformRequest::SocketShutdownWrite { .. }
            | PlatformRequest::SocketClose { .. }
            | PlatformRequest::LocalPort { .. }
            | PlatformRequest::PeerAddr { .. }
            | PlatformRequest::UdpBind { .. }
            | PlatformRequest::UdpSendTo { .. }
            | PlatformRequest::UdpRecvFrom { .. }
    )
}
