//! The fake platform's network: loopback only, in memory, never waits.

use std::collections::{BTreeMap, VecDeque};

use contracts::{PlatformError, PlatformErrorKind, PlatformRequest, PlatformResponse};

const FIRST_EPHEMERAL: u16 = 49152;

struct Stream {
    inbox: VecDeque<u8>,
    peer: Option<i64>,
    peer_done: bool,
    write_shut: bool,
    local_port: u16,
    peer_port: u16,
}

struct Listener {
    port: u16,
    backlog: VecDeque<i64>,
}

struct Udp {
    port: u16,
    queue: VecDeque<(Vec<u8>, String)>,
}

enum Socket {
    Stream(Stream),
    Listener(Listener),
    Udp(Udp),
}

pub(crate) struct FakeNet {
    next_id: i64,
    next_port: u16,
    sockets: BTreeMap<i64, Socket>,
    tcp_ports: BTreeMap<u16, i64>,
    udp_ports: BTreeMap<u16, i64>,
}

fn error(kind: PlatformErrorKind, message: &str) -> PlatformError {
    PlatformError { kind, message: message.to_string() }
}

fn closed() -> PlatformError {
    error(PlatformErrorKind::Other, "socket is closed")
}

fn would_block(message: &str) -> PlatformError {
    error(PlatformErrorKind::WouldBlock, message)
}

fn check_host(host: &str) -> Result<(), PlatformError> {
    match host {
        "" | "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" => Ok(()),
        _ => Err(error(PlatformErrorKind::Other, &format!("{host}: host unreachable"))),
    }
}

impl FakeNet {
    pub(crate) fn new() -> Self {
        FakeNet {
            next_id: 0,
            next_port: FIRST_EPHEMERAL,
            sockets: BTreeMap::new(),
            tcp_ports: BTreeMap::new(),
            udp_ports: BTreeMap::new(),
        }
    }

    fn id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    fn ephemeral(&mut self) -> u16 {
        let port = self.next_port;
        self.next_port += 1;
        port
    }

    fn stream(&mut self, id: i64) -> Result<&mut Stream, PlatformError> {
        match self.sockets.get_mut(&id) {
            Some(Socket::Stream(s)) => Ok(s),
            Some(_) => Err(error(PlatformErrorKind::Other, "socket is not a TCP stream")),
            None => Err(closed()),
        }
    }

    fn udp(&mut self, id: i64) -> Result<&mut Udp, PlatformError> {
        match self.sockets.get_mut(&id) {
            Some(Socket::Udp(s)) => Ok(s),
            Some(_) => Err(error(PlatformErrorKind::Other, "socket is not a UDP socket")),
            None => Err(closed()),
        }
    }

