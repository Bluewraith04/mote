//! `mote check --memory`: where each struct literal in the user's code lives.

use compiler::regions::{HeapReason, MemorySite};
use compiler::span::{source_info, Span};

fn at(span: Span) -> String {
    format!("{}:{}", span.line, span.col)
}

fn describe(site: &MemorySite) -> String {
    if site.in_registers {
        return "registers, no memory".to_string();
    }
    match &site.reason {
        None => "region, freed when its block ends".to_string(),
        Some(HeapReason::Redeclared(n)) => format!("heap: `{n}` is declared more than once"),
        Some(HeapReason::WholeValue(n, s)) => format!("heap: `{n}` is used as a whole value at {}", at(*s)),
        Some(HeapReason::Retained(n, s)) => format!("heap: `{n}` is passed at {} to a call that may keep it", at(*s)),
        Some(HeapReason::Captured(n, s)) => format!("heap: `{n}` is used inside a spawn or lambda at {}", at(*s)),
        Some(HeapReason::SharedVar(n)) => format!("heap: var `{n}` is shared with a lambda that assigns it"),
        Some(HeapReason::TooManyFields(n)) => format!("heap: {n} fields, and a region holds at most {}", isa::value::MAX_REGION_SLOTS),
        Some(HeapReason::NotBound) => "heap: not the initializer of a let or var".to_string(),
        Some(HeapReason::Unanalysed) => "heap: at module level".to_string(),
    }
}

/// One line per struct literal in a user module, in source order, then a summary; std modules are left out.
pub fn lines(sites: &[MemorySite]) -> Vec<String> {
    let mut rows: Vec<(String, usize, usize, String)> = Vec::new();
    let (mut registers, mut regions) = (0, 0);
    for site in sites {
        let Some((path, _)) = source_info(site.span.source) else { continue };
        if path.starts_with("<std>") {
            continue;
        }
        let row = (path, site.span.line, site.span.col, format!("{} {{ .. }}  {}", site.type_name, describe(site)));
        if !rows.contains(&row) {
            registers += site.in_registers as usize;
            regions += (!site.in_registers && site.reason.is_none()) as usize;
            rows.push(row);
        }
    }
    rows.sort();
    let mut out: Vec<String> = rows.iter().map(|(path, line, col, text)| format!("{path}:{line}:{col}  {text}")).collect();
    out.push(format!("{} struct literal(s): {} in registers, {} in regions, {} on the heap.", rows.len(), registers, regions, rows.len() - registers - regions));
    out.push("Lists, maps, classes, strings and closures are always on the heap.".to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(reason: Option<HeapReason>, in_registers: bool) -> MemorySite {
        MemorySite { span: Span::new(0, 0, 1, 1), type_name: "P".to_string(), reason, in_registers }
    }

    #[test]
    fn reasons_read_as_one_fact() {
        assert_eq!(describe(&site(None, false)), "region, freed when its block ends");
        assert_eq!(describe(&site(None, true)), "registers, no memory");
        assert_eq!(describe(&site(Some(HeapReason::TooManyFields(70)), false)), "heap: 70 fields, and a region holds at most 63");
    }

    #[test]
    fn a_program_with_no_sites_still_prints_the_summary() {
        let out = lines(&[]);
        assert_eq!(out[0], "0 struct literal(s): 0 in registers, 0 in regions, 0 on the heap.");
    }
}
