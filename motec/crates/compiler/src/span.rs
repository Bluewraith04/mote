use std::sync::atomic::{AtomicU32, Ordering};

/// A fresh id for one lexed source text; 0 is left for spans built by hand.
pub fn next_source_id() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

static SOURCES: std::sync::Mutex<Vec<(u32, String, String)>> = std::sync::Mutex::new(Vec::new());
static RELEASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Records the text of lexed source `id`, for fault locations.
pub fn register_source_text(id: u32, text: &str) {
    SOURCES.lock().unwrap().push((id, String::new(), text.to_string()));
}

/// Offsets from this up belong to code a `@derive` wrote; each generated node has a unique offset and the attribute's line and column.
pub(crate) const DERIVED_BASE: usize = 1 << 40;

/// Names source `id` (its file path); a source already registered by the lexer gains the name.
pub fn name_source(id: u32, path: &str) {
    if let Some(entry) = SOURCES.lock().unwrap().iter_mut().find(|e| e.0 == id) {
        entry.1 = path.to_string();
    }
}

/// The `(path, text)` of source `id`.
pub fn source_info(id: u32) -> Option<(String, String)> {
    SOURCES.lock().unwrap().iter().find(|e| e.0 == id).map(|e| (e.1.clone(), e.2.clone()))
}

/// A release build carries no span tables and no source text.
pub fn set_release(release: bool) {
    RELEASE.store(release, Ordering::Relaxed);
}

pub fn is_release() -> bool {
    RELEASE.load(Ordering::Relaxed)
}

/// A source location: byte offsets, line and column. `source` names the lexed text the offsets index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "SpanRepr")]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
    pub source: u32,
}

thread_local! {
    static SOURCE_REMAP: std::cell::Cell<Option<(u32, u32)>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with deserialized spans from source `from` moved to source `to`.
pub fn with_source_remap<R>(from: u32, to: u32, f: impl FnOnce() -> R) -> R {
    let old = SOURCE_REMAP.with(|r| r.replace(Some((from, to))));
    let out = f();
    SOURCE_REMAP.with(|r| r.set(old));
    out
}

#[derive(serde::Deserialize)]
struct SpanRepr {
    start: usize,
    end: usize,
    line: usize,
    col: usize,
    source: u32,
}

impl From<SpanRepr> for Span {
    fn from(r: SpanRepr) -> Self {
        let source = match SOURCE_REMAP.with(|m| m.get()) {
            Some((from, to)) if r.source == from => to,
            _ => r.source,
        };
        Span { start: r.start, end: r.end, line: r.line, col: r.col, source }
    }
}

impl Span {
    pub fn new(start: usize, end: usize, line: usize, col: usize) -> Self {
        Self { start, end, line, col, source: 0 }
    }

    pub(crate) fn with_source(mut self, source: u32) -> Self {
        self.source = source;
        self
    }

    pub fn merge(&self, other: &Span) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line.min(other.line),
            col: if self.start <= other.start { self.col } else { other.col },
            source: self.source,
        }
    }
}
