//! The native functions the runtime installs by default; a `CALLNATIVE` operand indexes [`registry`].

use std::ptr::NonNull;
use std::sync::Arc;

use isa::value::{
    ObjectHeader, TypeDescriptor, Value, BACKING_CAP_SLOT, BACKING_DATA_BASE,
    BYTES_TYPE_ID, CHANNEL_TYPE_ID, HEADER_BACKING_SLOT, HEADER_LEN_SLOT, LIST_ITER_TYPE_ID,
    LIST_TYPE_ID, MAP_ENTRY_STRIDE, MAP_TYPE_ID, SET_TYPE_ID, TABLE_HEADER_SLOTS, TABLE_INDEX_SLOT, TABLE_USED_SLOT,
};
use contracts::{
    EventPayload, FileMode, NativeError, Overflow, PlatformRequest, PlatformResponse, SourceRequest, StdStream, Whence,
};
use runtime::{NativeCtx, NativeDispatchHook, PlatformResult, Runtime};

use crate::native_call::{ContinuationFn, NativeCallContext, NativeFunctionRegistry, SourceSpec};

mod archives;
mod collections;
mod crypto;
mod formats;
mod http_wire;
mod libtools;
mod sql;
mod system;
mod text_math;
use self::archives::*;
use self::collections::*;
use self::crypto::*;
use self::formats::*;
use self::http_wire::*;
use self::libtools::*;
use self::sql::*;
use self::system::*;
use self::text_math::*;

