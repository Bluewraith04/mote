//! Foreign calls: shared libraries opened by path, symbols bound to a C signature, calls through libffi.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use contracts::{CArg, PlatformError, PlatformErrorKind, PlatformResponse};
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Library;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Int,
    Float,
    Bool,
    Text,
    Buffer,
    Void,
}

impl Kind {
    fn of(c: char) -> Option<Kind> {
        Some(match c {
            'i' => Kind::Int,
            'f' => Kind::Float,
            'b' => Kind::Bool,
            's' => Kind::Text,
            'p' => Kind::Buffer,
            'v' => Kind::Void,
            _ => return None,
        })
    }

    fn ffi_type(self) -> Type {
        match self {
            Kind::Int => Type::i64(),
            Kind::Float => Type::f64(),
            Kind::Bool => Type::c_int(),
            Kind::Text | Kind::Buffer => Type::pointer(),
            Kind::Void => Type::void(),
        }
    }
}

fn parse_signature(text: &str) -> Result<(Vec<Kind>, Kind), String> {
    let (params, ret) = text.split_once('>').ok_or_else(|| format!("bad signature `{text}`"))?;
    let params: Option<Vec<Kind>> = params.chars().map(Kind::of).collect();
    let mut ret_chars = ret.chars();
    let ret = ret_chars.next().and_then(Kind::of).filter(|k| *k != Kind::Buffer && ret_chars.next().is_none());
    match (params, ret) {
        (Some(p), Some(r)) if !p.contains(&Kind::Void) => Ok((p, r)),
        _ => Err(format!("bad signature `{text}`")),
    }
}

struct Symbol {
    _lib: Arc<Library>,
    address: *const c_void,
    cif: Cif,
    params: Vec<Kind>,
    ret: Kind,
}

// SAFETY: a symbol is an address into a library kept loaded by `_lib`, and its `Cif` is read-only after building.
unsafe impl Send for Symbol {}
unsafe impl Sync for Symbol {}

enum Slot {
    Int(i64),
    Float(f64),
    Bool(c_int),
    Pointer(*const c_void),
}

pub(crate) struct DynLibs {
    allowed: AtomicBool,
    packages: Mutex<Vec<(String, PathBuf)>>,
    next: AtomicI64,
    libs: Mutex<HashMap<i64, (Arc<Library>, PathBuf)>>,
    symbols: Mutex<HashMap<i64, Arc<Symbol>>>,
    bound: Mutex<HashMap<(PathBuf, String, String), i64>>,
}

fn error(kind: PlatformErrorKind, message: impl Into<String>) -> PlatformError {
    PlatformError { kind, message: message.into() }
}

