use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use contracts::{
    EventPayload, EventSink, FileKind, FileMode, FileStat, Platform, PlatformError, PlatformErrorKind, PlatformRequest, ProcessOutput,
    PlatformResponse,
    PushResult, SourceHandle, SourceRequest, StdStream, Whence,
};

use crate::fake_net::FakeNet;
use crate::net::is_socket_request;

/// A deterministic platform: a virtual clock that moves only when told, seeded entropy, captured output.
pub struct FakePlatform {
    state: Mutex<FakeState>,
}

struct FakeState {
    wall_millis: i64,
    mono_nanos: i64,
    entropy: u64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    env: BTreeMap<String, String>,
    args: Vec<String>,
    stdin: VecDeque<String>,
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    mtimes: BTreeMap<String, i64>,
    sources: Vec<FakeSource>,
    handles: BTreeMap<i64, FakeFile>,
    processes: BTreeMap<String, ProcessOutput>,
    process_log: Vec<PlatformRequest>,
    local_offset: i64,
    next_handle: i64,
    net: FakeNet,
}

fn not_found(path: &str) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::NotFound, message: format!("{path}: no such file") }
}

impl FakePlatform {
    pub fn new(seed: u64) -> Self {
        FakePlatform {
            state: Mutex::new(FakeState {
                wall_millis: 1_700_000_000_000,
                mono_nanos: 0,
                entropy: seed,
                stdout: Vec::new(),
                stderr: Vec::new(),
                env: BTreeMap::new(),
                args: vec!["fake".to_string()],
                stdin: VecDeque::new(),
                files: BTreeMap::new(),
                dirs: BTreeSet::new(),
                mtimes: BTreeMap::new(),
                sources: Vec::new(),
                handles: BTreeMap::new(),
                processes: BTreeMap::new(),
                process_log: Vec::new(),
                local_offset: 0,
                next_handle: 0,
                net: FakeNet::new(),
            }),
        }
    }

    pub fn with_args(self, args: Vec<String>) -> Self {
        self.state.lock().unwrap().args = args;
        self
    }

    pub fn with_env(self, name: &str, value: &str) -> Self {
        self.state.lock().unwrap().env.insert(name.to_string(), value.to_string());
        self
    }

    /// Queues one line for `ReadLine`; the queue running dry is end of input.
    pub fn with_stdin_line(self, line: &str) -> Self {
        self.state.lock().unwrap().stdin.push_back(line.to_string());
        self
    }

    pub fn with_file(self, path: &str, bytes: &[u8]) -> Self {
        self.state.lock().unwrap().files.insert(path.to_string(), bytes.to_vec());
        self
    }

    /// Scripts the output `RunProcess` answers for `program`; unscripted programs are `NotFound`.
    pub fn with_process(self, program: &str, status: i64, stdout: &[u8], stderr: &[u8]) -> Self {
        let output = ProcessOutput { status, stdout: stdout.to_vec(), stderr: stderr.to_vec() };
        self.state.lock().unwrap().processes.insert(program.to_string(), output);
        self
    }

    /// Scripts the machine's zone as a fixed offset in seconds east of UTC; the default is UTC.
    pub fn with_local_offset(self, seconds: i64) -> Self {
        self.state.lock().unwrap().local_offset = seconds;
        self
    }

    /// Every `RunProcess` request so far, in order.
    pub fn process_log(&self) -> Vec<PlatformRequest> {
        self.state.lock().unwrap().process_log.clone()
    }

    /// Moves both clocks forward.
    pub fn advance(&self, nanos: i64) {
        {
            let mut st = self.state.lock().unwrap();
            st.mono_nanos += nanos;
            st.wall_millis += nanos / 1_000_000;
        }
        self.fire_timers();
    }

    /// Queues `payload` on source `index` (opening order); `None` if there is no such source.
    pub fn emit(&self, index: usize, payload: EventPayload) -> Option<PushResult> {
        let sink = self.state.lock().unwrap().sources.get(index)?.sink.clone();
        Some(sink.try_push(payload))
    }

