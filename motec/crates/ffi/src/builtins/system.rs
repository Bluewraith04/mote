//! Environment, io, file, process, network and time natives.

use super::*;

pub(super) fn env_args(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let PlatformResponse::Strings(strs) = ask(ctx, PlatformRequest::Args)? else {
        return Err("platform returned a non-list for Args".to_string());
    };
    let mut elems = Vec::with_capacity(strs.len());
    for s in &strs {
        elems.push(ctx.heap.alloc_string(s.as_bytes())?);
    }
    alloc_list(ctx, &elems)
}

pub(super) fn env_var_raw(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let name = ctx
        .arg(0)
        .and_then(|v| v.as_heap_string())
        .ok_or_else(|| "env.var(name): name must be a String".to_string())?;
    match ask(ctx, PlatformRequest::EnvVar { name })? {
        PlatformResponse::OptionString(Some(v)) => ctx.heap.alloc_string(v.as_bytes()),
        _ => Ok(Value::null()),
    }
}

pub(super) fn env_vars(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let PlatformResponse::Pairs(vars) = ask(ctx, PlatformRequest::EnvVars)? else {
        return Err("platform returned a non-map for EnvVars".to_string());
    };
    let map = new_map(ctx, vars.len())?;
    for (k, v) in vars {
        let key = ctx.heap.alloc_string(k.as_bytes())?;
        let val = ctx.heap.alloc_string(v.as_bytes())?;
        map_insert(ctx, map, key, val)?;
    }
    Ok(map)
}

pub(super) fn answering(
    message: String,
    build: impl FnOnce(&mut NativeCallContext<'_>, PlatformResponse) -> Result<Value, String> + Send + 'static,
) -> ContinuationFn {
    Box::new(move |ctx, result| match result {
        Ok(response) => build(ctx, response).map_err(NativeError::from),
        Err(e) => Err(NativeError::failure(e.kind as i32, message)),
    })
}

pub(super) fn unit(message: String) -> ContinuationFn {
    answering(message, |_, _| Ok(Value::null()))
}

pub(super) fn answering_int(message: String, who: &'static str) -> ContinuationFn {
    answering(message, move |_, r| match r {
        PlatformResponse::Int(n) => Ok(Value::int(n)),
        other => Err(format!("{who}: platform returned {other:?}")),
    })
}

pub(super) fn answering_bytes(message: String, who: &'static str) -> ContinuationFn {
    answering(message, move |c, r| match r {
        PlatformResponse::Bytes(d) => alloc_bytes(c, &d),
        other => Err(format!("{who}: platform returned {other:?}")),
    })
}

pub(super) fn io_write(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let fd = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(1);
    let bytes = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default().into_bytes();
    ask(ctx, PlatformRequest::Write { stream: stream_of(fd), bytes })?;
    Ok(Value::null())
}

pub(super) fn io_flush(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let fd = ctx.arg(0).and_then(|v| v.as_int()).unwrap_or(1);
    ask(ctx, PlatformRequest::Flush { stream: stream_of(fd) })?;
    Ok(Value::null())
}

pub(super) fn alloc_bytes(ctx: &mut NativeCallContext<'_>, data: &[u8]) -> Result<Value, String> {
    let b = ctx.heap.alloc_header(BYTES_TYPE_ID, 2)?;
    let backing = ctx.heap.alloc_bytes_backing(data.len().max(1))?;
    if !data.is_empty() {
        ctx.heap.write_bytes(backing, 0, data)?;
    }
    ctx.heap.set_slot(b, HEADER_LEN_SLOT, Value::uint(data.len() as u64))?;
    ctx.heap.set_slot(b, HEADER_BACKING_SLOT, backing)?;
    Ok(b)
}

pub(super) fn arg_path(ctx: &NativeCallContext<'_>, who: &str) -> Result<String, String> {
    ctx.arg(0)
        .and_then(|v| v.as_heap_string())
        .ok_or_else(|| format!("{who}: path must be a String"))
}

pub(super) fn line_answer(message: &'static str) -> ContinuationFn {
    answering(message.to_string(), |ctx, r| match r {
        PlatformResponse::Line(None) => Ok(Value::null()),
        PlatformResponse::Line(Some(line)) => ctx.heap.alloc_string(line.as_bytes()),
        other => Err(format!("read_line: platform returned {other:?}")),
    })
}

pub(super) fn io_read_line(_ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::ReadLine, line_answer("read_line failed")))
}

