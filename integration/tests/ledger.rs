//! Every bullet and table row of the documents has a proof form or is a line of `docs/unproven`.

mod common;

use common::{files_with, fnv, relative, root, TABLE_CLOSE, TABLE_OPEN};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The names a marker can refer to.
struct Known {
    /// The tests of the repository.
    tests: BTreeSet<String>,
    /// The slices of `tasks.md`, as `S12`.
    slices: BTreeSet<String>,
}

/// A bullet, table row or `Done when` line of a document.
#[derive(Debug)]
struct Statement {
    line: usize,
    text: String,
    /// Inside a generated table.
    generated: bool,
    /// The marker on the statement, or on the line above its table.
    marker: Option<Marker>,
    /// A `mote` block comes right after the statement or its table.
    block_follows: bool,
    /// The slices named in the `Slice` or `Slices` row of its section.
    slices: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Marker {
    /// The tests named, separated by commas.
    Test(Vec<String>),
    Block,
}

/// The statements with no proof, and the markers that prove nothing.
struct Judged {
    unproven: Vec<Statement>,
    broken: Vec<String>,
}

fn marker_in(text: &str) -> Option<Marker> {
    if text.contains("<!-- block -->") {
        return Some(Marker::Block);
    }
    let rest = &text[text.find("<!-- test: ")? + "<!-- test: ".len()..];
    Some(Marker::Test(rest[..rest.find(" -->")?].split(',').map(|name| name.trim().to_string()).collect()))
}

/// The text after a bullet marker, if the line is a bullet.
fn bullet(trimmed: &str) -> Option<&str> {
    if let Some(rest) = trimmed.strip_prefix("* ").or_else(|| trimmed.strip_prefix("- ")) {
        return Some(rest);
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 { trimmed[digits..].strip_prefix(". ") } else { None }
}

fn is_separator(row: &str) -> bool {
    row.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// The slice numbers in a cell, as `S12`.
fn slice_names(cell: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = cell;
    while let Some(at) = rest.find('S') {
        let digits: String = rest[at + 1..].chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            names.push(format!("S{digits}"));
        }
        rest = &rest[at + 1..];
    }
    names
}

/// For each line, the slices named by the `Slice` or `Slices` row of the `##` section it is in.
fn slices_of(lines: &[&str]) -> Vec<Vec<String>> {
    let mut found = vec![Vec::new(); lines.len()];
    let mut start = 0;
    for end in 1..=lines.len() {
        if end == lines.len() || lines[end].starts_with("## ") {
            let slices = lines[start..end]
                .iter()
                .find_map(|line| {
                    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                    (cells.len() >= 3 && matches!(cells[1], "Slice" | "Slices")).then(|| slice_names(cells[2]))
                })
                .unwrap_or_default();
            for entry in &mut found[start..end] {
                entry.clone_from(&slices);
            }
            start = end;
        }
    }
    found
}

/// The statements of a document outside code fences.
fn statements(text: &str, core: bool) -> Vec<Statement> {
    let lines: Vec<&str> = text.lines().collect();
    let slices = if core { slices_of(&lines) } else { vec![Vec::new(); lines.len()] };
    let block_after = |end: usize| {
        let mut marked_table = false;
        for line in &lines[end..] {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                marked_table = false;
            } else if trimmed == "<!-- block -->" {
                marked_table = true;
            } else if !(line.contains("<!-- block -->") || line.starts_with("  ") || (marked_table && trimmed.starts_with('|'))) {
                return trimmed.starts_with("```mote");
            }
        }
        false
    };
    let mut found = Vec::new();
    let (mut fenced, mut generated) = (false, false);
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
        } else if fenced {
        } else if trimmed.starts_with(TABLE_OPEN) {
            generated = true;
        } else if trimmed.trim_end() == TABLE_CLOSE {
            generated = false;
        } else if let Some(first) = bullet(trimmed) {
            let mut end = i + 1;
            let mut joined = first.trim().to_string();
            while end < lines.len() && lines[end].starts_with("  ") && bullet(lines[end].trim_start()).is_none() && !lines[end].trim_start().starts_with("```") && !lines[end].trim_start().starts_with('|') {
                joined.push(' ');
                joined.push_str(lines[end].trim());
                end += 1;
            }
            found.push(Statement { line: i + 1, marker: marker_in(&joined), block_follows: block_after(end), text: joined, generated, slices: slices[i].clone() });
            i = end;
            continue;
        } else if trimmed.starts_with('|') {
            let mut end = i;
            while end < lines.len() && lines[end].trim_start().starts_with('|') {
                end += 1;
            }
            let above = if i > 0 && lines[i - 1].starts_with("<!-- ") { marker_in(lines[i - 1]) } else { None };
            let follows = block_after(end);
            for row in i..end {
                let row_text = lines[row].trim();
                if is_separator(row_text) {
                    continue;
                }
                let marker = marker_in(row_text).or_else(|| above.clone());
                found.push(Statement { line: row + 1, text: row_text.to_string(), generated, marker, block_follows: follows, slices: slices[row].clone() });
            }
            i = end;
            continue;
        } else if trimmed.starts_with("Done when:") {
            found.push(Statement { line: i + 1, text: trimmed.trim_end().to_string(), generated, marker: None, block_follows: false, slices: slices[i].clone() });
        }
        i += 1;
    }
    found
}