    /// Ends source `index` from the source side.
    pub fn end_source(&self, index: usize) {
        let sink = self.state.lock().unwrap().sources.get(index).map(|s| s.sink.clone());
        if let Some(sink) = sink {
            sink.end();
        }
    }

    /// Whether source `index` was closed by the runtime.
    pub fn source_closed(&self, index: usize) -> bool {
        self.state.lock().unwrap().sources.get(index).is_some_and(|s| s.stopped.load(Ordering::SeqCst))
    }

    pub fn sources_opened(&self) -> usize {
        self.state.lock().unwrap().sources.len()
    }

    fn fire_timers(&self) {
        let due: Vec<(usize, EventSink, u64)> = {
            let st = self.state.lock().unwrap();
            let now = st.mono_nanos;
            st.sources
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    let t = s.timer.as_ref()?;
                    let live = !s.stopped.load(Ordering::SeqCst) && now >= t.next_due;
                    live.then(|| (i, s.sink.clone(), ((now - t.next_due) / t.period + 1) as u64))
                })
                .collect()
        };
        for (i, sink, n) in due {
            let mut fired = 0u64;
            let first = self.state.lock().unwrap().sources[i].timer.as_ref().map_or(0, |t| t.ticks);
            while fired < n {
                match sink.try_push(EventPayload::Int((first + fired + 1) as i64)) {
                    PushResult::Queued | PushResult::Coalesced => fired += 1,
                    PushResult::Full | PushResult::Closed => break,
                }
            }
            let mut st = self.state.lock().unwrap();
            if let Some(t) = st.sources[i].timer.as_mut() {
                t.ticks += fired;
                t.next_due += fired as i64 * t.period;
            }
        }
    }

    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.state.lock().unwrap().stdout).into_owned()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.state.lock().unwrap().stderr).into_owned()
    }
}

impl FakePlatform {
    fn run(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        let mut st = self.state.lock().unwrap();
        Ok(match request {
            PlatformRequest::WallClockMillis => PlatformResponse::Int(st.wall_millis),
            PlatformRequest::MonotonicNanos => PlatformResponse::Int(st.mono_nanos),
            PlatformRequest::Entropy => {
                st.entropy = st.entropy.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = st.entropy;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                PlatformResponse::Int((z ^ (z >> 31)) as i64)
            }
            PlatformRequest::Write { stream, bytes } => {
                match stream {
                    StdStream::Stdout => st.stdout.extend_from_slice(&bytes),
                    StdStream::Stderr => st.stderr.extend_from_slice(&bytes),
                }
                PlatformResponse::Unit
            }
            PlatformRequest::Flush { .. } => PlatformResponse::Unit,
            PlatformRequest::EnvVar { name } => PlatformResponse::OptionString(st.env.get(&name).cloned()),
            PlatformRequest::EnvVars => PlatformResponse::Pairs(st.env.clone().into_iter().collect()),
            PlatformRequest::Args => PlatformResponse::Strings(st.args.clone()),
            PlatformRequest::ReadLine => PlatformResponse::Line(st.stdin.pop_front()),
            PlatformRequest::ReadFile { path } => {
                let bytes = st.files.get(&path).ok_or_else(|| not_found(&path))?;
                let text = String::from_utf8(bytes.clone()).map_err(|_| PlatformError {
                    kind: PlatformErrorKind::InvalidData,
                    message: "stream did not contain valid UTF-8".to_string(),
                })?;
                PlatformResponse::Text(text)
            }
            PlatformRequest::ReadFileBytes { path } => {
                PlatformResponse::Bytes(st.files.get(&path).ok_or_else(|| not_found(&path))?.clone())
            }
            PlatformRequest::WriteFile { path, bytes } => {
                let now = st.wall_millis;
                st.mtimes.insert(path.clone(), now);
                st.files.insert(path, bytes);
                PlatformResponse::Unit
            }
            PlatformRequest::Sleep { nanos } => {
                st.mono_nanos += nanos as i64;
                st.wall_millis += nanos as i64 / 1_000_000;
                PlatformResponse::Unit
            }
            PlatformRequest::LocalOffset { .. } => PlatformResponse::Int(st.local_offset),
            dir @ (PlatformRequest::ListDir { .. }
            | PlatformRequest::CreateDir { .. }
            | PlatformRequest::RemoveFile { .. }
            | PlatformRequest::RemoveDir { .. }
            | PlatformRequest::Rename { .. }
            | PlatformRequest::Stat { .. }) => return dir_request(&mut st, dir),
            run @ PlatformRequest::RunProcess { .. } => {
                st.process_log.push(run.clone());
                let PlatformRequest::RunProcess { program, .. } = run else { unreachable!() };
                PlatformResponse::Process(st.processes.get(&program).cloned().ok_or_else(|| not_found(&program))?)
            }
            PlatformRequest::LibOpen { .. }
            | PlatformRequest::LibOpenPackage { .. }
            | PlatformRequest::LibSymbol { .. }
            | PlatformRequest::LibCall { .. }
            | PlatformRequest::LibClose { .. } => {
                return Err(PlatformError { kind: PlatformErrorKind::Unsupported, message: "the fake platform has no native libraries".into() })
            }
            socket if is_socket_request(&socket) => return st.net.request(socket),
            file => return file_request(&mut st, file),
        })
    }
}