/// The registry of default built-ins, in `CALLNATIVE`-index order.
pub fn registry() -> NativeFunctionRegistry {
    let r = NativeFunctionRegistry::new();

    r.register(
        "print",
        |ctx| {
            if let Some(v) = ctx.arg(0) {
                let bytes = render(&v).into_bytes();
                ask(ctx, PlatformRequest::Write { stream: StdStream::Stdout, bytes })?;
            }
            Ok(Value::null())
        },
    );
    r.register(
        "println",
        |ctx| {
            let mut bytes = ctx.arg(0).map(|v| render(&v)).unwrap_or_default().into_bytes();
            bytes.push(b'\n');
            ask(ctx, PlatformRequest::Write { stream: StdStream::Stdout, bytes })?;
            Ok(Value::null())
        },
    );
    r.register(
        "assert",
        |ctx| match ctx.arg(0).map(|v| v.is_truthy()) {
            Some(true) => Ok(Value::null()),
            _ => Err("assertion failed".to_string()),
        },
    );
    r.register("int_abs", |ctx| Ok(Value::int(arg_int(ctx, 0).abs())));
    r.register(
        "int_min",
        |ctx| Ok(Value::int(arg_int(ctx, 0).min(arg_int(ctx, 1)))),
    );
    r.register(
        "int_max",
        |ctx| Ok(Value::int(arg_int(ctx, 0).max(arg_int(ctx, 1)))),
    );
    r.register(
        "time_millis",
        |ctx| ask_int(ctx, PlatformRequest::WallClockMillis),
    );
    r.register(
        "panic",
        |ctx| {
            Err(ctx
                .arg(0)
                .map(|v| render(&v))
                .unwrap_or_else(|| "panic".to_string()))
        },
    );
    r.register("len", native_len);
    r.register("list_new", list_new);
    r.register("coll_get", coll_get);
    r.register("coll_set", coll_set);
    r.register("coll_push", coll_push);
    r.register("list_pop", list_pop);
    r.register("coll_clear", coll_clear);
    r.register("list_iter", list_iter);
    r.register("map_new", map_new);
    r.register("set_new", set_new);
    r.register("coll_get_or", coll_get_or);
    r.register("coll_remove", coll_remove);
    r.register("coll_contains", coll_contains);
    r.register("set_add", set_add);
    r.register("map_keys", map_keys);
    r.register("map_values", map_values);
    r.register("map_entries", map_entries);
    r.register("set_items", set_items);
    r.register("bytes_new", bytes_new);
    r.register("bytes_extend", bytes_extend);
    r.register("coll_slice", coll_slice);
    r.register("bytes_is_utf8", bytes_is_utf8);
    r.register("bytes_decode", bytes_decode);
    r.register("str_find", str_find);
    r.register("str_replace", str_replace);
    r.register("str_to_upper", str_to_upper);
    r.register("str_to_lower", str_to_lower);
    r.register("str_trim", str_trim);
    r.register("str_split", str_split);
    r.register("str_bytes", str_bytes);
    r.register("str_repeat", str_repeat);
    r.register("str_starts_with", str_starts_with);
    r.register("str_ends_with", str_ends_with);
    r.register("str_from", str_from);
    r.register("math_f1", math_f1);
    r.register("math_f2", math_f2);
    r.register("math_pred", math_pred);
    r.register("math_iwrap", math_iwrap);
    r.register("num_to_float", num_to_float);
    r.register("num_to_int", num_to_int);
    r.register("clone", clone);
    r.register("__operand_check", operand_check);
    r.register("__unary_check", unary_check);
    r.register("__field_get", field_get);
    r.register("__field_set", field_set);
    r.register(
        "typeof",
        |ctx| {
            let v = ctx.arg(0).unwrap_or(Value::null());
            let name = describe_type_name(&v);
            ctx.heap.alloc_string(name.as_bytes())
        },
    );
    r.register_fallible("libtools_open", libtools_open);
    r.register_fallible("libtools_symbol", libtools_symbol);
    r.register("libtools_close", libtools_close);
    r.register_fn("libtools_call", None, Arc::new(LibtoolsCall));
    r.register("bytes_zeros", bytes_zeros);
    r.register("env_args", env_args);
    r.register("env_var_raw", env_var_raw);
    r.register("env_vars", env_vars);
    r.register_fallible("str_parse_int", str_parse_int);
    r.register_fallible("str_parse_float", str_parse_float);
    r.register("io_write", io_write);
    r.register("io_flush", io_flush);
    r.register_platform("io_read_line", io_read_line);
    r.register_platform("io_read_file", io_read_file);
    r.register_platform("io_write_file", io_write_file);
    r.register_platform("io_read_file_bytes", io_read_file_bytes);
    r.register_platform("io_write_file_bytes", io_write_file_bytes);
    r.register_platform("fs_open", fs_open);
    r.register_platform("fs_read", fs_read);
    r.register_platform("fs_read_line", fs_read_line);
    r.register_platform("fs_write", fs_write);
    r.register_platform("fs_write_text", fs_write_text);
    r.register_platform("fs_seek", fs_seek);
    r.register_platform("fs_close", fs_close);
    r.register_platform("fs_list_dir", fs_list_dir);
    r.register_platform("fs_create_dir", fs_create_dir);
    r.register_platform("fs_remove_file", fs_remove_file);
    r.register_platform("fs_remove_dir", fs_remove_dir);
    r.register_platform("fs_rename", fs_rename);
    r.register_platform("fs_stat", fs_stat);
    r.register_platform("process_run", process_run);
    r.register_platform("net_tcp_connect", net_tcp_connect);
    r.register_platform("net_tcp_listen", net_tcp_listen);
    r.register_platform("net_tcp_accept", net_tcp_accept);
    r.register_platform("net_read", net_read);
    r.register_platform("net_write", net_write);
    r.register_platform("net_write_text", net_write_text);
    r.register_platform("net_shutdown_write", net_shutdown_write);
    r.register_platform("net_close", net_close);
    r.register_platform("net_local_port", net_local_port);
    r.register_platform("net_peer_addr", net_peer_addr);
    r.register_platform("net_udp_bind", net_udp_bind);
    r.register_platform("net_udp_send_to", net_udp_send_to);
    r.register_platform("net_udp_recv_from", net_udp_recv_from);
    r.register("time_now_nanos", time_now_nanos);
    r.register_platform("time_sleep_nanos", time_sleep);
    r.register_platform("time_local_offset", time_local_offset);
    r.register_source("time_ticker", time_ticker);
    r.register("time_unix_millis", time_unix_millis);
    r.register("random_entropy", random_entropy);
    r.register("random_advance", |ctx| Ok(Value::int(splitmix_advance(arg_int(ctx, 0)))));
    r.register("random_bits", |ctx| Ok(Value::int(splitmix_bits(arg_int(ctx, 0)))));
    r.register(
        "random_unit",
        |ctx| Ok(Value::float((splitmix_bits(arg_int(ctx, 0)) >> 10) as f64 / (1u64 << 53) as f64)),
    );
    r.register_exit("process_exit", |ctx| {
        ask(ctx, PlatformRequest::Flush { stream: StdStream::Stdout })?;
        ask(ctx, PlatformRequest::Flush { stream: StdStream::Stderr })?;
        Ok((arg_int(ctx, 0) & 0xFF) as i32)
    });
    r.register("debug_raw", debug_raw);
    r.register("str_rfind", str_rfind);
    r.register("int_from", int_from);
    r.register("release_on_collect", |ctx| {
        let handle = ctx.arg(0).ok_or("release_on_collect: missing handle")?;
        ctx.heap.release_on_collect(handle, arg_int(ctx, 1) as u8)?;
        Ok(Value::null())
    });
    r.register_fallible("toml_parse", toml_parse);
    r.register_fallible("toml_render", toml_render);
    r.register("base64_encode", base64_encode);
    r.register_fallible("base64_decode", base64_decode);
    r.register("uuid_v4", uuid_v4);
    r.register("uuid_v7", uuid_v7);
    r.register_fallible("uuid_parse", uuid_parse);
    r.register_fallible("uuid_from_bytes", uuid_from_bytes);
    r.register("uuid_bytes", uuid_bytes);
    r.register("uuid_version", uuid_version);
    r.register_fallible("yaml_parse", yaml_parse);
    r.register_fallible("yaml_render", yaml_render);
    r.register("compress", compress);
    r.register_fallible("decompress", decompress);
    r.register_fallible("tar_pack", tar_pack);
    r.register_fallible("tar_unpack", tar_unpack);
    r.register_fallible("zip_pack", zip_pack);
    r.register_fallible("zip_unpack", zip_unpack);
    r.register_platform("http_request", http_request);
    r.register("form_encode", form_encode);
    r.register_fallible("http_parse_request", http_parse_request);
    r.register_fallible("http_decode_chunked", http_decode_chunked);
    r.register("http_reason", http_reason);
    r.register("http_date", http_date);
    r.register("form_decode", form_decode);
    r.register("percent_decode", percent_decode);
    r.register_platform("net_read_for", net_read_for);
    r.register("crypto_hash", crypto_hash);
    r.register("crypto_hmac", crypto_hmac);
    r.register("crypto_equal", crypto_equal);
    r.register_fallible("crypto_random", crypto_random);
    r.register("hex_encode", hex_encode);
    r.register_fallible("hex_decode", hex_decode);
    r.register_fallible("password_hash", password_hash);
    r.register_fallible("password_verify", password_verify);
    r.register_platform("net_tcp_connect_tls", net_tcp_connect_tls);
    r.register_platform("net_tcp_listen_tls", net_tcp_listen_tls);
    r.register_fallible("tls_self_signed", tls_self_signed);
    r.register_fallible("tls_check_identity", tls_check_identity);
    r.register_platform("sql_open", sql_open);
    r.register_platform("sql_run", sql_run);
    r.register_platform("sql_begin", sql_begin);
    r.register_platform("sql_end", sql_end);
    r.register_platform("sql_close", sql_close);

    r
}

