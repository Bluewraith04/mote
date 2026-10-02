//! TLS over sockets: a rustls session behind the same socket id as a plain stream.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};

use contracts::{PlatformError, PlatformErrorKind};
use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, Stream};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName};

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn invalid(message: impl std::fmt::Display) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::InvalidData, message: message.to_string() }
}

fn client_config(extra: &[CertificateDer<'static>]) -> Result<Arc<ClientConfig>, PlatformError> {
    let mut roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    for cert in extra {
        roots.add(cert.clone()).map_err(invalid)?;
    }
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(invalid)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn default_client_config() -> Result<Arc<ClientConfig>, PlatformError> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(config) = CONFIG.get() {
        return Ok(config.clone());
    }
    let config = client_config(&[])?;
    Ok(CONFIG.get_or_init(|| config).clone())
}

/// A server configuration from a PEM certificate chain and a PEM private key.
pub fn server_config(cert_pem: &str, key_pem: &str) -> Result<Arc<ServerConfig>, PlatformError> {
    let certs = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid(format!("certificate: {e}")))?;
    if certs.is_empty() {
        return Err(invalid("certificate: no certificate found"));
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(|e| invalid(format!("private key: {e}")))?;
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(invalid)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| invalid(format!("certificate and key: {e}")))?;
    Ok(Arc::new(config))
}

/// The certificates in `pem`, for trusting a test server.
pub fn certificates(pem: &str) -> Result<Vec<CertificateDer<'static>>, PlatformError> {
    CertificateDer::pem_slice_iter(pem.as_bytes()).collect::<Result<Vec<_>, _>>().map_err(|e| invalid(format!("certificate: {e}")))
}

enum Conn {
    Client(ClientConnection),
    Server(ServerConnection),
}

pub(crate) struct Session {
    tcp: Arc<TcpStream>,
    conn: Mutex<Conn>,
}

impl Session {
    pub(crate) fn client(tcp: Arc<TcpStream>, host: &str, extra: &[CertificateDer<'static>]) -> Result<Session, PlatformError> {
        let config = if extra.is_empty() { default_client_config()? } else { client_config(extra)? };
        let name = ServerName::try_from(host.to_string()).map_err(|e| invalid(format!("{host}: {e}")))?;
        let mut conn = ClientConnection::new(config, name).map_err(invalid)?;
        let mut sock: &TcpStream = &tcp;
        while conn.is_handshaking() {
            conn.complete_io(&mut sock).map_err(crate::system::io_error)?;
        }
        Ok(Session { tcp: tcp.clone(), conn: Mutex::new(Conn::Client(conn)) })
    }

    pub(crate) fn server(tcp: Arc<TcpStream>, config: Arc<ServerConfig>) -> Result<Session, PlatformError> {
        let conn = ServerConnection::new(config).map_err(invalid)?;
        Ok(Session { tcp, conn: Mutex::new(Conn::Server(conn)) })
    }

    pub(crate) fn tcp(&self) -> &Arc<TcpStream> {
        &self.tcp
    }

    pub(crate) fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let mut sock: &TcpStream = &self.tcp;
        let result = match &mut *conn {
            Conn::Client(c) => Stream::new(c, &mut sock).read(buf),
            Conn::Server(c) => Stream::new(c, &mut sock).read(buf),
        };
        match result {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
            other => other,
        }
    }

    pub(crate) fn buffered(&self, max: usize) -> Option<io::Result<Vec<u8>>> {
        let mut conn = self.conn.try_lock().ok()?;
        let mut buf = vec![0u8; max];
        let result = match &mut *conn {
            Conn::Client(c) => c.reader().read(&mut buf),
            Conn::Server(c) => c.reader().read(&mut buf),
        };
        match result {
            Ok(n) => {
                buf.truncate(n);
                Some(Ok(buf))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => None,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Some(Ok(Vec::new())),
            Err(e) => Some(Err(e)),
        }
    }

    pub(crate) fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let mut sock: &TcpStream = &self.tcp;
        match &mut *conn {
            Conn::Client(c) => {
                let mut stream = Stream::new(c, &mut sock);
                stream.write_all(bytes)?;
                stream.flush()
            }
            Conn::Server(c) => {
                let mut stream = Stream::new(c, &mut sock);
                stream.write_all(bytes)?;
                stream.flush()
            }
        }
    }

