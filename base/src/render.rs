use crate::diagnostic::{Diagnostic, Severity};
use crate::source::SourceMap;
use crate::span::Span;

struct Block<'a> {
    location: String,
    line: u32,
    text: String,
    offset: usize,
    width: usize,
    mark: char,
    message: Option<&'a str>,
}

/// Prints a diagnostic with its source lines.
pub fn render(map: &SourceMap, diagnostic: &Diagnostic) -> String {
    let primary = std::iter::once((diagnostic.span, '^', None));
    let labels = diagnostic.labels.iter().map(|label| (label.span, '-', Some(label.message.as_str())));
    let blocks: Vec<Block> = primary.chain(labels).filter_map(|(span, mark, message)| block(map, span, mark, message)).collect();

    let gutter = blocks.iter().map(|b| b.line.to_string().len()).max().unwrap_or(1);
    let pad = " ".repeat(gutter);
    let severity = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };

    let mut out = format!("{severity}: {}\n", diagnostic.message);
    for b in &blocks {
        let marks = b.mark.to_string().repeat(b.width);
        let message = b.message.map(|m| format!(" {m}")).unwrap_or_default();
        out.push_str(&format!("{pad}--> {}\n{pad} |\n{:>gutter$} | {}\n", b.location, b.line, b.text));
        out.push_str(&format!("{pad} | {}{marks}{message}\n", " ".repeat(b.offset)));
    }
    if let Some(note) = &diagnostic.note {
        out.push_str(&format!("{pad} = note: {note}\n"));
    }
    let mut at = diagnostic.span.start;
    while let Some(origin) = map.file_at(at).and_then(|id| map.file(id).origin) {
        if let Some(b) = block(map, origin, '^', None) {
            out.push_str(&format!("{pad} = expanded from {}\n", b.location));
        }
        at = origin.start;
    }
    out
}

fn block<'a>(map: &SourceMap, span: Span, mark: char, message: Option<&'a str>) -> Option<Block<'a>> {
    let at = map.lookup(span.start)?;
    let file = map.file(at.file);
    let text = file.line_text(at.line)?;
    let covered = map.snippet(span).and_then(|s| s.lines().next()).map_or(0, |line| line.chars().count());
    Some(Block {
        location: format!("{}:{}:{}", file.name, at.line, at.column),
        line: at.line,
        // A tab would shift the marks; one space per character keeps them aligned.
        text: text.replace('\t', " "),
        offset: at.column as usize - 1,
        width: covered.max(1),
        mark,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_the_span_under_its_line() {
        let mut map = SourceMap::new();
        map.add_file("main.mote", "fn main() {\n    let x = oops;\n}\n");
        let d = Diagnostic::error("unknown name", Span::new(24, 28));
        let expected = "error: unknown name\n --> main.mote:2:13\n  |\n2 |     let x = oops;\n  |             ^^^^\n";
        assert_eq!(render(&map, &d), expected);
    }

    #[test]
    fn prints_labels_and_the_note() {
        let mut map = SourceMap::new();
        map.add_file("main.mote", "let a = 1\nlet a = 2\n");
        let d = Diagnostic::error("a is defined twice", Span::new(14, 15)).label(Span::new(4, 5), "first defined here").note("names are unique");
        let expected = "\
error: a is defined twice
 --> main.mote:2:5
  |
2 | let a = 2
  |     ^
 --> main.mote:1:5
  |
1 | let a = 1
  |     - first defined here
  = note: names are unique
";
        assert_eq!(render(&map, &d), expected);
    }

    #[test]
    fn a_warning_says_warning() {
        let mut map = SourceMap::new();
        map.add_file("a", "x");
        let d = Diagnostic::warning("unused", Span::new(0, 1));
        assert!(render(&map, &d).starts_with("warning: unused\n"));
    }

    #[test]
    fn widens_the_gutter_for_long_line_numbers() {
        let mut map = SourceMap::new();
        map.add_file("a", "\n".repeat(9) + "x");
        let d = Diagnostic::error("bad", Span::new(9, 10));
        let expected = "error: bad\n  --> a:10:1\n   |\n10 | x\n   | ^\n";
        assert_eq!(render(&map, &d), expected);
    }

    #[test]
    fn a_tab_in_the_line_keeps_the_marks_aligned() {
        let mut map = SourceMap::new();
        map.add_file("a", "\tbad");
        let d = Diagnostic::error("bad", Span::new(1, 4));
        let expected = "error: bad\n --> a:1:2\n  |\n1 |  bad\n  |  ^^^\n";
        assert_eq!(render(&map, &d), expected);
    }

    #[test]
    fn a_multiline_span_marks_its_first_line() {
        let mut map = SourceMap::new();
        map.add_file("a", "ab\ncd");
        let d = Diagnostic::error("bad", Span::new(0, 5));
        let expected = "error: bad\n --> a:1:1\n  |\n1 | ab\n  | ^^\n";
        assert_eq!(render(&map, &d), expected);
    }

    #[test]
    fn a_span_outside_every_file_prints_only_the_headline() {
        let map = SourceMap::new();
        let d = Diagnostic::error("lost", Span::new(5, 6));
        assert_eq!(render(&map, &d), "error: lost\n");
    }

    #[test]
    fn generated_code_points_back_to_its_call_site() {
        let mut map = SourceMap::new();
        map.add_file("main.mote", "@derive struct A {}\n");
        map.add_expansion("<derive A>", "fn a() { bad }\n", Span::new(0, 7));
        let d = Diagnostic::error("unknown name", Span::new(30, 33));
        let expected = "error: unknown name\n --> <derive A>:1:10\n  |\n1 | fn a() { bad }\n  |          ^^^\n  = expanded from main.mote:1:1\n";
        assert_eq!(render(&map, &d), expected);
    }
}