/// The tests of `mote-syntax` that give every rule of `Syntax.md` an accepted and a rejected input.
const GRAMMAR_TESTS: [&str; 2] = ["grammar_every_rule_has_an_accepted_and_a_rejected_input", "grammar_every_case_gives_the_result_it_records"];

/// A bullet of the grammar: `` `Rule` ↔ … ``.
fn is_grammar_rule(text: &str) -> bool {
    text.starts_with('`') && text.contains(" ↔ ")
}

/// Splits the statements of a document into the proven and the unproven.
fn judge(text: &str, core: bool, known: &Known) -> Judged {
    let mut judged = Judged { unproven: Vec::new(), broken: Vec::new() };
    for statement in statements(text, core) {
        let proven = if statement.generated {
            true
        } else if is_grammar_rule(&statement.text) {
            if let Some(name) = GRAMMAR_TESTS.iter().find(|name| !known.tests.contains(**name)) {
                judged.broken.push(format!("line {}: the grammar test `{name}` is gone", statement.line));
                continue;
            }
            true
        } else {
            match &statement.marker {
                Some(Marker::Test(names)) => {
                    if let Some(name) = names.iter().find(|name| !known.tests.contains(*name)) {
                        judged.broken.push(format!("line {}: the marker names no test `{name}`", statement.line));
                        continue;
                    }
                    true
                }
                Some(Marker::Block) if statement.block_follows => true,
                Some(Marker::Block) => {
                    judged.broken.push(format!("line {}: the marker is not followed by a `mote` block", statement.line));
                    continue;
                }
                None => !statement.slices.is_empty() && statement.slices.iter().all(|slice| known.slices.contains(slice)),
            }
        };
        if !proven {
            judged.unproven.push(statement);
        }
    }
    judged
}

/// The `hash | start of the text` line of a statement.
fn entry(statement: &Statement) -> (String, String) {
    let start: String = statement.text.chars().take(70).collect();
    (fnv(&statement.text)[..8].to_string(), start)
}

/// The text of a list file for the unproven statements of a document.
fn render(label: &str, unproven: &[Statement]) -> String {
    let mut out = format!("# Statements of {label} with no proof form. Each line is `hash | start of the text`; the ledger test fails on an unproven statement that is not listed and on a listed one that is proven, so this list only shrinks.\n");
    let mut seen = BTreeSet::new();
    for statement in unproven {
        let (hash, start) = entry(statement);
        if seen.insert(hash.clone()) {
            out.push_str(&format!("{hash} | {start}\n"));
        }
    }
    out
}