static SYSTEM_PLATFORM: std::sync::LazyLock<Arc<platform::SystemPlatform>> =
    std::sync::LazyLock::new(|| Arc::new(platform::SystemPlatform::new(Vec::new())));

/// Lets `std.dev.libtools` load native libraries (`--allow-native`); off by default.
pub fn set_allow_native(allowed: bool) {
    SYSTEM_PLATFORM.set_allow_native(allowed);
}

/// Sets the answer of `std.sys.env.args()`; call once, before running a program.
pub fn set_script_args(args: Vec<String>) {
    SYSTEM_PLATFORM.set_args(args);
}

fn ask(ctx: &NativeCallContext<'_>, request: PlatformRequest) -> Result<PlatformResponse, String> {
    ctx.heap.platform()?.execute(request).map_err(|e| e.message)
}

fn ask_int(ctx: &NativeCallContext<'_>, request: PlatformRequest) -> Result<Value, String> {
    match ask(ctx, request)? {
        PlatformResponse::Int(n) => Ok(Value::int(n)),
        other => Err(format!("platform returned {other:?}, expected an integer")),
    }
}

fn stream_of(fd: i64) -> StdStream {
    if fd == 2 { StdStream::Stderr } else { StdStream::Stdout }
}

static BUILTINS: std::sync::LazyLock<NativeFunctionRegistry> = std::sync::LazyLock::new(registry);

/// A [`NativeDispatchHook`] over the builtins, ready for `Runtime::set_native_dispatcher`.
pub fn dispatch_hook() -> NativeDispatchHook {
    let registry = BUILTINS.clone();
    Arc::new(move |idx, heap: &mut dyn NativeCtx, args| registry.call(idx, args, heap))
}