impl Platform for FakePlatform {
    fn execute(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        let slept = matches!(request, PlatformRequest::Sleep { .. });
        let result = self.run(request);
        if slept {
            self.fire_timers();
        }
        result
    }

    fn blocking(&self, _request: &PlatformRequest) -> bool {
        false
    }

    fn open_source(&self, request: SourceRequest, sink: EventSink) -> Result<SourceHandle, PlatformError> {
        let SourceRequest::Timer { period_nanos } = request else {
            return Err(PlatformError { kind: PlatformErrorKind::Other, message: "not a platform source".to_string() });
        };
        let stopped = Arc::new(AtomicBool::new(false));
        let mut st = self.state.lock().unwrap();
        let period = period_nanos.max(1) as i64;
        let next_due = st.mono_nanos + period;
        st.sources.push(FakeSource { sink, timer: Some(Timer { period, next_due, ticks: 0 }), stopped: stopped.clone() });
        Ok(SourceHandle::new(move || stopped.store(true, Ordering::SeqCst)))
    }
}

struct Timer {
    period: i64,
    next_due: i64,
    ticks: u64,
}

struct FakeSource {
    sink: EventSink,
    timer: Option<Timer>,
    stopped: Arc<AtomicBool>,
}

struct FakeFile {
    path: String,
    mode: FileMode,
    pos: usize,
}

fn other(message: &str) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::Other, message: message.to_string() }
}