/// What differs between the unproven statements and the list text.
fn compare(label: &str, unproven: &[Statement], listed: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut listed_hashes = BTreeSet::new();
    for (number, line) in listed.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        match line.split_once(" | ") {
            Some((hash, _)) if !listed_hashes.insert(hash.to_string()) => problems.push(format!("{label}: the list holds `{hash}` twice (line {})", number + 1)),
            Some(_) => {}
            None => problems.push(format!("{label}: line {} of the list is not `hash | text`", number + 1)),
        }
    }
    let mut current = BTreeSet::new();
    for statement in unproven {
        let (hash, start) = entry(statement);
        if !listed_hashes.contains(&hash) && current.insert(hash.clone()) {
            problems.push(format!("{label}:{}: no proof form; prove it or add the line: {hash} | {start}", statement.line));
        }
        current.insert(hash);
    }
    for hash in &listed_hashes {
        if !current.contains(hash) {
            problems.push(format!("{label}: remove the line {hash}: its statement is proven, changed or gone"));
        }
    }
    problems
}

fn known() -> Known {
    let mut tests = BTreeSet::new();
    for path in files_with("rs") {
        let text = std::fs::read_to_string(&path).expect("a readable source file");
        let lines: Vec<&str> = text.lines().collect();
        for (at, line) in lines.iter().enumerate() {
            let Some(rest) = line.trim_start().strip_prefix("fn ") else { continue };
            let is_test = lines[at.saturating_sub(2)..at].iter().any(|before| before.trim() == "#[test]");
            if is_test {
                tests.insert(rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect());
            }
        }
    }
    let tasks = std::fs::read_to_string(root().join("tasks.md")).expect("tasks.md");
    let slices = tasks.lines().filter_map(|line| line.strip_prefix("- [x] **").or_else(|| line.strip_prefix("- [ ] **"))).filter_map(|rest| rest.split_once("**").map(|(slice, _)| slice.to_string())).collect();
    Known { tests, slices }
}

/// The documents the ledger covers.
fn documents() -> Vec<PathBuf> {
    files_with("md")
        .into_iter()
        .filter(|path| {
            let label = relative(path);
            let hand_reference = label.starts_with("core/") && label.ends_with("/reference.md");
            let about_the_project = label == "README.md" || label == "CHANGELOG.md";
            label != "tasks.md" && !about_the_project && !label.starts_with("demo/") && !label.starts_with("tools/fixtures/") && !label.starts_with("docs/unproven/") && !label.starts_with("editors/") && !hand_reference
        })
        .collect()
}

fn list_path(label: &str) -> PathBuf {
    root().join("docs/unproven").join(format!("{label}.txt"))
}

