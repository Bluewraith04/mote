//! The subset of TOML a manifest uses: tables, strings, integers, booleans, arrays and inline tables.

/// One value, with the line its key starts on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<Value>),
    Table(Table),
}

impl Value {
    /// What the value is, for an error.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Value::Str(_) => "a string",
            Value::Int(_) => "an integer",
            Value::Bool(_) => "a boolean",
            Value::Array(_) => "an array",
            Value::Table(_) => "a table",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub key: String,
    pub line: u32,
    pub value: Value,
}

/// Entries in file order.
pub(crate) type Table = Vec<Entry>;

/// A line of the text that is not in the subset, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Syntax {
    pub line: u32,
    pub message: String,
}

struct Cursor {
    chars: Vec<char>,
    at: usize,
    line: u32,
}

impl Cursor {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let found = self.peek()?;
        self.at += 1;
        if found == '\n' {
            self.line += 1;
        }
        Some(found)
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T, Syntax> {
        Err(Syntax { line: self.line, message: message.into() })
    }

    fn spaces(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r')) {
            self.bump();
        }
    }

    fn comment(&mut self) {
        if self.peek() == Some('#') {
            while self.peek().is_some_and(|found| found != '\n') {
                self.bump();
            }
        }
    }

    /// Spaces, comments and line breaks.
    fn blank(&mut self) {
        loop {
            self.spaces();
            self.comment();
            if self.peek() == Some('\n') {
                self.bump();
            } else {
                return;
            }
        }
    }

    fn end_of_line(&mut self) -> Result<(), Syntax> {
        self.spaces();
        self.comment();
        match self.peek() {
            None | Some('\n') => Ok(()),
            Some(_) => self.fail("expected the end of the line"),
        }
    }

    fn key(&mut self) -> Result<String, Syntax> {
        match self.peek() {
            Some('"') => self.basic(),
            Some('\'') => self.literal(),
            Some(found) if bare(found) => {
                let mut key = String::new();
                while let Some(found) = self.peek().filter(|&found| bare(found)) {
                    key.push(found);
                    self.bump();
                }
                Ok(key)
            }
            _ => self.fail("expected a key"),
        }
    }

    fn basic(&mut self) -> Result<String, Syntax> {
        self.bump();
        let mut text = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => return self.fail("the string is not closed"),
                Some('"') => {
                    self.bump();
                    return Ok(text);
                }
                Some('\\') => {
                    self.bump();
                    match self.peek() {
                        Some('"') => text.push('"'),
                        Some('\\') => text.push('\\'),
                        Some('n') => text.push('\n'),
                        Some('t') => text.push('\t'),
                        Some('r') => text.push('\r'),
                        _ => return self.fail("unknown escape"),
                    }
                    self.bump();
                }
                Some(found) => {
                    text.push(found);
                    self.bump();
                }
            }
        }
    }

    fn literal(&mut self) -> Result<String, Syntax> {
        self.bump();
        let mut text = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => return self.fail("the string is not closed"),
                Some('\'') => {
                    self.bump();
                    return Ok(text);
                }
                Some(found) => {
                    text.push(found);
                    self.bump();
                }
            }
        }
    }

    fn value(&mut self) -> Result<Value, Syntax> {
        match self.peek() {
            Some('"') => self.basic().map(Value::Str),
            Some('\'') => self.literal().map(Value::Str),
            Some('[') => self.array(),
            Some('{') => self.inline(),
            Some(found) if found.is_ascii_digit() || found == '-' || found == '+' => self.int(),
            Some(found) if found.is_ascii_alphabetic() => {
                let mut word = String::new();
                while let Some(found) = self.peek().filter(|found| found.is_ascii_alphanumeric()) {
                    word.push(found);
                    self.bump();
                }
                match word.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    _ => self.fail(format!("`{word}` is not a value")),
                }
            }
            _ => self.fail("expected a value"),
        }
    }

    fn int(&mut self) -> Result<Value, Syntax> {
        let mut text = String::new();
        while let Some(found) = self.peek().filter(|found| found.is_ascii_alphanumeric() || matches!(found, '-' | '+' | '_' | '.' | ':')) {
            text.push(found);
            self.bump();
        }
        match text.replace('_', "").parse::<i64>() {
            Ok(number) => Ok(Value::Int(number)),
            Err(_) => self.fail(format!("`{text}` is not an integer")),
        }
    }

    fn array(&mut self) -> Result<Value, Syntax> {
        self.bump();
        let mut items = Vec::new();
        loop {
            self.blank();
            if self.peek() == Some(']') {
                self.bump();
                return Ok(Value::Array(items));
            }
            items.push(self.value()?);
            self.blank();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some(']') => {}
                _ => return self.fail("expected `,` or `]`"),
            }
        }
    }

    fn inline(&mut self) -> Result<Value, Syntax> {
        self.bump();
        let mut table = Table::new();
        self.spaces();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::Table(table));
        }
        loop {
            self.spaces();
            let line = self.line;
            let key = self.key()?;
            self.spaces();
            if self.peek() != Some('=') {
                return self.fail("expected `=`");
            }
            self.bump();
            self.spaces();
            let value = self.value()?;
            insert(&mut table, key, line, value)?;
            self.spaces();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some('}') => {
                    self.bump();
                    return Ok(Value::Table(table));
                }
                _ => return self.fail("expected `,` or `}`"),
            }
        }
    }

    fn header(&mut self) -> Result<String, Syntax> {
        self.bump();
        self.spaces();
        let name = self.key()?;
        self.spaces();
        match self.peek() {
            Some(']') => {
                self.bump();
                Ok(name)
            }
            Some('.') => self.fail("a dotted table name is not supported"),
            _ => self.fail("expected `]`"),
        }
    }
}

