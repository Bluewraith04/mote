//! Regular-expression natives over the `regex` crate; a compiled pattern is kept by its text.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use regex::bytes::Regex;

use super::*;

/// Compiled patterns kept at once; the table is emptied when it fills.
const CACHE_LIMIT: usize = 256;

fn cache() -> &'static Mutex<HashMap<String, Arc<Regex>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Regex>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The compiler's reason on one line, without its picture of the pattern.
fn reason(e: &regex::Error) -> String {
    let text = e.to_string();
    match text.lines().rev().find_map(|l| l.strip_prefix("error: ")) {
        Some(r) => r.to_string(),
        None => text.lines().next().unwrap_or("bad pattern").to_string(),
    }
}

/// The compiled `pattern`, from the cache or built now.
fn compiled(pattern: &str) -> Result<Arc<Regex>, String> {
    if let Some(re) = cache().lock().get(pattern) {
        return Ok(re.clone());
    }
    regex::Regex::new(pattern).map_err(|e| reason(&e))?;
    let re = Arc::new(Regex::new(pattern).map_err(|e| reason(&e))?);
    let mut table = cache().lock();
    if table.len() >= CACHE_LIMIT {
        table.clear();
    }
    table.insert(pattern.to_string(), re.clone());
    Ok(re)
}

/// The text of argument `i`, read in place; valid until the heap next collects.
fn text_in_place(v: &Value) -> Result<&[u8], String> {
    v.heap_bytes().or_else(|| v.as_short_str().map(str::as_bytes)).ok_or_else(|| "regex: expected a String".to_string())
}

/// Whether `i` is between characters of the UTF-8 text `hay`.
fn on_boundary(hay: &[u8], i: usize) -> bool {
    i >= hay.len() || (hay[i] & 0xC0) != 0x80
}

/// The first match at or after `start` as `[start, end]` for the whole match and each group.
fn captures_flat(re: &Regex, hay: &[u8], start: usize) -> Vec<i64> {
    let mut at = start;
    while at <= hay.len() {
        let Some(c) = re.captures_at(hay, at) else { break };
        let whole = c.get(0).expect("group 0 always exists");
        if whole.is_empty() && !on_boundary(hay, whole.start()) {
            at = whole.start() + 1;
            continue;
        }
        return (0..c.len())
            .flat_map(|i| match c.get(i) {
                Some(m) => [m.start() as i64, m.end() as i64],
                None => [-1, -1],
            })
            .collect();
    }
    Vec::new()
}

/// The first match at or after `start` as `[start, end]`, found without resolving groups.
fn find_flat(re: &Regex, hay: &[u8], start: usize) -> Vec<i64> {
    let mut at = start;
    while at <= hay.len() {
        let Some(m) = re.find_at(hay, at) else { break };
        if m.is_empty() && !on_boundary(hay, m.start()) {
            at = m.start() + 1;
            continue;
        }
        return vec![m.start() as i64, m.end() as i64];
    }
    Vec::new()
}

/// Every non-overlapping match as `[start, end]` pairs, left to right.
fn find_all_flat(re: &Regex, hay: &[u8]) -> Vec<i64> {
    re.find_iter(hay)
        .filter(|m| !m.is_empty() || on_boundary(hay, m.start()))
        .flat_map(|m| [m.start() as i64, m.end() as i64])
        .collect()
}

fn int_list(ctx: &mut NativeCallContext<'_>, numbers: &[i64]) -> Result<Value, String> {
    let items: Vec<Value> = numbers.iter().map(|&n| Value::int(n)).collect();
    build_list(ctx, &items)
}

/// `(pattern)`: the group names, index 0 for the whole match.
pub(super) fn regex_compile(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let pattern = text_arg(ctx, 0);
    let re = compiled(&pattern).map_err(super::formats::invalid)?;
    let names: Vec<Value> =
        re.capture_names().map(|n| ctx.heap.alloc_string(n.unwrap_or("").as_bytes())).collect::<Result<_, _>>()?;
    Ok(build_list(ctx, &names)?)
}

/// `(pattern, text)`: whether the pattern matches anywhere in the text.
pub(super) fn regex_is_match(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let re = compiled(&text_arg(ctx, 0))?;
    let text = ctx.arg(1).ok_or("regex: missing text")?;
    Ok(Value::Bool(re.is_match(text_in_place(&text)?)))
}

/// `(pattern, text, start)`: `captures_flat` from byte offset `start`.
pub(super) fn regex_captures_at(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let re = compiled(&text_arg(ctx, 0))?;
    let text = ctx.arg(1).ok_or("regex: missing text")?;
    let flat = captures_flat(&re, text_in_place(&text)?, arg_int(ctx, 2).max(0) as usize);
    int_list(ctx, &flat)
}