pub(super) fn io_read_file(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = arg_path(ctx, "read_file(path)")?;
    let done = answering(format!("read_file: {path}"), |c, r| match r {
        PlatformResponse::Text(s) => c.heap.alloc_string(s.as_bytes()),
        other => Err(format!("read_file: platform returned {other:?}")),
    });
    Ok((PlatformRequest::ReadFile { path }, done))
}

pub(super) fn io_write_file(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = arg_path(ctx, "write_file(path, contents)")?;
    let bytes = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default().into_bytes();
    let done = unit(format!("write_file: {path}"));
    Ok((PlatformRequest::WriteFile { path, bytes }, done))
}

pub(super) fn io_read_file_bytes(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = arg_path(ctx, "read_file_bytes(path)")?;
    let done = answering_bytes(format!("read_file_bytes: {path}"), "read_file_bytes");
    Ok((PlatformRequest::ReadFileBytes { path }, done))
}

pub(super) fn io_write_file_bytes(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = arg_path(ctx, "write_file_bytes(path, b)")?;
    let b = ctx.arg(1).unwrap_or(Value::null());
    if !is_bytes(b) {
        return Err("write_file_bytes(path, b): b must be Bytes".to_string());
    }
    let (backing, len) = bytes_state(ctx, b)?;
    let bytes = ctx.heap.read_bytes(backing, 0, len)?;
    let done = unit(format!("write_file_bytes: {path}"));
    Ok((PlatformRequest::WriteFile { path, bytes }, done))
}

pub(super) type PlatformNative = Result<(PlatformRequest, ContinuationFn), String>;

pub(super) fn fs_open(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = arg_path(ctx, "fs open(path, mode)")?;
    let mode = match arg_int(ctx, 1) {
        0 => FileMode::Read,
        1 => FileMode::Write,
        _ => FileMode::Append,
    };
    let done = answering_int(format!("open: {path}"), "fs open");
    Ok((PlatformRequest::OpenFile { path, mode }, done))
}

pub(super) fn fs_read(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let id = arg_int(ctx, 0);
    let max = arg_int(ctx, 1).max(0) as usize;
    Ok((PlatformRequest::FileRead { id, max }, answering_bytes("fs read".to_string(), "fs read")))
}

pub(super) fn fs_read_line(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::FileReadLine { id: arg_int(ctx, 0) }, line_answer("fs read_line")))
}

pub(super) fn fs_write(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let id = arg_int(ctx, 0);
    let b = ctx.arg(1).unwrap_or(Value::null());
    if !is_bytes(b) {
        return Err("fs write(id, b): b must be Bytes".to_string());
    }
    let (backing, len) = bytes_state(ctx, b)?;
    let bytes = ctx.heap.read_bytes(backing, 0, len)?;
    Ok((PlatformRequest::FileWrite { id, bytes }, unit("fs write".to_string())))
}

pub(super) fn fs_write_text(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let id = arg_int(ctx, 0);
    let bytes = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default().into_bytes();
    Ok((PlatformRequest::FileWrite { id, bytes }, unit("fs write_text".to_string())))
}

pub(super) fn fs_seek(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let (whence, message) = match arg_int(ctx, 2) {
        0 => (Whence::Start, "fs seek"),
        1 => (Whence::Current, "fs seek_by"),
        _ => (Whence::End, "fs seek_end"),
    };
    let request = PlatformRequest::FileSeek { id: arg_int(ctx, 0), offset: arg_int(ctx, 1), whence };
    Ok((request, answering_int(message.to_string(), "fs seek")))
}

pub(super) fn fs_close(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::FileClose { id: arg_int(ctx, 0) }, unit("fs close".to_string())))
}

pub(super) fn path_arg(ctx: &NativeCallContext<'_>, idx: usize, who: &str) -> Result<String, String> {
    ctx.arg(idx).and_then(|v| v.as_heap_string()).ok_or_else(|| format!("{who}: path must be a String"))
}

pub(super) fn fs_list_dir(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = path_arg(ctx, 0, "list_dir(path)")?;
    let done = answering(format!("list_dir: {path}"), |c, r| match r {
        PlatformResponse::Strings(names) => {
            let mut elems = Vec::with_capacity(names.len());
            for name in &names {
                elems.push(c.heap.alloc_string(name.as_bytes())?);
            }
            alloc_list(c, &elems)
        }
        other => Err(format!("list_dir: platform returned {other:?}")),
    });
    Ok((PlatformRequest::ListDir { path }, done))
}

