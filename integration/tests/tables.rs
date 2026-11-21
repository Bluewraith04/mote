//! Compares the tables of the documents that code produces with the code. `MOTE_BLESS=1` rewrites them.

mod common;

use mote::cli;
use mote_abi::{CType, Entry, FaultKind, HandleKind, Scalar, CONTEXT, CONTEXT_FIELDS};
use mote_ir::intrinsics::{self, effect};
use mote_semantics::{LangItem, ATTRIBUTES};
use mote_syntax::KEYWORDS;

use common::{TABLE_CLOSE as CLOSE, TABLE_OPEN as OPEN};

/// The tables with what each says now, in name order.
fn tables() -> Vec<(&'static str, String)> {
    vec![
        ("abi-context", abi_context()),
        ("abi-ctypes", abi_ctypes()),
        ("abi-entries", abi_entries()),
        ("abi-handles", abi_handles()),
        ("abi-scalars", abi_scalars()),
        ("attributes", attributes()),
        ("fault-kinds", fault_kinds()),
        ("intrinsics", intrinsic_rows()),
        ("keywords", keywords()),
        ("lang-items", lang_items()),
        ("std-modules", std_modules()),
        ("usage", usage()),
    ]
}

fn abi_scalars() -> String {
    let mut out = String::from("| Scalar | Bytes |\n|---|---|\n");
    for scalar in Scalar::ALL {
        out.push_str(&format!("| `{scalar:?}` | {} |\n", scalar.size()));
    }
    out
}

fn abi_context() -> String {
    let mut out = String::from("| `TaskContext` field | Offset |\n|---|---|\n");
    for (name, offset) in CONTEXT_FIELDS {
        out.push_str(&format!("| `{name}` | {offset} |\n"));
    }
    out
}

fn abi_handles() -> String {
    let mut out = String::from("| Handle | Words after the header | Payload types | Bytes before the payload |\n|---|---|---|---|\n");
    for kind in HandleKind::ALL {
        let words: Vec<String> = kind.words().iter().map(|(name, offset)| format!("`{name}`@{offset}")).collect();
        out.push_str(&format!("| `{kind:?}` | {} | {} | {} |\n", words.join(" "), kind.payloads(), kind.fixed_size()));
    }
    out
}

fn abi_entries() -> String {
    let scalars = |list: &[Scalar]| -> String {
        if list.is_empty() {
            return "none".to_string();
        }
        list.iter().map(|scalar| format!("`{scalar:?}`")).collect::<Vec<_>>().join(" ")
    };
    let mut out = String::from("| Entry | Symbol | Parameters after the context | Results |\n|---|---|---|---|\n");
    for entry in Entry::ALL {
        let (context, rest) = entry.params().split_first().expect("a context");
        assert_eq!(*context, CONTEXT);
        out.push_str(&format!("| `{entry:?}` | `{}` | {} | {} |\n", entry.name(), scalars(rest), scalars(entry.results())));
    }
    out
}

fn abi_ctypes() -> String {
    let mut out = String::from("| `CType` | Machine values | Direction |\n|---|---|---|\n");
    for ty in CType::ALL {
        let values: Vec<String> = ty.scalars().iter().map(|scalar| format!("`{scalar:?}`")).collect();
        let values = if values.is_empty() { "none".to_string() } else { values.join(" ") };
        let direction = match (ty.is_parameter(), ty.is_result()) {
            (true, true) => "both",
            (true, false) => "parameter",
            _ => "result",
        };
        out.push_str(&format!("| `{ty:?}` | {values} | {direction} |\n"));
    }
    out
}

