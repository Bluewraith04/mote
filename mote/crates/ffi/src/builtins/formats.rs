//! Format and codec natives over the `base64`, `uuid` and `form_urlencoded` crates.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::*;

pub(super) fn invalid(message: impl Into<String>) -> NativeError {
    NativeError::failure(INVALID_DATA, message)
}

pub(super) fn text_arg(ctx: &NativeCallContext<'_>, i: usize) -> String {
    ctx.arg(i).and_then(|v| v.as_heap_string()).unwrap_or_default()
}

pub(super) fn bytes_arg(ctx: &NativeCallContext<'_>, i: usize) -> Result<Vec<u8>, String> {
    let b = ctx.arg(i).ok_or("missing Bytes argument")?;
    let (backing, len) = bytes_state(ctx, b)?;
    Ok(ctx.heap.read_bytes(backing, 0, len)?.to_vec())
}

/// Runs `f` on the bytes of the `Bytes` argument `i`, read in place.
fn with_bytes<R>(ctx: &NativeCallContext<'_>, i: usize, f: impl FnOnce(&[u8]) -> R) -> Result<R, String> {
    let b = ctx.arg(i).ok_or("missing Bytes argument")?;
    let (backing, len) = bytes_state(ctx, b)?;
    if len == 0 {
        return Ok(f(&[]));
    }
    let ptr = ctx.heap.bytes_ptr(backing)?;
    // SAFETY: the backing holds `len` initialised bytes, and objects never move or free during a native call.
    Ok(f(unsafe { std::slice::from_raw_parts(ptr, len) }))
}

/// The first index at or after argument 1 whose byte `stop` accepts, or the length.
fn scan_from(ctx: &NativeCallContext<'_>, stop: impl Fn(u8) -> bool) -> Result<Value, String> {
    let from = arg_int(ctx, 1).max(0) as usize;
    let end = with_bytes(ctx, 0, |b| b.get(from..).and_then(|rest| rest.iter().position(|&c| stop(c))).map_or(b.len(), |i| from + i))?;
    Ok(Value::int(end as i64))
}

/// `(text, from)`: the index of the first `"`, `\` or control byte at or after `from`; the length if none.
pub(super) fn json_scan_string(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    scan_from(ctx, |c| c == b'"' || c == b'\\' || c < 32)
}

/// `(text, from)`: the index of the first byte after `from` that is not JSON white space.
pub(super) fn json_skip_ws(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    scan_from(ctx, |c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
}

/// `(text, from)`: the index of the first byte at or after `from` that is not an ASCII digit.
pub(super) fn json_digits_end(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    scan_from(ctx, |c| !c.is_ascii_digit())
}

/// `s` as a JSON string literal, quotes included.
pub(super) fn json_quote(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let text = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let mut out = Vec::with_capacity(text.len() + 2);
    out.push(b'"');
    for &c in text.as_bytes() {
        match c {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            8 => out.extend_from_slice(b"\\b"),
            9 => out.extend_from_slice(b"\\t"),
            10 => out.extend_from_slice(b"\\n"),
            12 => out.extend_from_slice(b"\\f"),
            13 => out.extend_from_slice(b"\\r"),
            c if c < 32 => out.extend_from_slice(format!("\\u{c:04x}").as_bytes()),
            c => out.push(c),
        }
    }
    out.push(b'"');
    ctx.heap.alloc_string(&out)
}

/// The terminal cells of `text`, one entry a column.
fn terminal_cells(text: &str) -> Vec<String> {
    if text.is_ascii() {
        return text.chars().map(|c| c.to_string()).collect();
    }
    let mut out: Vec<String> = Vec::with_capacity(text.len());
    for g in text.graphemes(true) {
        if g.chars().next().is_some_and(char::is_control) {
            out.extend(g.chars().map(|c| c.to_string()));
            continue;
        }
        let width = UnicodeWidthStr::width(g);
        if width == 0 {
            match out.iter().rposition(|e| !e.is_empty()) {
                Some(i) => out[i].push_str(g),
                None => out.push(g.to_string()),
            }
            continue;
        }
        out.push(g.to_string());
        out.extend(std::iter::repeat_n(String::new(), width - 1));
    }
    out
}

/// The columns `text` takes on a terminal, as one entry each, a wide character followed by an empty entry.
pub(super) fn term_glyphs(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let text = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let cells: Vec<Value> = terminal_cells(&text)
        .iter()
        .map(|c| ctx.heap.alloc_string(c.as_bytes()))
        .collect::<Result<_, _>>()?;
    build_list(ctx, &cells)
}

/// How many columns `text` takes on a terminal.
pub(super) fn term_width(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let text = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    Ok(Value::int(terminal_cells(&text).len() as i64))
}

/// `parts` joined with `sep` between each.
pub(super) fn string_join(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let parts = super::system::string_list_arg(ctx, 0, "join")?;
    let sep = ctx.arg(1).and_then(|v| v.as_heap_string()).unwrap_or_default();
    ctx.heap.alloc_string(parts.join(&sep).as_bytes())
}

pub(super) fn base64_encode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let data = bytes_arg(ctx, 0)?;
    let url = ctx.arg(1).and_then(|v| v.as_bool()).unwrap_or(false);
    let text = if url { URL_SAFE_NO_PAD.encode(data) } else { STANDARD.encode(data) };
    ctx.heap.alloc_string(text.as_bytes())
}