pub(super) fn fs_create_dir(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = path_arg(ctx, 0, "create_dir(path)")?;
    let recursive = arg_int(ctx, 1) != 0;
    let who = if recursive { "create_dir_all" } else { "create_dir" };
    Ok((PlatformRequest::CreateDir { path: path.clone(), recursive }, unit(format!("{who}: {path}"))))
}

pub(super) fn fs_remove_file(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = path_arg(ctx, 0, "remove_file(path)")?;
    let done = unit(format!("remove_file: {path}"));
    Ok((PlatformRequest::RemoveFile { path }, done))
}

pub(super) fn fs_remove_dir(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = path_arg(ctx, 0, "remove_dir(path)")?;
    let recursive = arg_int(ctx, 1) != 0;
    let who = if recursive { "remove_dir_all" } else { "remove_dir" };
    Ok((PlatformRequest::RemoveDir { path: path.clone(), recursive }, unit(format!("{who}: {path}"))))
}

pub(super) fn fs_rename(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let from = path_arg(ctx, 0, "rename(from, to)")?;
    let to = path_arg(ctx, 1, "rename(from, to)")?;
    let done = unit(format!("rename: {from}"));
    Ok((PlatformRequest::Rename { from, to }, done))
}

pub(super) fn fs_stat(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = path_arg(ctx, 0, "stat(path)")?;
    let done = answering(format!("stat: {path}"), |c, r| match r {
        PlatformResponse::Stat(s) => {
            alloc_list(c, &[Value::int(s.kind as i64), Value::int(s.size), Value::int(s.modified_millis)])
        }
        other => Err(format!("stat: platform returned {other:?}")),
    });
    Ok((PlatformRequest::Stat { path }, done))
}

pub(super) fn string_list_arg(ctx: &NativeCallContext<'_>, idx: usize, who: &str) -> Result<Vec<String>, String> {
    let list = ctx.arg(idx).ok_or_else(|| format!("{who}: missing list argument"))?;
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    let backing = ctx.heap.get_slot(list, HEADER_BACKING_SLOT)?;
    (0..len)
        .map(|i| {
            let v = ctx.heap.get_slot(backing, BACKING_DATA_BASE + i)?;
            v.as_heap_string().ok_or_else(|| format!("{who}: list elements must be Strings"))
        })
        .collect()
}

pub(super) fn process_run(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let program = path_arg(ctx, 0, "run(program)")?;
    let args = string_list_arg(ctx, 1, "run(args)")?;
    let env = string_list_arg(ctx, 2, "run(env)")?;
    let input = ctx.arg(3).unwrap_or(Value::null());
    if !is_bytes(input) {
        return Err("run(stdin): stdin must be Bytes".to_string());
    }
    let (backing, len) = bytes_state(ctx, input)?;
    let stdin = ctx.heap.read_bytes(backing, 0, len)?;
    let cwd = path_arg(ctx, 4, "run(cwd)")?;
    let done = answering(format!("run: {program}"), |c, r| match r {
        PlatformResponse::Process(out) => {
            let stdout = alloc_bytes(c, &out.stdout)?;
            let stderr = alloc_bytes(c, &out.stderr)?;
            alloc_list(c, &[Value::int(out.status), stdout, stderr])
        }
        other => Err(format!("run: platform returned {other:?}")),
    });
    Ok((PlatformRequest::RunProcess { program, args, env, stdin, cwd }, done))
}

pub(super) fn http_request(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let method = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let url = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let flat = string_list_arg(ctx, 2, "http(headers)")?;
    let headers = flat.chunks(2).filter(|p| p.len() == 2).map(|p| (p[0].clone(), p[1].clone())).collect();
    let input = ctx.arg(3).unwrap_or(Value::null());
    if !is_bytes(input) {
        return Err("http(body): body must be Bytes".to_string());
    }
    let (backing, len) = bytes_state(ctx, input)?;
    let body = ctx.heap.read_bytes(backing, 0, len)?;
    let timeout_millis = arg_int(ctx, 4).max(1) as u64;
    let max_redirects = arg_int(ctx, 5).clamp(0, 100) as u32;
    let max_body = arg_int(ctx, 6).max(0) as u64;
    let target = url.clone();
    let done: ContinuationFn = Box::new(move |c, result| match result {
        Ok(PlatformResponse::Http { status, headers, body }) => {
            let mut flat = Vec::with_capacity(headers.len() * 2 + 2);
            flat.push(Value::int(status));
            flat.push(alloc_bytes(c, &body)?);
            for (name, value) in &headers {
                flat.push(c.heap.alloc_string(name.as_bytes())?);
                flat.push(c.heap.alloc_string(value.as_bytes())?);
            }
            Ok(alloc_list(c, &flat)?)
        }
        Ok(other) => Err(NativeError::from(format!("http: platform returned {other:?}"))),
        Err(e) => Err(NativeError::failure(e.kind as i32, format!("{target}: {}", e.message))),
    });
    Ok((PlatformRequest::HttpRequest { method, url, headers, body, timeout_millis, max_redirects, max_body }, done))
}