    fn peer_stream(&mut self, id: i64) -> Option<&mut Stream> {
        match self.sockets.get_mut(&id) {
            Some(Socket::Stream(s)) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn request(&mut self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        Ok(match request {
            PlatformRequest::TlsConnect { .. } | PlatformRequest::TlsListen { .. } => {
                return Err(error(PlatformErrorKind::Unsupported, "the fake platform has no TLS"));
            }
            PlatformRequest::TcpListen { host, port } => {
                check_host(&host)?;
                let port = if port == 0 { self.ephemeral() } else { port };
                if self.tcp_ports.contains_key(&port) {
                    return Err(error(PlatformErrorKind::AlreadyExists, "address already in use"));
                }
                let id = self.id();
                self.sockets.insert(id, Socket::Listener(Listener { port, backlog: VecDeque::new() }));
                self.tcp_ports.insert(port, id);
                PlatformResponse::Int(id)
            }
            PlatformRequest::TcpConnect { host, port } => {
                check_host(&host)?;
                let listener = *self.tcp_ports.get(&port).ok_or_else(|| error(PlatformErrorKind::Other, "connection refused"))?;
                let client_port = self.ephemeral();
                let (client, server) = (self.id(), self.id());
                let end = |peer, local_port, peer_port| {
                    Socket::Stream(Stream {
                        inbox: VecDeque::new(),
                        peer: Some(peer),
                        peer_done: false,
                        write_shut: false,
                        local_port,
                        peer_port,
                    })
                };
                self.sockets.insert(client, end(server, client_port, port));
                self.sockets.insert(server, end(client, port, client_port));
                if let Some(Socket::Listener(l)) = self.sockets.get_mut(&listener) {
                    l.backlog.push_back(server);
                }
                PlatformResponse::Int(client)
            }
            PlatformRequest::TcpAccept { id } => match self.sockets.get_mut(&id) {
                Some(Socket::Listener(l)) => {
                    PlatformResponse::Int(l.backlog.pop_front().ok_or_else(|| would_block("no pending connection"))?)
                }
                Some(_) => return Err(error(PlatformErrorKind::Other, "socket is not a listener")),
                None => return Err(closed()),
            },
            PlatformRequest::SocketRead { id, max } | PlatformRequest::SocketReadFor { id, max, .. } => {
                let s = self.stream(id)?;
                if s.inbox.is_empty() {
                    if s.peer_done || s.peer.is_none() {
                        return Ok(PlatformResponse::Bytes(Vec::new()));
                    }
                    return Err(would_block("no data yet"));
                }
                let n = max.min(s.inbox.len());
                PlatformResponse::Bytes(s.inbox.drain(..n).collect())
            }
            PlatformRequest::SocketWrite { id, bytes } => {
                let s = self.stream(id)?;
                let broken = || error(PlatformErrorKind::Other, "broken pipe");
                let peer = match (s.write_shut, s.peer) {
                    (false, Some(peer)) => peer,
                    _ => return Err(broken()),
                };
                self.peer_stream(peer).ok_or_else(broken)?.inbox.extend(bytes);
                PlatformResponse::Unit
            }
            PlatformRequest::SocketShutdownWrite { id } => {
                let s = self.stream(id)?;
                s.write_shut = true;
                if let Some(peer) = s.peer
                    && let Some(p) = self.peer_stream(peer) {
                        p.peer_done = true;
                    }
                PlatformResponse::Unit
            }
            PlatformRequest::SocketClose { id } => {
                match self.sockets.remove(&id).ok_or_else(closed)? {
                    Socket::Stream(s) => {
                        if let Some(p) = s.peer.and_then(|peer| self.peer_stream(peer)) {
                            p.peer = None;
                            p.peer_done = true;
                        }
                    }
                    Socket::Listener(l) => {
                        self.tcp_ports.remove(&l.port);
                    }
                    Socket::Udp(u) => {
                        self.udp_ports.remove(&u.port);
                    }
                }
                PlatformResponse::Unit
            }
            PlatformRequest::LocalPort { id } => PlatformResponse::Int(i64::from(match self.sockets.get(&id) {
                Some(Socket::Stream(s)) => s.local_port,
                Some(Socket::Listener(l)) => l.port,
                Some(Socket::Udp(u)) => u.port,
                None => return Err(closed()),
            })),
            PlatformRequest::PeerAddr { id } => {
                PlatformResponse::Text(format!("127.0.0.1:{}", self.stream(id)?.peer_port))
            }
            PlatformRequest::UdpBind { host, port } => {
                check_host(&host)?;
                let port = if port == 0 { self.ephemeral() } else { port };
                if self.udp_ports.contains_key(&port) {
                    return Err(error(PlatformErrorKind::AlreadyExists, "address already in use"));
                }
                let id = self.id();
                self.sockets.insert(id, Socket::Udp(Udp { port, queue: VecDeque::new() }));
                self.udp_ports.insert(port, id);
                PlatformResponse::Int(id)
            }
            PlatformRequest::UdpSendTo { id, host, port, bytes } => {
                check_host(&host)?;
                let from = format!("127.0.0.1:{}", self.udp(id)?.port);
                let sent = bytes.len() as i64;
                if let Some(dest) = self.udp_ports.get(&port).copied() {
                    self.udp(dest)?.queue.push_back((bytes, from));
                }
                PlatformResponse::Int(sent)
            }
            PlatformRequest::UdpRecvFrom { id, max } => {
                let (mut bytes, from) = self.udp(id)?.queue.pop_front().ok_or_else(|| would_block("no datagram yet"))?;
                bytes.truncate(max);
                PlatformResponse::Datagram { bytes, from }
            }
            _ => unreachable!("not a socket request"),
        })
    }
}