fn bare(found: char) -> bool {
    found.is_ascii_alphanumeric() || found == '_' || found == '-'
}

fn insert(table: &mut Table, key: String, line: u32, value: Value) -> Result<(), Syntax> {
    if table.iter().any(|entry| entry.key == key) {
        return Err(Syntax { line, message: format!("`{key}` is set twice") });
    }
    table.push(Entry { key, line, value });
    Ok(())
}

/// The root table of the text; a `[name]` header makes a table entry of the root.
pub(crate) fn parse(text: &str) -> Result<Table, Syntax> {
    let mut cursor = Cursor { chars: text.chars().collect(), at: 0, line: 1 };
    let mut root = Table::new();
    let mut current: Option<usize> = None;
    loop {
        cursor.blank();
        let Some(first) = cursor.peek() else { return Ok(root) };
        let line = cursor.line;
        if first == '[' {
            if cursor.peek_at(1) == Some('[') {
                return cursor.fail("an array of tables is not supported");
            }
            let name = cursor.header()?;
            insert(&mut root, name, line, Value::Table(Table::new()))?;
            current = Some(root.len() - 1);
        } else {
            let key = cursor.key()?;
            cursor.spaces();
            match cursor.peek() {
                Some('=') => {
                    cursor.bump();
                }
                Some('.') => return cursor.fail("a dotted key is not supported"),
                _ => return cursor.fail("expected `=`"),
            }
            cursor.spaces();
            let value = cursor.value()?;
            let table = match current {
                Some(at) => match &mut root[at].value {
                    Value::Table(table) => table,
                    _ => unreachable!("a header makes a table"),
                },
                None => &mut root,
            };
            insert(table, key, line, value)?;
        }
        cursor.end_of_line()?;
    }
}

impl Cursor {
    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(table: &Table) -> Vec<&str> {
        table.iter().map(|entry| entry.key.as_str()).collect()
    }

    fn syntax(text: &str) -> (u32, String) {
        let error = parse(text).expect_err("not in the subset");
        (error.line, error.message)
    }

