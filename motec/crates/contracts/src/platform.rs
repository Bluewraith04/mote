//! The outside world as requests: clock, entropy, stdio, environment.

/// Which standard stream a write or flush addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdStream {
    Stdout,
    Stderr,
}

/// How `OpenFile` opens: `Write` creates or truncates, `Append` creates and writes at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    Read,
    Write,
    Append,
}

/// The origin a `FileSeek` offset is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whence {
    Start,
    Current,
    End,
}

/// What a path names; ordinals are what `std.sys.fs` sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    File = 0,
    Dir = 1,
    Other = 2,
}

/// The answer to `Stat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub kind: FileKind,
    pub size: i64,
    pub modified_millis: i64,
}

/// What a finished child left behind; `status` is `-1` when a signal ended it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessOutput {
    pub status: i64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// One argument of a foreign call; the symbol's signature says how a `Word` is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CArg {
    /// An `Int`, a `Float`'s bits, a `Bool` as 0 or 1, or a `Bytes` backing's address.
    Word(u64),
    /// A borrowed NUL-terminated string.
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A request to the outside world.
pub enum PlatformRequest {
    /// Milliseconds since the Unix epoch.
    WallClockMillis,
    /// Nanoseconds since this platform's first monotonic reading.
    MonotonicNanos,
    Entropy,
    /// Write, then flush, so a line is atomic with respect to other writers.
    Write { stream: StdStream, bytes: Vec<u8> },
    Flush { stream: StdStream },
    EnvVar { name: String },
    EnvVars,
    /// The program's own arguments, `[0]` the program itself.
    Args,
    /// One line of stdin, without its line ending.
    ReadLine,
    /// The whole file as UTF-8 text.
    ReadFile { path: String },
    ReadFileBytes { path: String },
    /// Creates or replaces the file.
    WriteFile { path: String, bytes: Vec<u8> },
    Sleep { nanos: u64 },
    /// Opens a file; answers `Int(id)`, an id never reused by this platform.
    OpenFile { path: String, mode: FileMode },
    /// Up to `max` bytes; empty at end of file.
    FileRead { id: i64, max: usize },
    /// One line without its ending; `Line(None)` at end of file.
    FileReadLine { id: i64 },
    FileWrite { id: i64, bytes: Vec<u8> },
    /// Answers `Int(position)` from the start.
    FileSeek { id: i64, offset: i64, whence: Whence },
    FileClose { id: i64 },
    /// Entry names, sorted, without `.` and `..`.
    ListDir { path: String },
    /// An existing directory is `AlreadyExists` unless `recursive`, which also creates the parents.
    CreateDir { path: String, recursive: bool },
    RemoveFile { path: String },
    /// A non-empty directory needs `recursive`.
    RemoveDir { path: String, recursive: bool },
    /// Replaces a file at `to`.
    Rename { from: String, to: String },
    /// Follows symlinks.
    Stat { path: String },
    /// Runs `program` to completion and captures its output. `env` is `KEY=VALUE` overrides; empty `cwd` inherits.
    RunProcess { program: String, args: Vec<String>, env: Vec<String>, stdin: Vec<u8>, cwd: String },
    /// Seconds east of UTC that the machine's zone applies at `unix_seconds`; answers `Int`.
    LocalOffset { unix_seconds: i64 },
    /// Answers `Int(id)` of a connected stream.
    TcpConnect { host: String, port: u16 },
    /// A TCP connection with a TLS client session finished against the bundled root certificates; answers `Int(id)`.
    TlsConnect { host: String, port: u16 },
    /// A listener whose accepted streams are TLS server sessions for a PEM chain and key; answers `Int(id)`.
    TlsListen { host: String, port: u16, cert_pem: String, key_pem: String },
    /// Port `0` picks a free one; answers `Int(id)` of a listener.
    TcpListen { host: String, port: u16 },
    /// Answers `Int(id)` of the accepted stream.
    TcpAccept { id: i64 },
    /// Up to `max` bytes; empty once the peer finished sending.
    SocketRead { id: i64, max: usize },
    /// `SocketRead` that fails with `TimedOut` after `timeout_millis` without data.
    SocketReadFor { id: i64, max: usize, timeout_millis: u64 },
    SocketWrite { id: i64, bytes: Vec<u8> },
    /// The peer reads end of input; this side can still read.
    SocketShutdownWrite { id: i64 },
    SocketClose { id: i64 },
    /// Answers `Int(port)`.
    LocalPort { id: i64 },
    /// Answers `Text("host:port")` of a stream's peer.
    PeerAddr { id: i64 },
    UdpBind { host: String, port: u16 },
    /// Answers `Int(bytes sent)`.
    UdpSendTo { id: i64, host: String, port: u16, bytes: Vec<u8> },
    /// Answers `Datagram`; a longer datagram is truncated to `max`.
    UdpRecvFrom { id: i64, max: usize },
    /// Loads a shared library; answers `Int(id)`. `PermissionDenied` unless the run allows native libraries.
    LibOpen { path: String },
    /// Looks `name` up in a library and binds it to `signature` (`"fi>f"`: `i` Int, `f` Float, `b` Bool, `s` String, `p` Bytes, `v` no result); answers `Int(id)`.
    LibSymbol { lib: i64, name: String, signature: String },
    /// Calls a bound symbol; answers `Int` (a Float as its bits), `Text` or `Unit` by the signature's result.
    /// `blocking` runs it on the offload pool; otherwise it runs inline on the worker.
    LibCall { symbol: i64, args: Vec<CArg>, blocking: bool },
    /// Drops the handle; symbols already bound keep the library loaded.
    LibClose { lib: i64 },
    /// One HTTP exchange to completion; answers `Http`. Any status is an answer; a transport failure, a body past `max_body` or too many redirects is an error.
    HttpRequest { method: String, url: String, headers: Vec<(String, String)>, body: Vec<u8>, timeout_millis: u64, max_redirects: u32, max_body: u64 },
    /// Opens a SQLite database file (`:memory:` for a private one in memory); answers `Int(id)`.
    SqlOpen { path: String },
    /// Runs one statement; answers `Sql`. `tx` is `0` or the token of the open transaction.
    SqlRun { id: i64, tx: i64, sql: String, params: Vec<SqlValue>, mode: SqlMode },
    /// Starts a transaction, waiting while another is open; answers `Int(token)`, unique across databases.
    SqlBegin { id: i64 },
    /// Commits or rolls back the transaction with `tx` and lets the next caller in.
    SqlEnd { tx: i64, commit: bool },
    SqlClose { id: i64 },
}