impl DynLibs {
    pub(crate) fn new() -> Self {
        DynLibs {
            allowed: AtomicBool::new(false),
            packages: Mutex::new(Vec::new()),
            next: AtomicI64::new(1),
            libs: Mutex::new(HashMap::new()),
            symbols: Mutex::new(HashMap::new()),
            bound: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn set_allowed(&self, allowed: bool) {
        self.allowed.store(allowed, Ordering::SeqCst);
    }

    /// The packages granted native access, each with the directory holding its libraries.
    pub(crate) fn set_packages(&self, packages: Vec<(String, PathBuf)>) {
        *self.packages.lock().unwrap() = packages;
    }

    pub(crate) fn open(&self, path: &str) -> Result<i64, PlatformError> {
        if !self.allowed.load(Ordering::SeqCst) {
            return Err(error(PlatformErrorKind::PermissionDenied, "native libraries are off; run with --allow-native"));
        }
        self.load(Path::new(path))
    }

    /// Opens `name` from the directory of a granted package, with no `--allow-native`.
    pub(crate) fn open_package(&self, package: &str, name: &str) -> Result<i64, PlatformError> {
        let dir = self.packages.lock().unwrap().iter().find(|(p, _)| p == package).map(|(_, d)| d.clone());
        let dir = dir.ok_or_else(|| error(PlatformErrorKind::PermissionDenied, format!("{package} is not granted native access")))?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(error(PlatformErrorKind::InvalidData, format!("'{name}' is not a library name")));
        }
        if !dir.is_dir() {
            return Err(error(PlatformErrorKind::NotFound, format!("the native directory {} is missing", dir.display())));
        }
        let file = format!("{}{name}{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
        self.load(&dir.join(file))
    }

    fn load(&self, path: &Path) -> Result<i64, PlatformError> {
        // SAFETY: loading runs the library's initialisers; `--allow-native` or a package grant is the caller's consent to that.
        let lib = unsafe { Library::new(path) }.map_err(|e| error(PlatformErrorKind::NotFound, e.to_string()))?;
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.libs.lock().unwrap().insert(id, (Arc::new(lib), path.to_path_buf()));
        Ok(id)
    }

    /// One symbol per library path, name and signature, so binding the same function again adds nothing.
    pub(crate) fn symbol(&self, lib: i64, name: &str, signature: &str) -> Result<i64, PlatformError> {
        let (lib, path) = self.libs.lock().unwrap().get(&lib).cloned().ok_or_else(|| error(PlatformErrorKind::InvalidData, "the library is closed"))?;
        let key = (path, name.to_string(), signature.to_string());
        if let Some(id) = self.bound.lock().unwrap().get(&key) {
            return Ok(*id);
        }
        let (params, ret) = parse_signature(signature).map_err(|m| error(PlatformErrorKind::InvalidData, m))?;
        // SAFETY: only the address is taken here; the call site's signature is the caller's claim about it.
        let address = unsafe { lib.get::<*const c_void>(name.as_bytes()) }.map_err(|e| error(PlatformErrorKind::NotFound, e.to_string()))?;
        let address = *address;
        let cif = Cif::new(params.iter().map(|k| k.ffi_type()), ret.ffi_type());
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.symbols.lock().unwrap().insert(id, Arc::new(Symbol { _lib: lib, address, cif, params, ret }));
        self.bound.lock().unwrap().insert(key, id);
        Ok(id)
    }

    pub(crate) fn close(&self, lib: i64) -> Result<PlatformResponse, PlatformError> {
        self.libs.lock().unwrap().remove(&lib).ok_or_else(|| error(PlatformErrorKind::InvalidData, "the library is closed"))?;
        Ok(PlatformResponse::Unit)
    }

    pub(crate) fn call(&self, symbol: i64, args: Vec<CArg>) -> Result<PlatformResponse, PlatformError> {
        let sym = self.symbols.lock().unwrap().get(&symbol).cloned().ok_or_else(|| error(PlatformErrorKind::InvalidData, "no such symbol"))?;
        if args.len() != sym.params.len() {
            return Err(error(PlatformErrorKind::InvalidData, format!("expected {} argument(s), got {}", sym.params.len(), args.len())));
        }
        let mut texts: Vec<CString> = Vec::new();
        let mut slots: Vec<Slot> = Vec::new();
        for (kind, arg) in sym.params.iter().zip(&args) {
            slots.push(match (kind, arg) {
                (Kind::Int, CArg::Word(w)) => Slot::Int(*w as i64),
                (Kind::Float, CArg::Word(w)) => Slot::Float(f64::from_bits(*w)),
                (Kind::Bool, CArg::Word(w)) => Slot::Bool((*w != 0) as c_int),
                (Kind::Buffer, CArg::Word(w)) => Slot::Pointer(*w as usize as *const c_void),
                (Kind::Text, CArg::Text(s)) => {
                    let c = CString::new(s.as_str()).map_err(|_| error(PlatformErrorKind::InvalidData, "a String passed to C holds a NUL byte"))?;
                    texts.push(c);
                    Slot::Pointer(texts.last().unwrap().as_ptr() as *const c_void)
                }
                _ => return Err(error(PlatformErrorKind::InvalidData, "an argument does not match the signature")),
            });
        }
        let ffi_args: Vec<Arg> = slots
            .iter()
            .map(|s| match s {
                Slot::Int(v) => Arg::new(v),
                Slot::Float(v) => Arg::new(v),
                Slot::Bool(v) => Arg::new(v),
                Slot::Pointer(v) => Arg::new(v),
            })
            .collect();
        let code = CodePtr(sym.address as *mut c_void);
        // SAFETY: the address, argument kinds and result kind are those the program claimed at `bind`; a wrong claim is undefined behaviour, as in any C FFI.
        Ok(unsafe {
            match sym.ret {
                Kind::Int => PlatformResponse::Int(sym.cif.call::<i64>(code, &ffi_args)),
                Kind::Float => PlatformResponse::Int(sym.cif.call::<f64>(code, &ffi_args).to_bits() as i64),
                Kind::Bool => PlatformResponse::Int((sym.cif.call::<c_int>(code, &ffi_args) != 0) as i64),
                Kind::Void => {
                    sym.cif.call::<()>(code, &ffi_args);
                    PlatformResponse::Unit
                }
                Kind::Text => {
                    let p = sym.cif.call::<*const c_char>(code, &ffi_args);
                    PlatformResponse::Text(if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() })
                }
                Kind::Buffer => unreachable!("a buffer is never a result"),
            }
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn binding_the_same_function_again_reuses_its_symbol() {
        let libs = DynLibs::new();
        libs.set_allowed(true);
        let first = libs.open("libm.so.6").unwrap();
        let second = libs.open("libm.so.6").unwrap();
        let a = libs.symbol(first, "sin", "f>f").unwrap();
        assert_eq!(libs.symbol(second, "sin", "f>f").unwrap(), a);
        assert_ne!(libs.symbol(second, "cos", "f>f").unwrap(), a);
        assert_eq!(libs.symbols.lock().unwrap().len(), 2);
    }
}