    #[test]
    fn toml_reads_tables_strings_integers_and_booleans_with_the_line_of_each_key() {
        let root = parse("# top\nname = \"a\"\n\n[package]\nn = 7\nok = true\nlit = 'x\\y'\n").unwrap();
        assert_eq!(keys(&root), ["name", "package"]);
        assert_eq!((root[0].line, root[1].line), (2, 4));
        let Value::Table(package) = &root[1].value else { panic!("a table") };
        assert_eq!(package.iter().map(|entry| (entry.key.as_str(), entry.line)).collect::<Vec<_>>(), [("n", 5), ("ok", 6), ("lit", 7)]);
        assert_eq!(package[0].value, Value::Int(7));
        assert_eq!(package[1].value, Value::Bool(true));
        assert_eq!(package[2].value, Value::Str("x\\y".into()));
    }

    #[test]
    fn toml_reads_escapes_comments_and_quoted_keys() {
        let root = parse("\"a b\" = \"q\\\"\\\\\\n\\t\"  # trailing\n").unwrap();
        assert_eq!(root[0].key, "a b");
        assert_eq!(root[0].value, Value::Str("q\"\\\n\t".into()));
    }

    #[test]
    fn toml_reads_arrays_across_lines_and_inline_tables() {
        let root = parse("list = [\n  \"a\", # one\n  \"b\",\n]\npair = { path = \"../x\", n = 1 }\nnone = {}\nempty = []\n").unwrap();
        assert_eq!(root[0].value, Value::Array(vec![Value::Str("a".into()), Value::Str("b".into())]));
        let Value::Table(pair) = &root[1].value else { panic!("a table") };
        assert_eq!(keys(pair), ["path", "n"]);
        assert_eq!(root[2].value, Value::Table(Vec::new()));
        assert_eq!(root[3].value, Value::Array(Vec::new()));
    }

    #[test]
    fn toml_a_text_with_no_key_is_an_empty_table() {
        assert_eq!(parse("").unwrap(), Vec::new());
        assert_eq!(parse("\n# only a comment\n\n").unwrap(), Vec::new());
    }

    #[test]
    fn toml_reports_the_line_of_a_syntax_error() {
        assert_eq!(syntax("a = 1\nb = \n"), (2, "expected a value".into()));
        assert_eq!(syntax("a = 1\n\nb 2\n"), (3, "expected `=`".into()));
        assert_eq!(syntax("a = \"open\n"), (1, "the string is not closed".into()));
        assert_eq!(syntax("a = \"\\q\"\n"), (1, "unknown escape".into()));
        assert_eq!(syntax("a = 1 2\n"), (1, "expected the end of the line".into()));
        assert_eq!(syntax("a = [1 2]\n"), (1, "expected `,` or `]`".into()));
        assert_eq!(syntax("a = { b = 1\n"), (1, "expected `,` or `}`".into()));
        assert_eq!(syntax("[a\n"), (1, "expected `]`".into()));
        assert_eq!(syntax("a = nope\n"), (1, "`nope` is not a value".into()));
        assert_eq!(syntax("a = 1.5\n"), (1, "`1.5` is not an integer".into()));
    }

    #[test]
    fn toml_a_key_or_table_set_twice_is_an_error_at_the_second() {
        assert_eq!(syntax("a = 1\na = 2\n"), (2, "`a` is set twice".into()));
        assert_eq!(syntax("[t]\n[t]\n"), (2, "`t` is set twice".into()));
        assert_eq!(syntax("[t]\na = 1\na = 2\n"), (3, "`a` is set twice".into()));
        assert_eq!(syntax("a = { b = 1, b = 2 }\n"), (1, "`b` is set twice".into()));
    }

    #[test]
    fn toml_the_parts_of_toml_outside_the_subset_are_errors() {
        assert_eq!(syntax("[a.b]\n"), (1, "a dotted table name is not supported".into()));
        assert_eq!(syntax("a.b = 1\n"), (1, "a dotted key is not supported".into()));
        assert_eq!(syntax("[[a]]\n"), (1, "an array of tables is not supported".into()));
    }
}
