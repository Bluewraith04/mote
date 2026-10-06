//! Memory limits a run applies before it starts.

/// Environment variable overriding the per-task stack limit.
pub const MAX_STACK_ENV: &str = "MOTE_MAX_STACK";

/// Environment variable overriding the heap limit.
pub const MAX_HEAP_ENV: &str = "MOTE_MAX_HEAP";

/// The heap limit in bytes (`None` is unlimited): `--max-heap`, then `MOTE_MAX_HEAP`, then `[run] max-heap`, then half of `available` memory.
pub fn resolve_max_heap(flag: Option<&str>, env: Option<&str>, manifest: Option<&str>, available: Option<u64>) -> Result<Option<usize>, String> {
    for (source, raw) in [("--max-heap", flag), (MAX_HEAP_ENV, env), ("[run] max-heap", manifest)] {
        if let Some(raw) = raw {
            return parse_size(raw).map_err(|e| format!("{source}: {e}"));
        }
    }
    Ok(available.map(|bytes| usize::try_from(bytes / 2).unwrap_or(usize::MAX)))
}

/// Parses `64MiB`, `512k`, `2G`, a plain byte count, or `unlimited` (`None`).
pub fn parse_size(text: &str) -> Result<Option<usize>, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("unlimited") {
        return Ok(None);
    }
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (number, unit) = text.split_at(digits);
    let scale: usize = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        _ => return Err(format!("'{text}' is not a size (try 64MiB or 2GiB)")),
    };
    let count: usize = number.parse().map_err(|_| format!("'{text}' is not a size (try 64MiB or 2GiB)"))?;
    count.checked_mul(scale).map(Some).ok_or_else(|| format!("'{text}' is too large"))
}

/// Applies the stack limit from `MOTE_MAX_STACK`, if set.
pub fn apply(rt: &mut runtime::Runtime) -> Result<(), String> {
    if let Ok(raw) = std::env::var(MAX_STACK_ENV) {
        let bytes = parse_size(&raw).map_err(|e| format!("{MAX_STACK_ENV}: {e}"))?;
        rt.set_max_stack(bytes.unwrap_or(usize::MAX));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_size, resolve_max_heap};

    #[test]
    fn the_heap_limit_takes_flag_then_env_then_manifest_then_half_of_memory() {
        let gib = 1u64 << 30;
        assert_eq!(resolve_max_heap(Some("1GiB"), Some("2GiB"), Some("3GiB"), Some(16 * gib)), Ok(Some(1 << 30)));
        assert_eq!(resolve_max_heap(None, Some("2GiB"), Some("3GiB"), Some(16 * gib)), Ok(Some(2 << 30)));
        assert_eq!(resolve_max_heap(None, None, Some("3GiB"), Some(16 * gib)), Ok(Some(3 << 30)));
        assert_eq!(resolve_max_heap(None, None, None, Some(16 * gib)), Ok(Some(8 << 30)));
        assert_eq!(resolve_max_heap(None, None, None, None), Ok(None));
        assert_eq!(resolve_max_heap(Some("unlimited"), None, None, Some(16 * gib)), Ok(None));
    }

    #[test]
    fn a_bad_heap_limit_names_where_it_came_from() {
        let err = resolve_max_heap(None, Some("lots"), None, None).unwrap_err();
        assert!(err.starts_with("MOTE_MAX_HEAP: "), "{err}");
    }

    #[test]
    fn sizes_take_units() {
        assert_eq!(parse_size("64MiB"), Ok(Some(64 << 20)));
        assert_eq!(parse_size("512k"), Ok(Some(512 << 10)));
        assert_eq!(parse_size(" 2 GiB "), Ok(Some(2 << 30)));
        assert_eq!(parse_size("4096"), Ok(Some(4096)));
    }

    #[test]
    fn unlimited_is_none() {
        assert_eq!(parse_size("Unlimited"), Ok(None));
    }

    #[test]
    fn junk_is_an_error() {
        assert!(parse_size("lots").is_err());
        assert!(parse_size("12 parsecs").is_err());
        assert!(parse_size("").is_err());
    }
}
