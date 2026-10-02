//! `HttpRequest` over the `ureq` crate: blocking, HTTPS through rustls, redirects and gzip.

use std::time::Duration;

use contracts::{PlatformError, PlatformErrorKind, PlatformResponse};

fn failure(kind: PlatformErrorKind, e: impl std::fmt::Display) -> PlatformError {
    PlatformError { kind, message: e.to_string() }
}

fn kind_of(e: &ureq::Error) -> PlatformErrorKind {
    match e {
        ureq::Error::Timeout(_) => PlatformErrorKind::TimedOut,
        ureq::Error::HostNotFound => PlatformErrorKind::NotFound,
        ureq::Error::BadUri(_) | ureq::Error::Http(_) | ureq::Error::Protocol(_) | ureq::Error::BodyExceedsLimit(_) => PlatformErrorKind::InvalidData,
        ureq::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::NotFound => PlatformErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => PlatformErrorKind::PermissionDenied,
            std::io::ErrorKind::TimedOut => PlatformErrorKind::TimedOut,
            _ => PlatformErrorKind::Other,
        },
        _ => PlatformErrorKind::Other,
    }
}

pub(crate) fn request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    timeout_millis: u64,
    max_redirects: u32,
    max_body: u64,
) -> Result<PlatformResponse, PlatformError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(timeout_millis.max(1))))
        .max_redirects(max_redirects)
        .max_redirects_will_error(true)
        .http_status_as_error(false)
        .build()
        .into();
    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let send_body = body.to_vec();
    let request = builder.body(send_body).map_err(|e| failure(PlatformErrorKind::InvalidData, e))?;
    let mut response = agent.run(request).map_err(|e| failure(kind_of(&e), &e))?;
    let status = i64::from(response.status().as_u16());
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str().to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned()))
        .collect();
    let body = response
        .body_mut()
        .with_config()
        .limit(max_body.saturating_add(1))
        .read_to_vec()
        .map_err(|e| failure(kind_of(&e), &e))?;
    if body.len() as u64 > max_body {
        return Err(failure(PlatformErrorKind::InvalidData, format!("the body is over {max_body} bytes")));
    }
    Ok(PlatformResponse::Http { status, headers, body })
}