/// Installs the system platform, the leaf dispatcher and the name resolver on `rt`.
pub fn install(rt: &mut Runtime) {
    rt.set_platform(SYSTEM_PLATFORM.clone());
    rt.set_native_dispatcher(dispatch_hook());
    let registry = BUILTINS.clone();
    rt.set_native_resolver(Arc::new(move |name| registry.get_by_name(name)));
}

fn arg_int(ctx: &NativeCallContext<'_>, idx: usize) -> i64 {
    ctx.arg(idx).and_then(|v| v.as_int()).unwrap_or(0)
}

fn type_id_of(v: Value) -> Option<u64> {
    v.as_object_ptr()
        // SAFETY: a boxed pointer points at a live object with a live descriptor.
        .map(|p| unsafe { p.as_ref().type_ptr.as_ref().id })
}

fn is_list(v: Value) -> bool {
    type_id_of(v) == Some(LIST_TYPE_ID)
}

fn is_map(v: Value) -> bool {
    type_id_of(v) == Some(MAP_TYPE_ID)
}

fn is_set(v: Value) -> bool {
    type_id_of(v) == Some(SET_TYPE_ID)
}

fn is_bytes(v: Value) -> bool {
    type_id_of(v) == Some(BYTES_TYPE_ID)
}

fn alloc_list(ctx: &mut NativeCallContext<'_>, elems: &[Value]) -> Result<Value, String> {
    let list = ctx.heap.alloc_header(LIST_TYPE_ID, 2)?;
    let backing = ctx.heap.alloc_backing(elems.len())?;
    for (i, &v) in elems.iter().enumerate() {
        ctx.heap.set_slot(backing, BACKING_DATA_BASE + i, v)?;
    }
    ctx.heap.set_slot(list, HEADER_LEN_SLOT, Value::uint(elems.len() as u64))?;
    ctx.heap.set_slot(list, HEADER_BACKING_SLOT, backing)?;
    Ok(list)
}

fn clone(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let v = ctx.arg(0).unwrap_or(Value::null());
    ctx.heap.clone_value(v)
}

/// Human-readable rendering of a value for `print` / `println`: primitives, strings and collections directly, and a `TypeName { field: value }` / `TypeName(value)` / `TypeName` form for heap objects.
/// A user `to_string` override is applied at compile time, not here.
pub fn render(v: &Value) -> String {
    render_depth(v, 0)
}

/// A short name for `v`'s runtime type, used in the `Any`-boundary mismatch message; a generic value names its recorded instantiation (`List<Int>`).
pub(crate) fn describe_type_name(v: &Value) -> String {
    if v.as_any_string().is_some() {
        return "String".to_string();
    }
    if v.as_int().is_some() {
        return "Int".to_string();
    }
    if v.as_float().is_some() {
        return "Float".to_string();
    }
    if v.as_bool().is_some() {
        return "Bool".to_string();
    }
    if v.as_char().is_some() {
        return "Char".to_string();
    }
    if v.is_null() {
        return "Null".to_string();
    }
    if let Some(ptr) = v.as_object_ptr() {
        // SAFETY: as `render_depth` — a live `ObjectPtr`'s `type_ptr` outlives it.
        let td = unsafe { ptr.as_ref().type_ptr.as_ref() };
        if let Some(instance) = &td.instance {
            return instance.to_string();
        }
        if let Some(name) = isa::value::TypeDescriptor::intrinsic_name(td.id) {
            return name.to_string();
        }
        return match td.id {
            _ if td.name.is_none() => "Tuple".to_string(),
            _ => td.name.clone().unwrap_or_else(|| "a heap value".to_string()),
        };
    }
    "a value".to_string()
}

type FieldLocation = (NonNull<ObjectHeader>, usize, Option<NonNull<TypeDescriptor>>);

fn named_field(obj: Value, name: &str, module: &str, writing: bool) -> Result<FieldLocation, String> {
    let no_field = || format!("type mismatch: `{}` has no field '{name}'", describe_type_name(&obj));
    let ptr = obj.as_object_ptr().ok_or_else(no_field)?;
    // SAFETY: a live object's `type_ptr` outlives it, and descriptors are never freed.
    let td = unsafe { ptr.as_ref().type_ptr.as_ref() };
    let field = td.fields.iter().find(|f| f.name.as_deref() == Some(name)).ok_or_else(no_field)?;
    if td.module.as_deref().is_some_and(|m| m != module) {
        if writing {
            return Err(format!("field '{name}' can only be written in its module"));
        }
        if !field.is_pub {
            return Err(format!("field '{name}' is private to its module"));
        }
    }
    Ok((ptr, field.slot as usize, field.inline.as_deref().map(NonNull::from)))
}

