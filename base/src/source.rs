use crate::arena::{Arena, Id};
use crate::span::Span;

const TOO_LARGE: &str = "source text exceeds the u32 offset space";

/// One source text and where it sits in the offset space.
pub struct SourceFile {
    pub name: String,
    pub text: String,
    pub start: u32,
    pub origin: Option<Span>,
    line_starts: Vec<u32>,
}

impl SourceFile {
    pub fn span(&self) -> Span {
        Span::new(self.start, self.start + self.text.len() as u32)
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The text of a 1-based line without its line ending.
    pub fn line_text(&self, line: u32) -> Option<&str> {
        let index = (line as usize).checked_sub(1)?;
        let start = *self.line_starts.get(index)? as usize;
        let end = self.line_starts.get(index + 1).map_or(self.text.len(), |&next| next as usize);
        Some(self.text[start..end].trim_end_matches(['\n', '\r']))
    }
}

/// A position as a file with a 1-based line and column counted in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: Id<SourceFile>,
    pub line: u32,
    pub column: u32,
}

/// Owns every source text and maps offsets back to files, lines and columns.
#[derive(Default)]
pub struct SourceMap {
    files: Arena<SourceFile>,
    next_start: u32,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, name: impl Into<String>, text: impl Into<String>) -> Id<SourceFile> {
        self.add(name.into(), text.into(), None)
    }

    /// Adds generated text; `call_site` is the span that produced it.
    pub fn add_expansion(&mut self, name: impl Into<String>, text: impl Into<String>, call_site: Span) -> Id<SourceFile> {
        self.add(name.into(), text.into(), Some(call_site))
    }

    pub fn file(&self, id: Id<SourceFile>) -> &SourceFile {
        &self.files[id]
    }

    pub fn file_at(&self, offset: u32) -> Option<Id<SourceFile>> {
        let files = self.files.as_slice();
        let index = files.partition_point(|file| file.start <= offset).checked_sub(1)?;
        (offset <= files[index].span().end).then(|| Id::from_index(index as u32))
    }

    pub fn lookup(&self, offset: u32) -> Option<Location> {
        let id = self.file_at(offset)?;
        let file = &self.files[id];
        let relative = offset - file.start;
        let line = file.line_starts.partition_point(|&start| start <= relative);
        let line_start = file.line_starts[line - 1] as usize;
        let before = &file.text.as_bytes()[line_start..relative as usize];
        // Every byte that is not a UTF-8 continuation byte starts a character.
        let column = before.iter().filter(|&&byte| (byte & 0xC0) != 0x80).count() as u32 + 1;
        Some(Location { file: id, line: line as u32, column })
    }

    /// The text a span covers, if it lies inside one file.
    pub fn snippet(&self, span: Span) -> Option<&str> {
        let file = &self.files[self.file_at(span.start)?];
        if span.end > file.span().end {
            return None;
        }
        file.text.get((span.start - file.start) as usize..(span.end - file.start) as usize)
    }

    fn add(&mut self, name: String, text: String, origin: Option<Span>) -> Id<SourceFile> {
        let start = self.next_start;
        let len = u32::try_from(text.len()).expect(TOO_LARGE);
        let end = start.checked_add(len).expect(TOO_LARGE);
        self.next_start = end.checked_add(1).expect(TOO_LARGE);
        let newlines = text.bytes().enumerate().filter(|&(_, byte)| byte == b'\n').map(|(index, _)| index as u32 + 1);
        let line_starts = std::iter::once(0).chain(newlines).collect();
        self.files.alloc(SourceFile { name, text, start, origin, line_starts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_get_consecutive_ranges() {
        let mut map = SourceMap::new();
        let a = map.add_file("a", "ab");
        let b = map.add_file("b", "cde");
        assert_eq!(map.file(a).span(), Span::new(0, 2));
        assert_eq!(map.file(b).span(), Span::new(3, 6));
    }

    #[test]
    fn file_at_finds_the_owner_including_the_end_offset() {
        let mut map = SourceMap::new();
        let a = map.add_file("a", "ab");
        let b = map.add_file("b", "cde");
        assert_eq!(map.file_at(0), Some(a));
        assert_eq!(map.file_at(2), Some(a));
        assert_eq!(map.file_at(3), Some(b));
        assert_eq!(map.file_at(6), Some(b));
        assert_eq!(map.file_at(7), None);
    }

    #[test]
    fn an_empty_file_still_owns_its_offset() {
        let mut map = SourceMap::new();
        let empty = map.add_file("empty", "");
        let next = map.add_file("next", "x");
        assert_eq!(map.file_at(0), Some(empty));
        assert_eq!(map.file_at(1), Some(next));
    }

    #[test]
    fn lookup_gives_line_and_column() {
        let mut map = SourceMap::new();
        let id = map.add_file("a", "one\ntwo\nthree");
        assert_eq!(map.lookup(0), Some(Location { file: id, line: 1, column: 1 }));
        assert_eq!(map.lookup(3), Some(Location { file: id, line: 1, column: 4 }));
        assert_eq!(map.lookup(4), Some(Location { file: id, line: 2, column: 1 }));
        assert_eq!(map.lookup(10), Some(Location { file: id, line: 3, column: 3 }));
    }

    #[test]
    fn lookup_is_relative_to_the_file() {
        let mut map = SourceMap::new();
        map.add_file("a", "x\ny\n");
        let b = map.add_file("b", "p\nq");
        let start = map.file(b).start;
        assert_eq!(map.lookup(start + 2), Some(Location { file: b, line: 2, column: 1 }));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let mut map = SourceMap::new();
        let id = map.add_file("a", "é=1");
        assert_eq!(map.lookup(2), Some(Location { file: id, line: 1, column: 2 }));
    }

    #[test]
    fn line_text_drops_line_endings() {
        let mut map = SourceMap::new();
        let id = map.add_file("a", "one\r\ntwo\n");
        let file = map.file(id);
        assert_eq!(file.line_text(1), Some("one"));
        assert_eq!(file.line_text(2), Some("two"));
        assert_eq!(file.line_text(3), Some(""));
        assert_eq!(file.line_text(4), None);
        assert_eq!(file.line_text(0), None);
        assert_eq!(file.line_count(), 3);
    }

    #[test]
    fn snippet_returns_the_covered_text() {
        let mut map = SourceMap::new();
        map.add_file("a", "let x");
        map.add_file("b", "fn y");
        assert_eq!(map.snippet(Span::new(4, 5)), Some("x"));
        assert_eq!(map.snippet(Span::new(6, 8)), Some("fn"));
        assert_eq!(map.snippet(Span::new(4, 8)), None);
    }

    #[test]
    fn an_expansion_records_its_call_site() {
        let mut map = SourceMap::new();
        map.add_file("main", "@derive");
        let call_site = Span::new(0, 7);
        let id = map.add_expansion("<derive>", "fn f() {}", call_site);
        assert_eq!(map.file(id).origin, Some(call_site));
    }
}
