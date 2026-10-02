use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use contracts::{
    EventSink, FileKind, FileMode, FileStat, Platform, PlatformError, PlatformErrorKind, PlatformRequest, ProcessOutput,
    PlatformResponse,
    SourceHandle, SourceRequest, StdStream, Wake, Whence, Woken,
};

use crate::dynlib::DynLibs;
use crate::net::{is_socket_request, Readiness, SystemNet};
use crate::reactor::Reactor;
use crate::sql::{is_sql_request, SystemSql};

type Handle = Arc<Mutex<BufReader<File>>>;

/// The real machine.
pub struct SystemPlatform {
    origin: Instant,
    args: RwLock<Vec<String>>,
    handles: Mutex<HashMap<i64, Handle>>,
    next_handle: AtomicI64,
    net: SystemNet,
    sql: SystemSql,
    libs: DynLibs,
    reactor: OnceLock<Option<Arc<Reactor>>>,
}

impl Drop for SystemPlatform {
    fn drop(&mut self) {
        if let Some(Some(reactor)) = self.reactor.get() {
            reactor.shutdown();
        }
    }
}

impl SystemPlatform {
    pub fn new(args: Vec<String>) -> Self {
        SystemPlatform {
            origin: Instant::now(),
            args: RwLock::new(args),
            handles: Mutex::new(HashMap::new()),
            next_handle: AtomicI64::new(1),
            net: SystemNet::new(),
            sql: SystemSql::new(),
            libs: DynLibs::new(),
            reactor: OnceLock::new(),
        }
    }

    fn reactor(&self) -> Option<&Arc<Reactor>> {
        self.reactor.get_or_init(|| Reactor::start().ok()).as_ref()
    }

    pub fn set_args(&self, args: Vec<String>) {
        *self.args.write().unwrap() = args;
    }

    /// Whether `LibOpen` may load native libraries (`--allow-native`).
    pub fn set_allow_native(&self, allowed: bool) {
        self.libs.set_allowed(allowed);
    }

    fn handle(&self, id: i64) -> Result<Handle, PlatformError> {
        self.handles.lock().unwrap().get(&id).cloned().ok_or_else(closed_error)
    }

    fn open(&self, path: &str, mode: FileMode) -> Result<i64, PlatformError> {
        let mut options = OpenOptions::new();
        match mode {
            FileMode::Read => options.read(true),
            FileMode::Write => options.write(true).create(true).truncate(true),
            FileMode::Append => options.append(true).create(true),
        };
        let file = options.open(path).map_err(io_error)?;
        let id = self.next_handle.fetch_add(1, Ordering::SeqCst);
        self.handles.lock().unwrap().insert(id, Arc::new(Mutex::new(BufReader::new(file))));
        Ok(id)
    }

    fn file_request(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        Ok(match request {
            PlatformRequest::OpenFile { path, mode } => PlatformResponse::Int(self.open(&path, mode)?),
            PlatformRequest::FileClose { id } => {
                self.handles.lock().unwrap().remove(&id).ok_or_else(closed_error)?;
                PlatformResponse::Unit
            }
            PlatformRequest::FileRead { id, max } => {
                let handle = self.handle(id)?;
                let mut reader = handle.lock().unwrap();
                let mut buf = Vec::new();
                reader.by_ref().take(max as u64).read_to_end(&mut buf).map_err(io_error)?;
                PlatformResponse::Bytes(buf)
            }
            PlatformRequest::FileReadLine { id } => {
                let handle = self.handle(id)?;
                let mut reader = handle.lock().unwrap();
                let mut line = String::new();
                if reader.read_line(&mut line).map_err(io_error)? == 0 {
                    return Ok(PlatformResponse::Line(None));
                }
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                PlatformResponse::Line(Some(line))
            }
            PlatformRequest::FileWrite { id, bytes } => {
                let handle = self.handle(id)?;
                let mut reader = handle.lock().unwrap();
                reader.get_mut().write_all(&bytes).map_err(io_error)?;
                PlatformResponse::Unit
            }
            PlatformRequest::FileSeek { id, offset, whence } => {
                let handle = self.handle(id)?;
                let mut reader = handle.lock().unwrap();
                let target = match whence {
                    Whence::Start => SeekFrom::Start(u64::try_from(offset).map_err(|_| negative_seek())?),
                    Whence::Current => SeekFrom::Current(offset),
                    Whence::End => SeekFrom::End(offset),
                };
                PlatformResponse::Int(reader.seek(target).map_err(io_error)? as i64)
            }
            _ => unreachable!("not a file request"),
        })
    }
}

fn closed_error() -> PlatformError {
    PlatformError { kind: PlatformErrorKind::Other, message: "file is closed".to_string() }
}

fn negative_seek() -> PlatformError {
    PlatformError { kind: PlatformErrorKind::InvalidData, message: "seek before the start of the file".to_string() }
}