fn field_get(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let name = ctx.arg(1).and_then(|v| v.as_any_string()).unwrap_or_default();
    let module = ctx.arg(2).and_then(|v| v.as_any_string()).unwrap_or_default();
    let obj = ctx.arg(0).unwrap_or(Value::null());
    let (ptr, slot, embedded) = named_field(obj, &name, &module, false)?;
    let Some(nested) = embedded else {
        // SAFETY: `slot` indexes one of the descriptor's fields, which the object was allocated with.
        return Ok(unsafe { ptr.as_ref().get_field(slot) });
    };
    // SAFETY: descriptors are never freed, and the embedded struct's slots lie inside the object.
    let nested = unsafe { nested.as_ref() };
    let copy = ctx.heap.alloc_struct(nested)?;
    for k in 0..nested.slots as usize {
        let v = ctx.heap.get_slot(obj, slot + k)?;
        ctx.heap.set_slot(copy, k, v)?;
    }
    Ok(copy)
}

fn field_set(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let obj = ctx.arg(0).unwrap_or(Value::null());
    let name = ctx.arg(1).and_then(|v| v.as_any_string()).unwrap_or_default();
    let val = ctx.arg(2).unwrap_or(Value::null());
    let module = ctx.arg(3).and_then(|v| v.as_any_string()).unwrap_or_default();
    let (mut ptr, slot, embedded) = named_field(obj, &name, &module, true)?;
    if isa::seal::is_sealed(ptr) {
        return Err("cannot write a field: this object is shared and read-only".to_string());
    }
    let Some(nested) = embedded else {
        // SAFETY: `ptr` is a live object header.
        isa::value::check_store(unsafe { ptr.as_ref() }, val)?;
        // SAFETY: as `field_get`.
        unsafe { ptr.as_mut().set_field(slot, val) };
        return Ok(Value::null());
    };
    // SAFETY: descriptors are never freed.
    let size = unsafe { nested.as_ref() }.slots as usize;
    for k in 0..size {
        let v = ctx.heap.get_slot(val, k)?;
        // SAFETY: `ptr` is a live object header.
        isa::value::check_store(unsafe { ptr.as_ref() }, v)?;
        // SAFETY: the embedded struct's slots lie inside the object.
        unsafe { ptr.as_mut().set_field(slot + k, v) };
    }
    Ok(Value::null())
}

fn render_float(f: f64) -> String {
    let s = f.to_string();
    if f.is_finite() && !s.contains('.') { s + ".0" } else { s }
}

fn render_depth(v: &Value, depth: usize) -> String {
    if let Some(s) = v.as_any_string() {
        return if depth == 0 { s } else { format!("{s:?}") };
    }
    if depth > 0
        && let Some(c) = v.as_char() {
            return format!("{c:?}");
        }
    if let Some(i) = v.as_int() {
        return i.to_string();
    }
    if let Some(f) = v.as_float() {
        return render_float(f);
    }
    if let Some(b) = v.as_bool() {
        return b.to_string();
    }
    if let Some(c) = v.as_char() {
        return c.to_string();
    }
    if v.is_null() {
        return "null".to_string();
    }
    if let Some(ptr) = v.as_object_ptr() {
        if depth >= 8 {
            return "…".to_string();
        }
        // SAFETY: a live `ObjectPtr` points at an `ObjectHeader` whose `type_ptr`
        // outlives it (descriptors live for the whole run). Everything the block
        // returns is owned, so no borrow escapes.
        return unsafe {
            let header = ptr.as_ref();
            let td = header.type_ptr.as_ref();
            if td.is_function() {
                return "<fn>".to_string();
            }
            if td.id == LIST_TYPE_ID {
                return render_list(ptr, depth);
            }
            if td.id == MAP_TYPE_ID {
                return render_map(ptr, depth);
            }
            if td.id == SET_TYPE_ID {
                return render_set(ptr, depth);
            }
            if td.id == BYTES_TYPE_ID {
                return render_bytes(ptr);
            }
            if td.id == LIST_ITER_TYPE_ID {
                return "<list iterator>".to_string();
            }
            render_struct(td, header, 0, depth)
        };
    }
    format!("{v:?}")
}