/// `(pattern, text, start)`: `find_flat` from byte offset `start`.
pub(super) fn regex_find_at(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let re = compiled(&text_arg(ctx, 0))?;
    let text = ctx.arg(1).ok_or("regex: missing text")?;
    let flat = find_flat(&re, text_in_place(&text)?, arg_int(ctx, 2).max(0) as usize);
    int_list(ctx, &flat)
}

/// `(pattern, text)`: `find_all_flat`.
pub(super) fn regex_find_all(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let re = compiled(&text_arg(ctx, 0))?;
    let text = ctx.arg(1).ok_or("regex: missing text")?;
    let flat = find_all_flat(&re, text_in_place(&text)?);
    int_list(ctx, &flat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(pattern: &str) -> Arc<Regex> {
        compiled(pattern).unwrap()
    }

    #[test]
    fn a_match_is_reported_in_bytes_on_character_boundaries() {
        let text = "日本語 abc";
        assert_eq!(captures_flat(&re("."), text.as_bytes(), 0), vec![0, 3]);
        assert_eq!(captures_flat(&re("^.{3}"), text.as_bytes(), 0), vec![0, 9]);
        assert_eq!(captures_flat(&re("\\w+"), "naïve x".as_bytes(), 0), vec![0, 6]);
    }

    #[test]
    fn groups_that_did_not_match_are_minus_one() {
        assert_eq!(captures_flat(&re("(a)|(b)"), b"b", 0), vec![0, 1, -1, -1, 0, 1]);
        assert_eq!(captures_flat(&re("x"), b"abc", 0), Vec::<i64>::new());
    }

    #[test]
    fn a_search_from_an_offset_still_sees_what_is_before_it() {
        assert_eq!(captures_flat(&re("\\bb"), b"ab b", 1), vec![3, 4]);
        assert_eq!(captures_flat(&re("^a"), b"aa", 1), Vec::<i64>::new());
        assert_eq!(captures_flat(&re("a"), b"a", 5), Vec::<i64>::new());
    }

    #[test]
    fn find_gives_the_span_of_what_captures_gives_for_the_whole_match() {
        for (pattern, text, start) in [("\\d+", "a1 b22", 0), ("\\d+", "a1 b22", 2), ("^b", "a\nb", 2), ("(?m)^b", "a\nb", 1), ("", "é", 1), ("\\B", "é", 0), ("x", "abc", 0)] {
            let whole: Vec<i64> = captures_flat(&re(pattern), text.as_bytes(), start).into_iter().take(2).collect();
            assert_eq!(find_flat(&re(pattern), text.as_bytes(), start), whole, "{pattern} on {text:?} from {start}");
        }
    }

    #[test]
    fn find_all_walks_the_text_and_steps_over_empty_matches() {
        assert_eq!(find_all_flat(&re("\\d+"), b"a1 b22 c333"), vec![1, 2, 4, 6, 8, 11]);
        assert_eq!(find_all_flat(&re("a*"), b"baaac"), vec![0, 0, 1, 4, 5, 5]);
        assert_eq!(find_all_flat(&re(""), "é".as_bytes()), vec![0, 0, 2, 2]);
    }

    #[test]
    fn an_empty_match_never_lands_inside_a_character() {
        assert_eq!(captures_flat(&re("\\B"), "é".as_bytes(), 0), Vec::<i64>::new());
        assert_eq!(captures_flat(&re(""), "é".as_bytes(), 1), vec![2, 2]);
        assert_eq!(find_all_flat(&re("\\B"), "日本".as_bytes()), vec![3, 3]);
    }

    #[test]
    fn flags_and_classes_of_the_crate_work() {
        assert_eq!(captures_flat(&re("(?i)hello"), b"say HeLLo", 0), vec![4, 9]);
        assert_eq!(find_all_flat(&re("(?m)^b"), b"a\nb\nb"), vec![2, 3, 4, 5]);
        assert_eq!(captures_flat(&re("[[:alpha:]]+ world"), b"hello world", 0), vec![0, 11]);
    }

    #[test]
    fn a_bad_pattern_is_one_short_clause() {
        assert_eq!(compiled("a(b").err().unwrap(), "unclosed group");
        assert_eq!(compiled("[z-a]").err().unwrap(), "invalid character class range, the start must be <= the end");
        assert!(compiled("(a)\\1").is_err());
        assert!(compiled("wor(?=ld)").is_err());
        assert!(!compiled("a(b").err().unwrap().contains('\n'));
    }

    #[test]
    fn a_pattern_that_could_match_bytes_that_are_not_text_is_refused() {
        assert!(compiled("(?-u:\\xFF)").is_err());
    }

    #[test]
    fn the_cache_keeps_a_pattern_and_empties_when_full() {
        let first = re("cache_probe_0");
        assert!(Arc::ptr_eq(&first, &re("cache_probe_0")));
        for i in 0..CACHE_LIMIT + 4 {
            compiled(&format!("cache_probe_{i}_x")).unwrap();
        }
        assert!(cache().lock().len() <= CACHE_LIMIT);
    }
}