impl Platform for SystemPlatform {
    fn execute(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        Ok(match request {
            PlatformRequest::WallClockMillis => {
                let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
                PlatformResponse::Int(ms)
            }
            PlatformRequest::MonotonicNanos => PlatformResponse::Int(self.origin.elapsed().as_nanos() as i64),
            PlatformRequest::Entropy => PlatformResponse::Int(entropy()),
            PlatformRequest::Write { stream, bytes } => {
                write_flush(stream, &bytes);
                PlatformResponse::Unit
            }
            PlatformRequest::Flush { stream } => {
                write_flush(stream, &[]);
                PlatformResponse::Unit
            }
            PlatformRequest::EnvVar { name } => PlatformResponse::OptionString(std::env::var(&name).ok()),
            PlatformRequest::EnvVars => PlatformResponse::Pairs(std::env::vars().collect()),
            PlatformRequest::Args => PlatformResponse::Strings(self.args.read().unwrap().clone()),
            PlatformRequest::ReadLine => read_line()?,
            PlatformRequest::ReadFile { path } => {
                PlatformResponse::Text(std::fs::read_to_string(path).map_err(io_error)?)
            }
            PlatformRequest::ReadFileBytes { path } => {
                PlatformResponse::Bytes(std::fs::read(path).map_err(io_error)?)
            }
            PlatformRequest::WriteFile { path, bytes } => {
                std::fs::write(path, bytes).map_err(io_error)?;
                PlatformResponse::Unit
            }
            PlatformRequest::Sleep { nanos } => {
                std::thread::sleep(std::time::Duration::from_nanos(nanos));
                PlatformResponse::Unit
            }
            PlatformRequest::LocalOffset { unix_seconds } => PlatformResponse::Int(local_offset(unix_seconds)?),
            dir @ (PlatformRequest::ListDir { .. }
            | PlatformRequest::CreateDir { .. }
            | PlatformRequest::RemoveFile { .. }
            | PlatformRequest::RemoveDir { .. }
            | PlatformRequest::Rename { .. }
            | PlatformRequest::Stat { .. }) => return dir_request(dir),
            PlatformRequest::RunProcess { program, args, env, stdin, cwd } => {
                return run_process(program, args, env, stdin, cwd)
            }
            PlatformRequest::HttpRequest { method, url, headers, body, timeout_millis, max_redirects, max_body } => {
                return crate::http::request(&method, &url, &headers, &body, timeout_millis, max_redirects, max_body)
            }
            PlatformRequest::LibOpen { path } => PlatformResponse::Int(self.libs.open(&path)?),
            PlatformRequest::LibSymbol { lib, name, signature } => PlatformResponse::Int(self.libs.symbol(lib, &name, &signature)?),
            PlatformRequest::LibCall { symbol, args, .. } => return self.libs.call(symbol, args),
            PlatformRequest::LibClose { lib } => return self.libs.close(lib),
            socket if is_socket_request(&socket) => return self.net.request(socket),
            sql if is_sql_request(&sql) => return self.sql.request(sql),
            file => return self.file_request(file),
        })
    }

    fn blocking(&self, request: &PlatformRequest) -> bool {
        is_socket_request(request)
            || is_sql_request(request)
            || matches!(request, PlatformRequest::LibCall { blocking: true, .. })
            || matches!(
            request,
            PlatformRequest::ReadLine
                | PlatformRequest::OpenFile { .. }
                | PlatformRequest::FileRead { .. }
                | PlatformRequest::FileReadLine { .. }
                | PlatformRequest::FileWrite { .. }
                | PlatformRequest::FileSeek { .. }
                | PlatformRequest::FileClose { .. }
                | PlatformRequest::ListDir { .. }
                | PlatformRequest::CreateDir { .. }
                | PlatformRequest::RemoveFile { .. }
                | PlatformRequest::RemoveDir { .. }
                | PlatformRequest::Rename { .. }
                | PlatformRequest::Stat { .. }
                | PlatformRequest::RunProcess { .. }
                | PlatformRequest::HttpRequest { .. }
                | PlatformRequest::ReadFile { .. }
                | PlatformRequest::ReadFileBytes { .. }
                | PlatformRequest::WriteFile { .. }
                | PlatformRequest::Sleep { .. }
        )
    }

    fn wait(&self, request: PlatformRequest, wake: Wake) -> Result<(), Box<(PlatformRequest, Wake)>> {
        let Some(reactor) = self.reactor() else { return Err(Box::new((request, wake))) };
        if let PlatformRequest::Sleep { nanos } = request {
            reactor.sleep(Duration::from_nanos(nanos), wake);
            return Ok(());
        }
        match self.net.readiness(&request) {
            Readiness::Now(result) => {
                wake(Woken::Done(result));
                Ok(())
            }
            Readiness::Wait(source, limit) => reactor.readable(source, request, limit, wake),
            Readiness::Run => Err(Box::new((request, wake))),
        }
    }