/// # Safety
/// `header` is a live object whose slots match `td`, with the struct starting at slot `base`.
unsafe fn render_struct(td: &TypeDescriptor, header: &ObjectHeader, base: usize, depth: usize) -> String {
    let name = td.name.clone().unwrap_or_default();
    if td.fields.is_empty() {
        return if name.is_empty() { "()".to_string() } else { name };
    }
    let rendered: Vec<String> = td
        .fields
        .iter()
        .map(|f| match &f.inline {
            // SAFETY: a nested struct's slots lie inside the parent's, as `td` describes.
            Some(nested) => unsafe { render_struct(nested, header, base + f.slot as usize, depth + 1) },
            // SAFETY: `f.slot` is within the object's slots.
            None => render_depth(&unsafe { header.get_field(base + f.slot as usize) }, depth + 1),
        })
        .collect();
    if td.fields.iter().all(|f| f.name.is_some()) {
        let body = td
            .fields
            .iter()
            .zip(&rendered)
            .map(|(f, r)| format!("{}: {r}", f.name.as_deref().unwrap_or("?")))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{name} {{ {body} }}");
    }
    format!("{name}({})", rendered.join(", "))
}

/// # Safety
/// `ptr` is a live `List` header with a live backing.
unsafe fn render_list(ptr: NonNull<ObjectHeader>, depth: usize) -> String {
    unsafe {
        let header = ptr.as_ref();
        let len = header.get_field(HEADER_LEN_SLOT).as_uint().unwrap_or(0) as usize;
        let Some(backing) = header.get_field(HEADER_BACKING_SLOT).as_object_ptr() else {
            return "[]".to_string();
        };
        let backing = backing.as_ref();
        let items: Vec<String> = (0..len)
            .map(|i| render_depth(&backing.get_field(BACKING_DATA_BASE + i), depth + 1))
            .collect();
        format!("[{}]", items.join(", "))
    }
}

/// # Safety
/// `ptr` is a live `Map` or `Set` header with live entries, and `stride` is its entry width.
unsafe fn table_entries(ptr: NonNull<ObjectHeader>, stride: usize) -> Vec<(Value, Value)> {
    unsafe {
        let header = ptr.as_ref();
        let Some(entries) = header.get_field(HEADER_BACKING_SLOT).as_object_ptr() else {
            return Vec::new();
        };
        let used = header.get_field(TABLE_USED_SLOT).as_uint().unwrap_or(0) as usize;
        (0..used)
            .map(|i| (entries.as_ref().get_field(1 + stride * i), if stride > 1 { entries.as_ref().get_field(2 + stride * i) } else { Value::null() }))
            .filter(|(k, _)| !k.is_null())
            .collect()
    }
}

/// # Safety
/// `ptr` is a live `Map` header.
unsafe fn render_map(ptr: NonNull<ObjectHeader>, depth: usize) -> String {
    // SAFETY: the caller passes a live `Map` header.
    let items: Vec<String> = unsafe { table_entries(ptr, MAP_ENTRY_STRIDE) }
        .into_iter()
        .map(|(k, v)| format!("{}: {}", render_depth(&k, depth + 1), render_depth(&v, depth + 1)))
        .collect();
    format!("{{{}}}", items.join(", "))
}

/// # Safety
/// `ptr` is a live `Set` header.
unsafe fn render_set(ptr: NonNull<ObjectHeader>, depth: usize) -> String {
    // SAFETY: the caller passes a live `Set` header.
    let items: Vec<String> = unsafe { table_entries(ptr, 1) }.into_iter().map(|(k, _)| render_depth(&k, depth + 1)).collect();
    format!("{{{}}}", items.join(", "))
}

/// # Safety
/// `ptr` is a live `Bytes` header with a live backing.
unsafe fn render_bytes(ptr: NonNull<ObjectHeader>) -> String {
    unsafe {
        let header = ptr.as_ref();
        let len = header.get_field(HEADER_LEN_SLOT).as_uint().unwrap_or(0) as usize;
        let Some(backing) = header.get_field(HEADER_BACKING_SLOT).as_object_ptr() else {
            return "Bytes[]".to_string();
        };
        let base = backing.as_ref().field_ptr(1) as *const u8;
        let bytes = std::slice::from_raw_parts(base, len);
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        format!("Bytes[{}]", hex.join(" "))
    }
}
