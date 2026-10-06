//! Runtime faults that name the source they came from.

use isa::code::{SourceFile, SourceSpan};

use crate::{Runtime, TaskContext};

const MAX_CALLERS: usize = 10;

impl Runtime {
    pub(crate) fn locate_fault(&self, task: &TaskContext, message: String) -> String {
        let Some((file, span)) = self.source_at(task.current_code, task.pc) else { return message };
        let line_no = span.line as usize;
        let Some(line) = file.text.lines().nth(line_no.saturating_sub(1)) else { return message };
        let col = (span.col as usize).saturating_sub(1);
        let room = line.chars().count().saturating_sub(col).max(1);
        let carets = (span.len as usize).clamp(1, room);
        let width = line_no.to_string().len();
        let mut out = format!(
            "{message}\n{:>w$}--> {}:{}:{}\n{:>w$} |\n{line_no} | {line}\n{:>w$} | {}{}",
            "",
            file.path,
            span.line,
            span.col,
            "",
            "",
            " ".repeat(col),
            "^".repeat(carets),
            w = width,
        );
        let mut callers: Vec<(String, usize)> = Vec::new();
        for frame in task.call_stack.iter().skip(1).rev() {
            let Some((file, span)) = frame.return_pc.checked_sub(1).and_then(|pc| self.source_at(frame.caller_code, pc)) else {
                continue;
            };
            let at = format!("{}:{}:{}", file.path, span.line, span.col);
            match callers.last_mut() {
                Some((last, n)) if *last == at => *n += 1,
                _ => callers.push((at, 1)),
            }
        }
        for (caller, n) in callers.iter().take(MAX_CALLERS) {
            let times = if *n > 1 { format!(" ({n} times)") } else { String::new() };
            out.push_str(&format!("\n{:>width$} = called from {caller}{times}", ""));
        }
        if callers.len() > MAX_CALLERS {
            out.push_str(&format!("\n{:>width$} = … {} more caller(s)", "", callers.len() - MAX_CALLERS));
        }
        out
    }

    fn source_at(&self, code_idx: usize, pc: usize) -> Option<(&SourceFile, &SourceSpan)> {
        let span = self.code_objects.get(code_idx)?.span_at(pc)?;
        let file = self.source_files.iter().find(|f| f.id == span.source)?;
        Some((file, span))
    }
}