#[test]
fn the_ledger_matches_the_lists() {
    let known = known();
    let mut problems = Vec::new();
    let mut expected = BTreeSet::new();
    for path in documents() {
        let label = relative(&path);
        let text = std::fs::read_to_string(&path).expect("a readable document");
        let judged = judge(&text, label == "core/CORE.md", &known);
        problems.extend(judged.broken.iter().map(|problem| format!("{label}: {problem}")));
        expected.insert(list_path(&label));
        let listed = std::fs::read_to_string(list_path(&label)).unwrap_or_default();
        problems.extend(compare(&label, &judged.unproven, &listed));
    }
    for path in files_with("txt") {
        if relative(&path).starts_with("docs/unproven/") && !expected.contains(&path) {
            problems.push(format!("{}: no document has this list; delete it", relative(&path)));
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

fn sample_known() -> Known {
    Known { tests: ["proves_it".to_string()].into(), slices: ["S5".to_string()].into() }
}

#[test]
fn statements_are_bullets_rows_and_done_when_lines_outside_fences() {
    let text = "# T\n\n* one\n  and more\n1. two\n\n| a | b |\n|---|---|\n| c | d |\n\n```text\n* not\n| not |\n```\n\nDone when: it works.\nProse is not one.\n";
    let found: Vec<(usize, String)> = statements(text, false).into_iter().map(|s| (s.line, s.text)).collect();
    assert_eq!(found, vec![(3, "one and more".to_string()), (5, "two".to_string()), (7, "| a | b |".to_string()), (9, "| c | d |".to_string()), (16, "Done when: it works.".to_string())]);
}

#[test]
fn a_generated_table_proves_its_rows() {
    let text = "<!-- table: x -->\n| a | b |\n|---|---|\n| c | d |\n<!-- /table -->\n\n* after\n";
    let judged = judge(text, false, &sample_known());
    assert_eq!(judged.unproven.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), vec!["after"]);
}

#[test]
fn a_test_marker_proves_a_bullet_and_a_table() {
    let text = "* said <!-- test: proves_it -->\n* said <!-- test: nothing -->\n\n<!-- test: proves_it -->\n| a | b |\n| c | d |\n";
    let judged = judge(text, false, &sample_known());
    assert!(judged.unproven.is_empty());
    assert_eq!(judged.broken, vec!["line 2: the marker names no test `nothing`".to_string()]);
}

#[test]
fn a_test_marker_may_name_several_tests_and_all_must_exist() {
    let text = "* both <!-- test: proves_it, proves_it -->\n* one missing <!-- test: proves_it, nothing -->\n";
    let judged = judge(text, false, &sample_known());
    assert!(judged.unproven.is_empty());
    assert_eq!(judged.broken, vec!["line 2: the marker names no test `nothing`".to_string()]);
}

#[test]
fn a_block_marker_needs_a_mote_block_after_it() {
    let text = "* shown <!-- block -->\n\n```mote\nlet x = 1\n```\n\n* alone <!-- block -->\n\nprose\n";
    let judged = judge(text, false, &sample_known());
    assert!(judged.unproven.is_empty());
    assert_eq!(judged.broken, vec!["line 7: the marker is not followed by a `mote` block".to_string()]);
}

#[test]
fn one_block_proves_the_run_of_marked_statements_before_it() {
    let text = "* one <!-- block -->\n* two <!-- block -->\n\n<!-- block -->\n| a | b |\n| c | d |\n\n```mote\nlet x = 1\n```\n\n* three <!-- block -->\n* plain\n\n```mote\nlet y = 2\n```\n";
    let judged = judge(text, false, &sample_known());
    assert_eq!(judged.unproven.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), vec!["plain"]);
    assert_eq!(judged.broken, vec!["line 12: the marker is not followed by a `mote` block".to_string()]);
}

#[test]
fn a_grammar_rule_is_proven_by_the_grammar_tests() {
    let text = "- `Let` ↔ `let`\n- `Block` ↔ `{` `}`\n- a plain bullet\n";
    let judged = judge(text, false, &sample_known());
    assert_eq!(judged.unproven.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), vec!["a plain bullet"]);
    assert_eq!(judged.broken.len(), 2);
    let known = Known { tests: GRAMMAR_TESTS.iter().map(|name| name.to_string()).collect(), slices: BTreeSet::new() };
    let judged = judge(text, false, &known);
    assert!(judged.broken.is_empty());
    assert_eq!(judged.unproven.len(), 1);
}

#[test]
fn the_grammar_tests_exist() {
    let known = known();
    assert!(GRAMMAR_TESTS.iter().all(|name| known.tests.contains(*name)));
}

#[test]
fn a_slice_row_proves_a_core_section_when_the_slice_exists() {
    let text = "## a\n\n| Provides | x |\n|---|---|\n| Slice | S5 |\n\nDone when: `x` passes.\n\n## b\n\n| Slice | S6 |\n\nDone when: `y` passes.\n";
    let judged = judge(text, true, &sample_known());
    let left: Vec<&str> = judged.unproven.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(left, vec!["| Slice | S6 |", "Done when: `y` passes."]);
    assert_eq!(judge(text, false, &sample_known()).unproven.len(), 5);
}

#[test]
fn a_slices_row_proves_a_core_section_when_every_slice_exists() {
    let text = "## a\n\n| Slices | S5 (native), S5 (API) |\n\n## b\n\n| Slices | S5, S9 |\n";
    let judged = judge(text, true, &sample_known());
    assert_eq!(judged.unproven.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), vec!["| Slices | S5, S9 |"]);
}