pub(super) const INVALID_DATA: i32 = 2;

pub(super) fn str_parse_int(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let s = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let parsed = if s.starts_with('+') { None } else { s.parse::<i64>().ok() };
    parsed.map(Value::int).ok_or_else(|| NativeError::failure(INVALID_DATA, format!("not an integer: {s}")))
}

pub(super) fn str_parse_float(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let s = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let digits = s.strip_prefix('-').unwrap_or(&s);
    let shaped = digits.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'));
    let parsed = if shaped { s.parse::<f64>().ok().filter(|f| f.is_finite()) } else { None };
    parsed.map(Value::float).ok_or_else(|| NativeError::failure(INVALID_DATA, format!("not a number: {s}")))
}

pub(super) fn port_arg(ctx: &NativeCallContext<'_>, idx: usize, who: &str) -> Result<u16, String> {
    u16::try_from(arg_int(ctx, idx)).map_err(|_| format!("{who}: port must be 0 to 65535"))
}

pub(super) fn net_tcp_connect(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let host = path_arg(ctx, 0, "connect(host, port)")?;
    let done = answering_int(format!("connect: {host}"), "net connect");
    Ok((PlatformRequest::TcpConnect { host, port: port_arg(ctx, 1, "connect")? }, done))
}

pub(super) fn net_tcp_listen(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let host = path_arg(ctx, 0, "listen(host, port)")?;
    let done = answering_int(format!("listen: {host}"), "net listen");
    Ok((PlatformRequest::TcpListen { host, port: port_arg(ctx, 1, "listen")? }, done))
}

fn answering_tls(label: String) -> ContinuationFn {
    Box::new(move |_, result| match result {
        Ok(PlatformResponse::Int(n)) => Ok(Value::int(n)),
        Ok(other) => Err(NativeError::from(format!("{label}: platform returned {other:?}"))),
        Err(e) => Err(NativeError::failure(e.kind as i32, format!("{label}: {}", e.message))),
    })
}

pub(super) fn net_tcp_connect_tls(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let host = path_arg(ctx, 0, "connect_tls(host, port)")?;
    let done = answering_tls(format!("connect_tls {host}"));
    Ok((PlatformRequest::TlsConnect { host, port: port_arg(ctx, 1, "connect_tls")? }, done))
}

pub(super) fn net_tcp_listen_tls(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let host = path_arg(ctx, 0, "listen_tls(host, port, identity)")?;
    let cert_pem = ctx.arg(2).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let key_pem = ctx.arg(3).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let done = answering_tls(format!("listen_tls {host}"));
    Ok((PlatformRequest::TlsListen { host, port: port_arg(ctx, 1, "listen_tls")?, cert_pem, key_pem }, done))
}

pub(super) fn net_tcp_accept(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::TcpAccept { id: arg_int(ctx, 0) }, answering_int("net accept".to_string(), "net accept")))
}

pub(super) fn net_read(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let request = PlatformRequest::SocketRead { id: arg_int(ctx, 0), max: arg_int(ctx, 1).max(0) as usize };
    Ok((request, answering_bytes("net read".to_string(), "net read")))
}

pub(super) fn net_read_for(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let request = PlatformRequest::SocketReadFor {
        id: arg_int(ctx, 0),
        max: arg_int(ctx, 1).max(0) as usize,
        timeout_millis: arg_int(ctx, 2).max(1) as u64,
    };
    Ok((request, answering_bytes("net read_for".to_string(), "net read_for")))
}

pub(super) fn net_write(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let id = arg_int(ctx, 0);
    let b = ctx.arg(1).unwrap_or(Value::null());
    if !is_bytes(b) {
        return Err("net write(id, b): b must be Bytes".to_string());
    }
    let (backing, len) = bytes_state(ctx, b)?;
    let bytes = ctx.heap.read_bytes(backing, 0, len)?;
    Ok((PlatformRequest::SocketWrite { id, bytes }, unit("net write".to_string())))
}

