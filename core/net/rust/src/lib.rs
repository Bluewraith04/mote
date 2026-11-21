//! TLS sessions as state machines: Mote moves the bytes over a socket, this only transforms them.
use std::ffi::{CStr, c_char};
use std::io::Read;
use std::io::Write;
use std::sync::{Arc, OnceLock};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, Connection, RootCertStore, ServerConfig, ServerConnection};

const FAILED: i64 = -1;
const ENDED: i64 = -2;

struct Session {
    connection: Connection,
    message: String,
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn bundled_roots() -> RootCertStore {
    RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() }
}

fn default_client() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let config = ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions().expect("protocol versions").with_root_certificates(bundled_roots()).with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

unsafe fn bytes<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

unsafe fn bytes_mut<'a>(pointer: *mut u8, length: i64) -> &'a mut [u8] {
    if length <= 0 { &mut [] } else { unsafe { std::slice::from_raw_parts_mut(pointer, length as usize) } }
}

unsafe fn session<'a>(handle: i64) -> &'a mut Session {
    unsafe { &mut *(handle as *mut Session) }
}

fn into_handle(connection: Connection) -> i64 {
    Box::into_raw(Box::new(Session { connection, message: String::new() })) as i64
}

/// A client session for `host`; `roots` is PEM of extra trusted certificates. Returns a handle, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_client(host: *const c_char, roots: *const u8, roots_length: i64) -> i64 {
    let Ok(host) = unsafe { CStr::from_ptr(host) }.to_str() else { return FAILED };
    let Ok(name) = ServerName::try_from(host.to_string()) else { return FAILED };
    let extra = unsafe { bytes(roots, roots_length) };
    let config = if extra.is_empty() {
        default_client()
    } else {
        let mut store = bundled_roots();
        for certificate in CertificateDer::pem_slice_iter(extra) {
            let Ok(certificate) = certificate else { return FAILED };
            if store.add(certificate).is_err() {
                return FAILED;
            }
        }
        let Ok(builder) = ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions() else { return FAILED };
        Arc::new(builder.with_root_certificates(store).with_no_client_auth())
    };
    match ClientConnection::new(config, name) {
        Ok(connection) => into_handle(Connection::Client(connection)),
        Err(_) => FAILED,
    }
}

/// A server session with a certificate chain and a private key, both PEM. Returns a handle, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_server(chain: *const u8, chain_length: i64, key: *const u8, key_length: i64) -> i64 {
    let chain: Result<Vec<CertificateDer<'static>>, _> = CertificateDer::pem_slice_iter(unsafe { bytes(chain, chain_length) }).collect();
    let Ok(chain) = chain else { return FAILED };
    let Ok(key) = PrivateKeyDer::from_pem_slice(unsafe { bytes(key, key_length) }) else { return FAILED };
    let Ok(builder) = ServerConfig::builder_with_provider(provider()).with_safe_default_protocol_versions() else { return FAILED };
    let Ok(config) = builder.with_no_client_auth().with_single_cert(chain, key) else { return FAILED };
    match ServerConnection::new(Arc::new(config)) {
        Ok(connection) => into_handle(Connection::Server(connection)),
        Err(_) => FAILED,
    }
}

/// Takes bytes that came from the peer; returns how many were used, or -1 with the reason in `tls_message`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_incoming(handle: i64, data: *const u8, length: i64) -> i64 {
    let session = unsafe { session(handle) };
    let mut rest = unsafe { bytes(data, length) };
    let mut used = 0;
    while !rest.is_empty() {
        let count = match session.connection.read_tls(&mut rest) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                session.message = error.to_string();
                return FAILED;
            }
        };
        used += count;
        if let Err(error) = session.connection.process_new_packets() {
            session.message = error.to_string();
            return FAILED;
        }
    }
    used as i64
}

/// Puts bytes to send to the peer at the start of the buffer; returns how many, 0 when there are none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_outgoing(handle: i64, buffer: *mut u8, length: i64) -> i64 {
    let session = unsafe { session(handle) };
    if !session.connection.wants_write() {
        return 0;
    }
    let mut out = unsafe { bytes_mut(buffer, length) };
    match session.connection.write_tls(&mut out) {
        Ok(count) => count as i64,
        Err(error) => {
            session.message = error.to_string();
            FAILED
        }
    }
}

/// 1 when the session still needs bytes from the peer to go on, else 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_wants_read(handle: i64) -> i64 {
    unsafe { session(handle) }.connection.wants_read() as i64
}

/// 1 when bytes wait to be sent to the peer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_wants_write(handle: i64) -> i64 {
    unsafe { session(handle) }.connection.wants_write() as i64
}

/// 1 while the handshake is not finished.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_handshaking(handle: i64) -> i64 {
    unsafe { session(handle) }.connection.is_handshaking() as i64
}

/// Reads decrypted bytes: the count, 0 when none are ready, -2 after the peer closed cleanly, -1 on an error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_plain_read(handle: i64, buffer: *mut u8, length: i64) -> i64 {
    let session = unsafe { session(handle) };
    let out = unsafe { bytes_mut(buffer, length) };
    match session.connection.reader().read(out) {
        Ok(0) => ENDED,
        Ok(count) => count as i64,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => 0,
        Err(error) => {
            session.message = error.to_string();
            FAILED
        }
    }
}

/// Queues bytes to encrypt; returns how many were taken, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_plain_write(handle: i64, data: *const u8, length: i64) -> i64 {
    let session = unsafe { session(handle) };
    match session.connection.writer().write(unsafe { bytes(data, length) }) {
        Ok(count) => count as i64,
        Err(error) => {
            session.message = error.to_string();
            FAILED
        }
    }
}

/// Queues the close notice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_send_close(handle: i64) -> i64 {
    unsafe { session(handle) }.connection.send_close_notify();
    0
}

/// Copies the last error text to the buffer; returns its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_message(handle: i64, buffer: *mut u8, length: i64) -> i64 {
    let session = unsafe { session(handle) };
    let out = unsafe { bytes_mut(buffer, length) };
    let count = session.message.len().min(out.len());
    out[..count].copy_from_slice(&session.message.as_bytes()[..count]);
    count as i64
}

/// Frees a session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tls_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Session) });
    }
    0
}