    pub(crate) fn send_close_notify(&self) {
        let Ok(mut conn) = self.conn.try_lock() else { return };
        let mut sock: &TcpStream = &self.tcp;
        match &mut *conn {
            Conn::Client(c) => {
                c.send_close_notify();
                let _ = c.write_tls(&mut sock);
            }
            Conn::Server(c) => {
                c.send_close_notify();
                let _ = c.write_tls(&mut sock);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use contracts::{PlatformRequest, PlatformResponse};

    use super::*;
    use crate::net::SystemNet;

    fn identity() -> (String, String) {
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        (made.cert.pem(), made.signing_key.serialize_pem())
    }

    fn listen(net: &SystemNet, cert: &str, key: &str) -> (i64, u16) {
        let request = PlatformRequest::TlsListen { host: "127.0.0.1".into(), port: 0, cert_pem: cert.into(), key_pem: key.into() };
        let PlatformResponse::Int(id) = net.request(request).unwrap() else { panic!("not an id") };
        let PlatformResponse::Int(port) = net.request(PlatformRequest::LocalPort { id }).unwrap() else { panic!("not a port") };
        (id, port as u16)
    }

    fn bytes(net: &SystemNet, id: i64, max: usize) -> Vec<u8> {
        match net.request(PlatformRequest::SocketRead { id, max }).unwrap() {
            PlatformResponse::Bytes(b) => b,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_session_carries_bytes_both_ways_and_ends_cleanly() {
        let (cert, key) = identity();
        let net = Arc::new(SystemNet::new());
        let (listener, port) = listen(&net, &cert, &key);
        let server_net = net.clone();
        let server = std::thread::spawn(move || {
            let PlatformResponse::Int(conn) = server_net.request(PlatformRequest::TcpAccept { id: listener }).unwrap() else { panic!() };
            assert_eq!(bytes(&server_net, conn, 5), b"hello");
            server_net.request(PlatformRequest::SocketWrite { id: conn, bytes: b"world".to_vec() }).unwrap();
            server_net.request(PlatformRequest::SocketShutdownWrite { id: conn }).unwrap();
            assert!(bytes(&server_net, conn, 8).is_empty());
        });
        let roots = certificates(&cert).unwrap();
        let client = net.tls_connect("localhost", port, &roots).unwrap();
        net.request(PlatformRequest::SocketWrite { id: client, bytes: b"hello".to_vec() }).unwrap();
        assert_eq!(bytes(&net, client, 5), b"world");
        assert!(bytes(&net, client, 8).is_empty());
        net.request(PlatformRequest::SocketShutdownWrite { id: client }).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn an_unknown_certificate_is_refused_while_connecting() {
        let (cert, key) = identity();
        let net = Arc::new(SystemNet::new());
        let (listener, port) = listen(&net, &cert, &key);
        let server_net = net.clone();
        let server = std::thread::spawn(move || {
            let PlatformResponse::Int(conn) = server_net.request(PlatformRequest::TcpAccept { id: listener }).unwrap() else { panic!() };
            let _ = server_net.request(PlatformRequest::SocketRead { id: conn, max: 1 });
        });
        let err = net.tls_connect("localhost", port, &[]).unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidData);
        assert!(err.message.contains("UnknownIssuer"), "{}", err.message);
        server.join().unwrap();
    }

    #[test]
    fn a_wrong_host_name_is_refused() {
        let (cert, key) = identity();
        let net = Arc::new(SystemNet::new());
        let (listener, port) = listen(&net, &cert, &key);
        let server_net = net.clone();
        let server = std::thread::spawn(move || {
            let PlatformResponse::Int(conn) = server_net.request(PlatformRequest::TcpAccept { id: listener }).unwrap() else { panic!() };
            let _ = server_net.request(PlatformRequest::SocketRead { id: conn, max: 1 });
        });
        let roots = certificates(&cert).unwrap();
        let err = net.tls_connect("127.0.0.1", port, &roots).unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidData, "{}", err.message);
        server.join().unwrap();
    }

    #[test]
    fn a_bad_certificate_or_key_is_refused_when_listening() {
        let (cert, key) = identity();
        let net = SystemNet::new();
        for (c, k, want) in [("", key.as_str(), "no certificate"), (cert.as_str(), "", "private key"), ("junk", "junk", "certificate")] {
            let request = PlatformRequest::TlsListen { host: "127.0.0.1".into(), port: 0, cert_pem: c.into(), key_pem: k.into() };
            let err = net.request(request).unwrap_err();
            assert_eq!(err.kind, PlatformErrorKind::InvalidData);
            assert!(err.message.contains(want), "{}", err.message);
        }
    }
}
