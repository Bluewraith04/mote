//! The C signature a `std.dev.libtools` `bind<F>` names.

use crate::types::Type;

/// Most parameters a bound C function takes.
pub(crate) const MAX_PARAMS: usize = 6;

/// Whether `mangled` is `std.dev.libtools`'s `bind` or `bind_blocking`.
pub(crate) fn is_bind(mangled: &str) -> bool {
    mangled.ends_with("libtools_bind") || is_bind_blocking(mangled)
}

/// Whether `mangled` is `std.dev.libtools`'s `bind_blocking`: its calls run off the worker.
pub(crate) fn is_bind_blocking(mangled: &str) -> bool {
    mangled.ends_with("libtools_bind_blocking")
}

/// Marks the function in `bind`'s `Result<F, Error>` Sendable: it captures only an id and a signature.
pub(crate) fn make_sendable(result: &mut Type) {
    let Type::Enum { args, variants, .. } = result else { return };
    let payloads = variants.iter_mut().flat_map(|(_, payload)| payload.iter_mut());
    for ty in args.iter_mut().take(1).chain(payloads) {
        if let Type::Function { sendable, .. } = ty {
            *sendable = true;
        }
    }
}

/// `F` as a signature string (`"fi>f"`: `i` Int, `f` Float, `b` Bool, `s` String, `p` Bytes, `v` Null) and its parameter count.
pub(crate) fn signature_of(f: Option<&Type>) -> Result<(String, usize), String> {
    let Some(Type::Function { params, ret, .. }) = f else {
        return Err("`bind` needs a function type, as in `bind<(Float) -> Float>(lib, name)`".into());
    };
    if params.len() > MAX_PARAMS {
        return Err(format!("a C function takes at most {MAX_PARAMS} parameters here"));
    }
    let mut out = String::new();
    for p in params {
        out.push(match p {
            Type::Int => 'i',
            Type::Float => 'f',
            Type::Bool => 'b',
            Type::String => 's',
            Type::Bytes => 'p',
            other => return Err(format!("a C function takes Int, Float, Bool, String or Bytes, not `{other:?}`")),
        });
    }
    out.push('>');
    out.push(match ret.as_ref() {
        Type::Int => 'i',
        Type::Float => 'f',
        Type::Bool => 'b',
        Type::String => 's',
        Type::Null => 'v',
        other => return Err(format!("a C function returns Int, Float, Bool, String or Null, not `{other:?}`")),
    });
    Ok((out, params.len()))
}
