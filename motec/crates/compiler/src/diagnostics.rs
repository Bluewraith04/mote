use crate::span::Span;

/// Renders an error with its source line.
pub struct DiagnosticFormatter;

impl DiagnosticFormatter {
    pub fn format_error(source: &str, file_name: &str, message: &str, span: &Span) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let line_idx = if span.line > 0 { span.line - 1 } else { 0 };
        let source_line = lines.get(line_idx).unwrap_or(&"");

        let col_idx = if span.col > 0 { span.col - 1 } else { 0 };
        let underline_len = if span.end > span.start && span.start < crate::span::DERIVED_BASE { (span.end - span.start).max(1) } else { 1 };
        let underline = format!("{}{}", " ".repeat(col_idx), "^".repeat(underline_len));

        format!(
            "error: {}\n --> {}:{}:{}\n  |\n{:>3} | {}\n  | {}\n",
            message, file_name, span.line, span.col, span.line, source_line, underline
        )
    }
}