pub(super) fn base64_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let text = text_arg(ctx, 0);
    let url = ctx.arg(1).and_then(|v| v.as_bool()).unwrap_or(false);
    let decoded = if url { URL_SAFE_NO_PAD.decode(text) } else { STANDARD.decode(text) };
    let data = decoded.map_err(|e| invalid(e.to_string()))?;
    Ok(alloc_bytes(ctx, &data)?)
}

pub(super) fn form_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let text = text_arg(ctx, 0);
    let mut flat = Vec::new();
    for (name, value) in form_urlencoded::parse(text.as_bytes()) {
        flat.push(ctx.heap.alloc_string(name.as_bytes())?);
        flat.push(ctx.heap.alloc_string(value.as_bytes())?);
    }
    alloc_list(ctx, &flat)
}

pub(super) fn percent_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let text = text_arg(ctx, 0);
    ctx.heap.alloc_string(percent_encoding::percent_decode_str(&text).decode_utf8_lossy().as_bytes())
}

fn uuid_error(e: uuid::Error) -> NativeError {
    invalid(e.to_string().replace(": ", ", "))
}

fn alloc_uuid(ctx: &mut NativeCallContext<'_>, id: uuid::Uuid) -> Result<Value, String> {
    ctx.heap.alloc_string(id.hyphenated().to_string().as_bytes())
}

pub(super) fn uuid_v4(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    alloc_uuid(ctx, uuid::Uuid::new_v4())
}

pub(super) fn uuid_v7(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    alloc_uuid(ctx, uuid::Uuid::now_v7())
}

pub(super) fn uuid_parse(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let id = uuid::Uuid::parse_str(&text_arg(ctx, 0)).map_err(uuid_error)?;
    Ok(alloc_uuid(ctx, id)?)
}

pub(super) fn uuid_from_bytes(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let id = uuid::Uuid::from_slice(&bytes_arg(ctx, 0)?).map_err(uuid_error)?;
    Ok(alloc_uuid(ctx, id)?)
}

pub(super) fn uuid_bytes(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(&text_arg(ctx, 0)).map_err(|e| e.to_string())?;
    alloc_bytes(ctx, id.as_bytes())
}

pub(super) fn uuid_version(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(&text_arg(ctx, 0)).map_err(|e| e.to_string())?;
    Ok(Value::int(id.get_version_num() as i64))
}

#[cfg(test)]
mod tests {
    use super::terminal_cells;

    fn cells(text: &str) -> Vec<String> {
        terminal_cells(text)
    }

    #[test]
    fn ascii_is_a_cell_a_character_and_a_control_character_is_its_own() {
        assert_eq!(cells("a b"), ["a", " ", "b"]);
        assert_eq!(cells("a\r\nb"), ["a", "\r", "\n", "b"]);
        assert!(cells("").is_empty());
    }

    #[test]
    fn a_wide_character_is_followed_by_an_empty_entry() {
        assert_eq!(cells("日本"), ["日", "", "本", ""]);
        assert_eq!(cells("a😀b"), ["a", "😀", "", "b"]);
        assert_eq!(cells("✅"), ["✅", ""]);
    }

    #[test]
    fn a_combining_mark_joins_the_character_before_it() {
        assert_eq!(cells("e\u{301}x"), ["e\u{301}", "x"]);
        assert_eq!(cells("日\u{301}"), ["日\u{301}", ""]);
    }

    #[test]
    fn a_mark_with_nothing_before_it_stands_alone() {
        assert_eq!(cells("\u{301}a"), ["\u{301}", "a"]);
    }

    #[test]
    fn a_sequence_that_draws_as_one_emoji_is_two_cells() {
        assert_eq!(cells("👨\u{200d}👩\u{200d}👧").len(), 2);
        assert_eq!(cells("🇯🇵").len(), 2);
    }

    #[test]
    fn non_ascii_text_that_is_narrow_stays_one_cell_each() {
        assert_eq!(cells("é•→"), ["é", "•", "→"]);
    }
}
