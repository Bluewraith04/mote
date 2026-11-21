use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A message attached to a second place in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// One problem found in the source, with where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub labels: Vec<Label>,
    pub note: Option<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self::new(Severity::Error, message, span)
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self::new(Severity::Warning, message, span)
    }

    pub fn label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label { span, message: message.into() });
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    fn new(severity: Severity, message: impl Into<String>, span: Span) -> Self {
        Self { severity, message: message.into(), span, labels: Vec::new(), note: None }
    }
}

/// Collects diagnostics so a stage can report more than one.
#[derive(Debug, Default)]
pub struct DiagnosticSink {
    items: Vec<Diagnostic>,
}

impl DiagnosticSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn emit(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_fill_the_fields() {
        let d = Diagnostic::error("bad", Span::new(1, 2)).label(Span::new(5, 6), "here").note("why");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "bad");
        assert_eq!(d.span, Span::new(1, 2));
        assert_eq!(d.labels, [Label { span: Span::new(5, 6), message: "here".into() }]);
        assert_eq!(d.note.as_deref(), Some("why"));
    }

    #[test]
    fn a_warning_is_not_an_error() {
        let mut sink = DiagnosticSink::new();
        sink.emit(Diagnostic::warning("odd", Span::new(0, 1)));
        assert!(!sink.has_errors());
        sink.emit(Diagnostic::error("bad", Span::new(0, 1)));
        assert!(sink.has_errors());
        assert_eq!(sink.len(), 2);
    }

    #[test]
    fn the_sink_keeps_emission_order() {
        let mut sink = DiagnosticSink::new();
        sink.emit(Diagnostic::error("first", Span::new(0, 1)));
        sink.emit(Diagnostic::error("second", Span::new(0, 1)));
        let messages: Vec<_> = sink.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages, ["first", "second"]);
    }
}