/// What `SqlRun` does with its statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlMode {
    /// Runs it, answering the rows changed and the last inserted row id.
    Execute,
    /// Runs it, answering the result columns and rows.
    Query,
    /// Only prepares it: answers its parameter count (as `changed`) and result columns, running nothing.
    Check,
}

/// One SQLite value; a float is held as its bits so requests stay `Eq`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SqlValue {
    Null,
    Int(i64),
    Float(u64),
    Text(String),
    Blob(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// What executing a [`PlatformRequest`] answers.
pub enum PlatformResponse {
    Unit,
    Int(i64),
    /// `None` at end of input.
    Line(Option<String>),
    Text(String),
    Bytes(Vec<u8>),
    OptionString(Option<String>),
    Strings(Vec<String>),
    Pairs(Vec<(String, String)>),
    Stat(FileStat),
    Process(ProcessOutput),
    /// One UDP datagram and its sender as `host:port`.
    Datagram { bytes: Vec<u8>, from: String },
    /// A finished HTTP exchange; header names are lowercase.
    Http { status: i64, headers: Vec<(String, String)>, body: Vec<u8> },
    /// A finished statement: rows changed, the last inserted row id, and the result columns and rows.
    Sql { changed: i64, last_id: i64, columns: Vec<String>, rows: Vec<Vec<SqlValue>> },
}

/// Ordinals match `std.error.ErrorKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformErrorKind {
    NotFound = 0,
    PermissionDenied = 1,
    InvalidData = 2,
    Interrupted = 3,
    AlreadyExists = 4,
    WouldBlock = 5,
    TimedOut = 6,
    Unsupported = 7,
    Other = 8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A failed platform request: a kind and a message.
pub struct PlatformError {
    pub kind: PlatformErrorKind,
    pub message: String,
}

/// How a wait started by [`Platform::wait`] ended.
pub enum Woken {
    /// The request can now run without waiting; run it with [`Platform::execute`].
    Run(PlatformRequest),
    /// The wait was the whole request; this is its answer.
    Done(crate::PlatformResult),
}

/// Called once, from any thread, when a wait ends.
pub type Wake = Box<dyn FnOnce(Woken) + Send>;

/// The outside world: it executes requests, and may wait on the reactor.
pub trait Platform: Send + Sync {
    fn execute(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError>;

    /// Whether the runtime should park the task and run `request` off the worker.
    fn blocking(&self, request: &PlatformRequest) -> bool;

    /// Waits for time or readiness without holding a thread, then calls `wake` once; a request that
    /// does not wait is handed back and runs on the offload pool.
    fn wait(&self, request: PlatformRequest, wake: Wake) -> Result<(), Box<(PlatformRequest, Wake)>> {
        Err(Box::new((request, wake)))
    }

    /// Starts a source that pushes into `sink` until it ends or the handle is closed.
    fn open_source(&self, _request: crate::SourceRequest, _sink: crate::EventSink) -> Result<crate::SourceHandle, PlatformError> {
        Err(PlatformError { kind: PlatformErrorKind::Unsupported, message: "this platform has no event sources".to_string() })
    }
}