fn std_modules() -> String {
    let mut names: Vec<String> = std::fs::read_dir(common::root().join("std/src"))
        .expect("the std source")
        .filter_map(|entry| entry.ok()?.path().file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .filter(|name| name != "lib" && name != "prelude")
        .map(|name| format!("`std.{name}`"))
        .collect();
    names.sort();
    format!("* The modules are {}.\n", names.join(" "))
}

fn attributes() -> String {
    let names: Vec<String> = ATTRIBUTES.iter().map(|name| format!("`@{name}`")).collect();
    let (last, first) = names.split_last().expect("an attribute");
    format!("* The attributes are {} and {last}.\n", first.join(", "))
}

fn fault_kinds() -> String {
    let mut out = String::from("| Kind | Message |\n|---|---|\n");
    for kind in FaultKind::ALL {
        out.push_str(&format!("| `{kind:?}` | {} |\n", kind.message()));
    }
    out
}

fn effects_text(effects: u8) -> String {
    let names: Vec<&str> = [(effect::READS, "reads"), (effect::WRITES, "writes"), (effect::FAULTS, "faults"), (effect::PARKS, "parks"), (effect::IO, "io")].into_iter().filter(|(flag, _)| effects & flag != 0).map(|(_, name)| name).collect();
    if names.is_empty() { "pure".to_string() } else { names.join(", ") }
}

fn intrinsic_rows() -> String {
    let mut out = String::from("| Intrinsic | Arguments | Effects |\n|---|---|---|\n");
    for intrinsic in intrinsics::TABLE {
        out.push_str(&format!("| `{}` | {} | {} |\n", intrinsic.name, intrinsic.params, effects_text(intrinsic.effects)));
    }
    out
}

fn keywords() -> String {
    let words: Vec<String> = KEYWORDS.iter().map(|(word, _)| format!("`{word}`")).collect();
    format!("* Reserved: {}.\n", words.join(" "))
}

fn lang_items() -> String {
    let mut out = String::from("| `@lang` name | Handle |\n|---|---|\n");
    for item in LangItem::ALL {
        out.push_str(&format!("| `{}` | {} |\n", item.name(), if item.is_handle() { "yes" } else { "no" }));
    }
    out
}

/// The text of `mote help` after the version line.
fn usage() -> String {
    let full = cli::usage();
    let (_, rest) = full.split_once("\n\n").expect("a version line, then the usage");
    format!("```text\n{rest}```\n")
}

/// A document with its tables brought up to date.
struct Reconciled {
    text: String,
    /// The names of the tables found, in order.
    found: Vec<String>,
    /// The tables whose text differed.
    stale: Vec<String>,
    /// Markers that are not in pairs, or name no table.
    broken: Vec<String>,
}

/// Replaces the text between each pair of markers in `text` with what `tables` says.
fn reconcile(label: &str, text: &str, tables: &[(&str, String)]) -> Reconciled {
    let mut out = Reconciled { text: String::new(), found: Vec::new(), stale: Vec::new(), broken: Vec::new() };
    let mut open: Option<(String, String)> = None;
    for (number, line) in text.split_inclusive('\n').enumerate() {
        let trimmed = line.trim_end();
        let at = number + 1;
        if let Some(name) = trimmed.strip_prefix(OPEN).and_then(|rest| rest.strip_suffix(" -->")) {
            if open.is_some() {
                out.broken.push(format!("{label}:{at}: a table opens inside a table"));
            }
            out.text.push_str(line);
            open = Some((name.to_string(), String::new()));
        } else if trimmed == CLOSE {
            let Some((name, old)) = open.take() else {
                out.broken.push(format!("{label}:{at}: a table closes and none is open"));
                out.text.push_str(line);
                continue;
            };
            match tables.iter().find(|(known, _)| *known == name) {
                Some((_, new)) => {
                    if *new != old {
                        out.stale.push(name.clone());
                    }
                    out.text.push_str(new);
                    out.found.push(name);
                }
                None => {
                    out.broken.push(format!("{label}:{at}: no table is called `{name}`"));
                    out.text.push_str(&old);
                }
            }
            out.text.push_str(line);
        } else if let Some((_, old)) = &mut open {
            old.push_str(line);
        } else {
            out.text.push_str(line);
        }
    }
    if let Some((name, old)) = open {
        out.broken.push(format!("{label}: the table `{name}` is never closed"));
        out.text.push_str(&old);
    }
    out
}

#[test]
fn tables_every_generated_table_in_the_documents_is_what_the_code_says() {
    let tables = tables();
    let bless = std::env::var("MOTE_BLESS").is_ok_and(|value| value == "1");
    let mut problems = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for path in common::files_with("md") {
        let text = std::fs::read_to_string(&path).expect("a readable file");
        if !text.contains(OPEN) {
            continue;
        }
        let label = common::relative(&path);
        let done = reconcile(&label, &text, &tables);
        problems.extend(done.broken);
        seen.extend(done.found);
        if bless && done.text != text {
            std::fs::write(&path, &done.text).expect("a writable file");
        } else {
            problems.extend(done.stale.iter().map(|name| format!("{label}: the table `{name}` is out of date; run with MOTE_BLESS=1")));
        }
    }
    for (name, _) in &tables {
        match seen.iter().filter(|found| found == name).count() {
            1 => {}
            count => problems.push(format!("the table `{name}` is in {count} places; it belongs in one")),
        }
    }
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn tables_the_keywords_are_the_words_the_lexer_reserves() {
    assert!(keywords().starts_with("* Reserved: `Self` `as` "));
    assert_eq!(KEYWORDS.len(), 33);
}

#[test]
fn tables_the_usage_text_lists_every_command_without_the_version() {
    let text = usage();
    assert!(text.starts_with("```text\nUsage: mote <command> [arguments]\n"), "{text}");
    for command in cli::COMMANDS {
        assert!(text.contains(&format!("  {}", command.name)), "{text}");
    }
    assert!(!text.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn tables_intrinsic_effects_are_named_in_a_fixed_order() {
    assert_eq!(effects_text(effect::PURE), "pure");
    assert_eq!(effects_text(effect::IO), "io");
    assert_eq!(effects_text(effect::PARKS | effect::READS | effect::FAULTS), "reads, faults, parks");
}

fn demo() -> Vec<(&'static str, String)> {
    vec![("a", "new\n".to_string()), ("b", "b text\n".to_string())]
}

#[test]
fn tables_reconcile_replaces_the_text_between_the_markers_and_nothing_else() {
    let text = "before\n<!-- table: a -->\nold\nlines\n<!-- /table -->\nafter\n";
    let done = reconcile("x.md", text, &demo());
    assert_eq!(done.text, "before\n<!-- table: a -->\nnew\n<!-- /table -->\nafter\n");
    assert_eq!((done.found, done.stale, done.broken), (vec!["a".to_string()], vec!["a".to_string()], Vec::<String>::new()));
}

#[test]
fn tables_reconcile_leaves_a_table_that_is_current_alone() {
    let text = "<!-- table: b -->\nb text\n<!-- /table -->\n";
    let done = reconcile("x.md", text, &demo());
    assert_eq!((done.text.as_str(), done.stale.len(), done.broken.len()), (text, 0, 0));
}

#[test]
fn tables_reconcile_reports_markers_that_are_not_in_pairs_or_name_no_table() {
    let unknown = reconcile("x.md", "<!-- table: z -->\nx\n<!-- /table -->\n", &demo());
    assert_eq!(unknown.broken, ["x.md:3: no table is called `z`"]);
    assert_eq!(unknown.text, "<!-- table: z -->\nx\n<!-- /table -->\n");
    let open = reconcile("x.md", "<!-- table: a -->\nx\n", &demo());
    assert_eq!(open.broken, ["x.md: the table `a` is never closed"]);
    let stray = reconcile("x.md", "text\n<!-- /table -->\n", &demo());
    assert_eq!(stray.broken, ["x.md:2: a table closes and none is open"]);
    let nested = reconcile("x.md", "<!-- table: a -->\n<!-- table: b -->\n<!-- /table -->\n", &demo());
    assert_eq!(nested.broken, ["x.md:2: a table opens inside a table"]);
}