    fn open_source(&self, request: SourceRequest, sink: EventSink) -> Result<SourceHandle, PlatformError> {
        let SourceRequest::Timer { period_nanos } = request;
        let reactor = self
            .reactor()
            .ok_or_else(|| PlatformError { kind: PlatformErrorKind::Other, message: "the reactor could not start".to_string() })?
            .clone();
        let key = reactor.ticker(Duration::from_nanos(period_nanos.max(1)), sink);
        Ok(SourceHandle::new(move || reactor.cancel(key)))
    }
}

fn read_line() -> Result<PlatformResponse, PlatformError> {
    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line).map_err(io_error)?;
    if n == 0 {
        return Ok(PlatformResponse::Line(None));
    }
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(PlatformResponse::Line(Some(line)))
}

pub(crate) fn io_error(e: std::io::Error) -> PlatformError {
    use std::io::ErrorKind::*;
    let kind = match e.kind() {
        NotFound => PlatformErrorKind::NotFound,
        PermissionDenied => PlatformErrorKind::PermissionDenied,
        InvalidData => PlatformErrorKind::InvalidData,
        Interrupted => PlatformErrorKind::Interrupted,
        AlreadyExists => PlatformErrorKind::AlreadyExists,
        WouldBlock => PlatformErrorKind::WouldBlock,
        TimedOut => PlatformErrorKind::TimedOut,
        Unsupported => PlatformErrorKind::Unsupported,
        _ => PlatformErrorKind::Other,
    };
    PlatformError { kind, message: e.to_string() }
}

#[cfg(unix)]
fn local_offset(unix_seconds: i64) -> Result<i64, PlatformError> {
    let t = unix_seconds as libc::time_t;
    // SAFETY: `libc::tm` is plain C data for which all-zero is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `t` and `tm` are valid for the call; `localtime_r` is the reentrant form.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return Err(PlatformError { kind: PlatformErrorKind::InvalidData, message: "time is out of range".to_string() });
    }
    Ok(tm.tm_gmtoff as i64)
}

#[cfg(not(unix))]
fn local_offset(_unix_seconds: i64) -> Result<i64, PlatformError> {
    Err(PlatformError { kind: PlatformErrorKind::Unsupported, message: "no local time zone on this platform".to_string() })
}

fn write_flush(stream: StdStream, bytes: &[u8]) {
    match stream {
        StdStream::Stdout => {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(bytes);
            let _ = out.flush();
        }
        StdStream::Stderr => {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(bytes);
            let _ = err.flush();
        }
    }
}

fn entropy() -> i64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0));
    h.finish() as i64
}

fn dir_request(request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
    Ok(match request {
        PlatformRequest::ListDir { path } => {
            let mut names = Vec::new();
            for entry in std::fs::read_dir(path).map_err(io_error)? {
                names.push(entry.map_err(io_error)?.file_name().to_string_lossy().into_owned());
            }
            names.sort();
            PlatformResponse::Strings(names)
        }
        PlatformRequest::CreateDir { path, recursive: true } => {
            std::fs::create_dir_all(path).map_err(io_error)?;
            PlatformResponse::Unit
        }
        PlatformRequest::CreateDir { path, recursive: false } => {
            std::fs::create_dir(path).map_err(io_error)?;
            PlatformResponse::Unit
        }
        PlatformRequest::RemoveFile { path } => {
            std::fs::remove_file(path).map_err(io_error)?;
            PlatformResponse::Unit
        }
        PlatformRequest::RemoveDir { path, recursive: true } => {
            std::fs::remove_dir_all(path).map_err(io_error)?;
            PlatformResponse::Unit
        }
        PlatformRequest::RemoveDir { path, recursive: false } => {
            std::fs::remove_dir(path).map_err(io_error)?;
            PlatformResponse::Unit
        }
        PlatformRequest::Rename { from, to } => {
            std::fs::rename(from, to).map_err(io_error)?;
            PlatformResponse::Unit
        }
        PlatformRequest::Stat { path } => {
            let meta = std::fs::metadata(path).map_err(io_error)?;
            let kind = if meta.is_file() {
                FileKind::File
            } else if meta.is_dir() {
                FileKind::Dir
            } else {
                FileKind::Other
            };
            let modified_millis = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64);
            PlatformResponse::Stat(FileStat { kind, size: meta.len() as i64, modified_millis })
        }
        _ => unreachable!("not a directory request"),
    })
}

fn run_process(
    program: String,
    args: Vec<String>,
    env: Vec<String>,
    stdin: Vec<u8>,
    cwd: String,
) -> Result<PlatformResponse, PlatformError> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for pair in env {
        let (key, value) = pair.split_once('=').unwrap_or((pair.as_str(), ""));
        command.env(key, value);
    }
    if !cwd.is_empty() {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(io_error)?;
    let mut pipe = child.stdin.take().expect("stdin was piped");
    let feeder = std::thread::spawn(move || {
        let _ = pipe.write_all(&stdin);
    });
    let output = child.wait_with_output().map_err(io_error)?;
    let _ = feeder.join();
    Ok(PlatformResponse::Process(ProcessOutput {
        status: output.status.code().map_or(-1, i64::from),
        stdout: output.stdout,
        stderr: output.stderr,
    }))
}