#[test]
fn the_list_must_match_the_unproven_statements() {
    let text = "* one\n* two\n";
    let judged = judge(text, false, &sample_known());
    let listed = render("x.md", &judged.unproven);
    assert!(compare("x.md", &judged.unproven, &listed).is_empty());
    let stale = format!("{listed}00000000 | gone\n");
    assert_eq!(compare("x.md", &judged.unproven, &stale), vec!["x.md: remove the line 00000000: its statement is proven, changed or gone".to_string()]);
    let short = listed.lines().take(2).collect::<Vec<_>>().join("\n");
    let problems = compare("x.md", &judged.unproven, &short);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("x.md:2: no proof form"), "{problems:?}");
}

#[test]
fn the_documents_are_found_without_the_plan_and_the_fixtures() {
    let labels: Vec<String> = documents().iter().map(|path| relative(path)).collect();
    assert!(labels.iter().any(|label| label == "core/CORE.md"));
    assert!(labels.iter().all(|label| label != "tasks.md" && !label.starts_with("tools/fixtures/")));
}

/// A document has no unproven statement and no list.
fn assert_strict(label: &str) {
    let text = std::fs::read_to_string(root().join(label)).expect("a readable document");
    let judged = judge(&text, label == "core/CORE.md", &known());
    let mut problems: Vec<String> = judged.broken.iter().map(|problem| format!("{label}: {problem}")).collect();
    problems.extend(judged.unproven.iter().map(|statement| format!("{label}:{}: no proof form: {}", statement.line, entry(statement).1)));
    if list_path(label).exists() {
        problems.push(format!("{label}: delete the list {}", relative(&list_path(label))));
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

macro_rules! strict {
    ($($name:ident: $label:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                assert_strict($label);
            }
        )*
    };
}

strict! {
    strict_lexical: "docs/reference/lexical.md",
    strict_types: "docs/reference/types.md",
    strict_numbers: "docs/reference/numbers.md",
    strict_bindings: "docs/reference/bindings.md",
    strict_operators: "docs/reference/operators.md",
    strict_control_flow: "docs/reference/control-flow.md",
    strict_patterns: "docs/reference/patterns.md",
    strict_functions: "docs/reference/functions.md",
    strict_structs_classes: "docs/reference/structs-classes.md",
    strict_enums: "docs/reference/enums.md",
    strict_generics_traits: "docs/reference/generics-traits.md",
    strict_optionals_errors: "docs/reference/optionals-errors.md",
    strict_collections: "docs/reference/collections.md",
    strict_modules: "docs/reference/modules.md",
    strict_concurrency: "docs/reference/concurrency.md",
    strict_attributes_tests: "docs/reference/attributes-tests.md",
    strict_derive: "docs/reference/derive.md",
    strict_contract: "docs/reference/contract.md",
    strict_command: "docs/reference/command.md",
    strict_reference_readme: "docs/reference/README.md",
    strict_syntax: "syntax/Syntax.md",
    strict_abi: "abi/Abi.md",
}

#[test]
fn strict_abi_has_no_undecided_section() {
    let text = std::fs::read_to_string(root().join("abi/Abi.md")).expect("the ABI document");
    assert!(!text.lines().any(|line| line.starts_with("## Not decided")));
}

/// The file names of the pages of `docs/reference` other than its README.
fn reference_pages() -> Vec<String> {
    let mut pages: Vec<String> = std::fs::read_dir(root().join("docs/reference"))
        .expect("docs/reference")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().into_owned();
            (name.ends_with(".md") && name != "README.md").then_some(name)
        })
        .collect();
    pages.sort();
    pages
}

#[test]
fn reference_readme_lists_the_pages_on_disk() {
    let readme = std::fs::read_to_string(root().join("docs/reference/README.md")).expect("the reference README");
    let mut listed: Vec<String> = readme.lines().filter_map(|line| line.strip_prefix("| [")).filter_map(|rest| rest.split_once("](")).map(|(name, _)| name.to_string()).collect();
    listed.sort();
    assert_eq!(listed, reference_pages());
}

#[test]
fn strict_reference() {
    for page in reference_pages() {
        assert_strict(&format!("docs/reference/{page}"));
    }
    assert_strict("docs/reference/README.md");
    assert_strict("syntax/Syntax.md");
}
