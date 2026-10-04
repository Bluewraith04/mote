//! What a full runtime adds to the base platform: TLS. A lean runtime leaves it out and links none of its code.

use std::any::Any;
use std::io;
use std::net::TcpStream;
use std::sync::Arc;

use contracts::{PlatformError, PlatformErrorKind};

/// An encrypted session over a TCP stream.
pub(crate) trait Secure: Send + Sync {
    fn tcp(&self) -> &Arc<TcpStream>;
    fn read(&self, buf: &mut [u8]) -> io::Result<usize>;
    fn buffered(&self, max: usize) -> Option<io::Result<Vec<u8>>>;
    fn write_all(&self, bytes: &[u8]) -> io::Result<()>;
    fn send_close_notify(&self);
}

/// A server's TLS settings, opaque to the sockets that hold them.
pub(crate) type ServerSettings = Arc<dyn Any + Send + Sync>;

/// Starts TLS sessions.
pub(crate) trait Tls: Send + Sync {
    /// Whether this runtime has TLS at all.
    fn present(&self) -> bool {
        true
    }

    /// `trust_pem` holds extra certificates to trust, or is empty.
    fn client(&self, tcp: Arc<TcpStream>, host: &str, trust_pem: &str) -> Result<Arc<dyn Secure>, PlatformError>;
    fn server_settings(&self, cert_pem: &str, key_pem: &str) -> Result<ServerSettings, PlatformError>;
    fn server(&self, tcp: Arc<TcpStream>, settings: &ServerSettings) -> Result<Arc<dyn Secure>, PlatformError>;
}

/// The TLS of a lean runtime; a plain `Option` let the optimizer fill `None` with the real TLS's vtable.
pub(crate) struct NoTls;

pub(crate) static NO_TLS: NoTls = NoTls;

impl Tls for NoTls {
    fn present(&self) -> bool {
        false
    }

    fn client(&self, _: Arc<TcpStream>, _: &str, _: &str) -> Result<Arc<dyn Secure>, PlatformError> {
        Err(missing("TLS"))
    }

    fn server_settings(&self, _: &str, _: &str) -> Result<ServerSettings, PlatformError> {
        Err(missing("TLS"))
    }

    fn server(&self, _: Arc<TcpStream>, _: &ServerSettings) -> Result<Arc<dyn Secure>, PlatformError> {
        Err(missing("TLS"))
    }
}

pub(crate) fn missing(what: &str) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::Unsupported, message: format!("{what} is not part of this runtime") }
}