fn file_request(st: &mut FakeState, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
    if let PlatformRequest::OpenFile { path, mode } = request {
        let pos = match mode {
            FileMode::Read => {
                if !st.files.contains_key(&path) {
                    return Err(not_found(&path));
                }
                0
            }
            FileMode::Write => {
                st.files.insert(path.clone(), Vec::new());
                let now = st.wall_millis;
                st.mtimes.insert(path.clone(), now);
                0
            }
            FileMode::Append => st.files.entry(path.clone()).or_default().len(),
        };
        st.next_handle += 1;
        let id = st.next_handle;
        st.handles.insert(id, FakeFile { path, mode, pos });
        return Ok(PlatformResponse::Int(id));
    }
    let id = match &request {
        PlatformRequest::FileRead { id, .. }
        | PlatformRequest::FileReadLine { id }
        | PlatformRequest::FileWrite { id, .. }
        | PlatformRequest::FileSeek { id, .. }
        | PlatformRequest::FileClose { id } => *id,
        _ => unreachable!("not a file request"),
    };
    if let PlatformRequest::FileClose { .. } = request {
        st.handles.remove(&id).ok_or_else(|| other("file is closed"))?;
        return Ok(PlatformResponse::Unit);
    }
    let file = st.handles.get_mut(&id).ok_or_else(|| other("file is closed"))?;
    let data = st.files.entry(file.path.clone()).or_default();
    Ok(match request {
        PlatformRequest::FileRead { max, .. } => {
            if file.mode != FileMode::Read {
                return Err(other("file is not open for reading"));
            }
            let start = file.pos.min(data.len());
            let end = start.saturating_add(max).min(data.len());
            file.pos = end;
            PlatformResponse::Bytes(data[start..end].to_vec())
        }
        PlatformRequest::FileReadLine { .. } => {
            if file.mode != FileMode::Read {
                return Err(other("file is not open for reading"));
            }
            let start = file.pos.min(data.len());
            if start == data.len() {
                return Ok(PlatformResponse::Line(None));
            }
            let stop = data[start..].iter().position(|b| *b == b'\n').map_or(data.len(), |i| start + i);
            file.pos = (stop + 1).min(data.len());
            let mut line = data[start..stop].to_vec();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let text = String::from_utf8(line).map_err(|_| PlatformError {
                kind: PlatformErrorKind::InvalidData,
                message: "stream did not contain valid UTF-8".to_string(),
            })?;
            PlatformResponse::Line(Some(text))
        }
        PlatformRequest::FileWrite { bytes, .. } => {
            if file.mode == FileMode::Read {
                return Err(other("file is not open for writing"));
            }
            if file.mode == FileMode::Append {
                file.pos = data.len();
            }
            if data.len() < file.pos {
                data.resize(file.pos, 0);
            }
            let overlap = bytes.len().min(data.len() - file.pos);
            data[file.pos..file.pos + overlap].copy_from_slice(&bytes[..overlap]);
            st.mtimes.insert(file.path.clone(), st.wall_millis);
            data.extend_from_slice(&bytes[overlap..]);
            file.pos += bytes.len();
            PlatformResponse::Unit
        }
        PlatformRequest::FileSeek { offset, whence, .. } => {
            let base = match whence {
                Whence::Start => 0,
                Whence::Current => file.pos as i64,
                Whence::End => data.len() as i64,
            };
            let target = base.checked_add(offset).filter(|t| *t >= 0).ok_or_else(|| PlatformError {
                kind: PlatformErrorKind::InvalidData,
                message: "seek before the start of the file".to_string(),
            })?;
            file.pos = target as usize;
            PlatformResponse::Int(target)
        }
        _ => unreachable!("not a file request"),
    })
}

fn is_root(path: &str) -> bool {
    matches!(path, "" | "." | "/")
}

fn prefix(path: &str) -> String {
    format!("{}/", path.trim_end_matches('/'))
}

fn is_dir(st: &FakeState, path: &str) -> bool {
    let p = prefix(path);
    is_root(path)
        || st.dirs.contains(path.trim_end_matches('/'))
        || st.files.keys().any(|k| k.starts_with(&p))
        || st.dirs.iter().any(|d| d.starts_with(&p))
}

fn children(st: &FakeState, path: &str) -> std::collections::BTreeSet<String> {
    let p = if is_root(path) { String::new() } else { prefix(path) };
    st.files
        .keys()
        .chain(st.dirs.iter())
        .filter_map(|k| k.strip_prefix(p.as_str()))
        .filter_map(|rest| rest.split('/').next().filter(|n| !n.is_empty()))
        .map(str::to_string)
        .collect()
}