pub(super) fn net_write_text(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let id = arg_int(ctx, 0);
    let bytes = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default().into_bytes();
    Ok((PlatformRequest::SocketWrite { id, bytes }, unit("net write_text".to_string())))
}

pub(super) fn net_shutdown_write(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::SocketShutdownWrite { id: arg_int(ctx, 0) }, unit("net shutdown_write".to_string())))
}

pub(super) fn net_close(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::SocketClose { id: arg_int(ctx, 0) }, unit("net close".to_string())))
}

pub(super) fn net_local_port(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::LocalPort { id: arg_int(ctx, 0) }, answering_int("net local_port".to_string(), "net local_port")))
}

pub(super) fn net_peer_addr(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let done = answering("net peer_addr".to_string(), |c, r| match r {
        PlatformResponse::Text(s) => c.heap.alloc_string(s.as_bytes()),
        other => Err(format!("net peer_addr: platform returned {other:?}")),
    });
    Ok((PlatformRequest::PeerAddr { id: arg_int(ctx, 0) }, done))
}

pub(super) fn net_udp_bind(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let host = path_arg(ctx, 0, "bind(host, port)")?;
    let done = answering_int(format!("bind: {host}"), "net bind");
    Ok((PlatformRequest::UdpBind { host, port: port_arg(ctx, 1, "bind")? }, done))
}

pub(super) fn net_udp_send_to(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let id = arg_int(ctx, 0);
    let host = path_arg(ctx, 1, "send_to(host, port, b)")?;
    let port = port_arg(ctx, 2, "send_to")?;
    let b = ctx.arg(3).unwrap_or(Value::null());
    if !is_bytes(b) {
        return Err("send_to(host, port, b): b must be Bytes".to_string());
    }
    let (backing, len) = bytes_state(ctx, b)?;
    let bytes = ctx.heap.read_bytes(backing, 0, len)?;
    Ok((PlatformRequest::UdpSendTo { id, host, port, bytes }, answering_int("net send_to".to_string(), "net send_to")))
}

pub(super) fn net_udp_recv_from(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let request = PlatformRequest::UdpRecvFrom { id: arg_int(ctx, 0), max: arg_int(ctx, 1).max(0) as usize };
    let done = answering("net recv_from".to_string(), |c, r| match r {
        PlatformResponse::Datagram { bytes, from } => {
            let data = alloc_bytes(c, &bytes)?;
            let from = c.heap.alloc_string(from.as_bytes())?;
            alloc_list(c, &[data, from])
        }
        other => Err(format!("net recv_from: platform returned {other:?}")),
    });
    Ok((request, done))
}

pub(super) const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

pub(super) fn splitmix_advance(state: i64) -> i64 {
    (state as u64).wrapping_add(SPLITMIX_GAMMA) as i64
}

pub(super) fn splitmix_bits(state: i64) -> i64 {
    let mut z = state as u64;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 1) as i64
}

pub(super) fn random_entropy(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    ask_int(ctx, PlatformRequest::Entropy)
}

pub(super) fn time_now_nanos(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    ask_int(ctx, PlatformRequest::MonotonicNanos)
}

pub(super) fn time_unix_millis(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    ask_int(ctx, PlatformRequest::WallClockMillis)
}

pub(super) fn time_local_offset(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    Ok((PlatformRequest::LocalOffset { unix_seconds: arg_int(ctx, 0) }, answering_int("local offset".to_string(), "local offset")))
}

pub(super) fn time_sleep(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let nanos = arg_int(ctx, 0).max(0) as u64;
    Ok((PlatformRequest::Sleep { nanos }, Box::new(|_, _| Ok(Value::null()))))
}

pub(super) fn time_ticker(ctx: &mut NativeCallContext<'_>) -> Result<SourceSpec, String> {
    let period = arg_int(ctx, 0);
    if period < 1 {
        return Err("ticker: the period must be at least 1ns".to_string());
    }
    fn tick(_: &mut dyn NativeCtx, payload: EventPayload) -> Result<Value, String> {
        match payload {
            EventPayload::Int(n) => Ok(Value::int(n)),
            other => Err(format!("ticker: unexpected event {other:?}")),
        }
    }
    Ok(SourceSpec { request: SourceRequest::Timer { period_nanos: period as u64 }, capacity: 8, overflow: Overflow::Pause, decode: tick })
}