fn parent_of(path: &str) -> &str {
    path.trim_end_matches('/').rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn dir_request(st: &mut FakeState, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
    Ok(match request {
        PlatformRequest::ListDir { path } => {
            if st.files.contains_key(&path) {
                return Err(other("not a directory"));
            }
            if !is_dir(st, &path) {
                return Err(not_found(&path));
            }
            PlatformResponse::Strings(children(st, &path).into_iter().collect())
        }
        PlatformRequest::CreateDir { path, recursive } => {
            let path = path.trim_end_matches('/').to_string();
            if st.files.contains_key(&path) || (is_dir(st, &path) && !recursive) {
                return Err(PlatformError {
                    kind: PlatformErrorKind::AlreadyExists,
                    message: format!("{path}: already exists"),
                });
            }
            if recursive {
                let mut dir = path.as_str();
                while !is_root(dir) {
                    st.dirs.insert(dir.to_string());
                    dir = parent_of(dir);
                }
            } else {
                if !is_dir(st, parent_of(&path)) {
                    return Err(not_found(&path));
                }
                st.dirs.insert(path);
            }
            PlatformResponse::Unit
        }
        PlatformRequest::RemoveFile { path } => {
            if st.files.remove(&path).is_none() {
                return Err(if is_dir(st, &path) { other("is a directory") } else { not_found(&path) });
            }
            st.mtimes.remove(&path);
            PlatformResponse::Unit
        }
        PlatformRequest::RemoveDir { path, recursive } => {
            if st.files.contains_key(&path) {
                return Err(other("not a directory"));
            }
            if is_root(&path) || !is_dir(st, &path) {
                return Err(not_found(&path));
            }
            let path = path.trim_end_matches('/').to_string();
            if !children(st, &path).is_empty() && !recursive {
                return Err(other("directory not empty"));
            }
            let p = prefix(&path);
            st.files.retain(|k, _| !k.starts_with(&p));
            st.mtimes.retain(|k, _| !k.starts_with(&p));
            st.dirs.retain(|d| d != &path && !d.starts_with(&p));
            PlatformResponse::Unit
        }
        PlatformRequest::Rename { from, to } => {
            if let Some(bytes) = st.files.remove(&from) {
                let modified = st.mtimes.remove(&from);
                st.files.insert(to.clone(), bytes);
                st.mtimes.extend(modified.map(|m| (to.clone(), m)));
                for file in st.handles.values_mut().filter(|f| f.path == from) {
                    file.path = to.clone();
                }
            } else if is_dir(st, &from) && !is_root(&from) {
                let (from, to) = (from.trim_end_matches('/').to_string(), to.trim_end_matches('/').to_string());
                let p = prefix(&from);
                let moved = |k: &str| format!("{to}/{}", &k[p.len()..]);
                let files: Vec<_> = st.files.keys().filter(|k| k.starts_with(&p)).cloned().collect();
                for k in files {
                    let bytes = st.files.remove(&k).unwrap();
                    st.files.insert(moved(&k), bytes);
                    if let Some(m) = st.mtimes.remove(&k) {
                        st.mtimes.insert(moved(&k), m);
                    }
                }
                let dirs: Vec<_> = st.dirs.iter().filter(|d| d.starts_with(&p)).cloned().collect();
                for d in dirs {
                    st.dirs.remove(&d);
                    st.dirs.insert(moved(&d));
                }
                st.dirs.remove(&from);
                st.dirs.insert(to);
            } else {
                return Err(not_found(&from));
            }
            PlatformResponse::Unit
        }
        PlatformRequest::Stat { path } => {
            if let Some(bytes) = st.files.get(&path) {
                let modified_millis = st.mtimes.get(&path).copied().unwrap_or(st.wall_millis);
                PlatformResponse::Stat(FileStat { kind: FileKind::File, size: bytes.len() as i64, modified_millis })
            } else if is_dir(st, &path) {
                PlatformResponse::Stat(FileStat { kind: FileKind::Dir, size: 0, modified_millis: st.wall_millis })
            } else {
                return Err(not_found(&path));
            }
        }
        _ => unreachable!("not a directory request"),
    })
}
