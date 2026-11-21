//! The Rust backend of `markdown`: CommonMark text to the byte form of a `doc` tree.
#![allow(clippy::missing_safety_doc)]
mod doc_model;
use doc_model::{Doc, NONE};
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};

const FAILED: i64 = -1;

const TABLES: i64 = 1;
const STRIKETHROUGH: i64 = 2;
const TASKLISTS: i64 = 4;
const SMART_PUNCTUATION: i64 = 8;

#[rustfmt::skip]
const HREF_SAFE: [u8; 128] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 0, 1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 1,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 0,
];

/// A link target with the bytes a URL cannot hold percent-encoded; `&` and `'` stay, the HTML writer escapes them.
fn href(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for &byte in url.as_bytes() {
        let safe = byte < 0x80 && (HREF_SAFE[byte as usize] != 0 || byte == b'&' || byte == b'\'');
        if safe {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

struct Build {
    doc: Doc,
    open: Vec<u32>,
    pending: String,
    pending_raw: bool,
    alignments: Vec<Alignment>,
    cell: usize,
    head: bool,
    html: bool,
}

impl Build {
    fn new() -> Build {
        Build { doc: Doc::new(), open: Vec::new(), pending: String::new(), pending_raw: false, alignments: Vec::new(), cell: 0, head: false, html: false }
    }

    fn top(&self) -> u32 {
        self.open.last().copied().unwrap_or(0)
    }

    fn add(&mut self, text: &str, raw: bool) {
        if !self.pending.is_empty() && self.pending_raw != raw {
            self.flush();
        }
        self.pending_raw = raw;
        self.pending.push_str(text);
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let node = if self.pending_raw { self.doc.raw(self.pending.as_bytes()) } else { self.doc.text(self.pending.as_bytes()) };
        let top = self.top();
        self.doc.insert(top, node, NONE);
        self.pending.clear();
    }

    fn push(&mut self, name: &str) -> u32 {
        self.flush();
        let element = self.doc.element(name.as_bytes());
        let top = self.top();
        self.doc.insert(top, element, NONE);
        self.open.push(element);
        element
    }

    fn pop(&mut self) {
        self.flush();
        self.open.pop();
    }

    fn leaf(&mut self, name: &str) -> u32 {
        let element = self.push(name);
        self.pop();
        element
    }

    fn attr(&mut self, element: u32, name: &str, value: &str) {
        self.doc.set_attr(element, name.as_bytes(), value.as_bytes());
    }

    fn run(&mut self, events: &mut Parser) {
        while let Some(event) = events.next() {
            match event {
                Event::Start(tag) => self.start(tag, events),
                Event::End(tag) => self.end(tag),
                Event::Text(text) => {
                    let raw = self.html;
                    self.add(&text, raw);
                }
                Event::Code(text) => {
                    self.push("code");
                    self.add(&text, false);
                    self.pop();
                }
                Event::InlineMath(text) => self.math(&text, "math math-inline"),
                Event::DisplayMath(text) => self.math(&text, "math math-display"),
                Event::Html(html) | Event::InlineHtml(html) => self.add(&html, true),
                Event::SoftBreak => self.add("\n", false),
                Event::HardBreak => {
                    self.leaf("br");
                }
                Event::Rule => {
                    self.leaf("hr");
                }
                Event::FootnoteReference(name) => self.add(&format!("[^{name}]"), false),
                Event::TaskListMarker(checked) => {
                    let input = self.leaf("input");
                    self.attr(input, "disabled", "");
                    self.attr(input, "type", "checkbox");
                    if checked {
                        self.attr(input, "checked", "");
                    }
                }
            }
        }
        self.flush();
    }

    fn math(&mut self, text: &str, class: &str) {
        let span = self.push("span");
        self.attr(span, "class", class);
        self.add(text, false);
        self.pop();
    }

    fn start(&mut self, tag: Tag, events: &mut Parser) {
        match tag {
            Tag::HtmlBlock => self.html = true,
            Tag::Paragraph => {
                self.push("p");
            }
            Tag::Heading { level, id, classes, attrs } => {
                let heading = self.push(&level.to_string());
                if let Some(id) = id {
                    self.attr(heading, "id", &id);
                }
                if !classes.is_empty() {
                    let classes: Vec<&str> = classes.iter().map(|class| &**class).collect();
                    self.attr(heading, "class", &classes.join(" "));
                }
                for (name, value) in attrs {
                    self.attr(heading, &name, value.as_deref().unwrap_or(""));
                }
            }
            Tag::Table(alignments) => {
                self.alignments = alignments;
                self.push("table");
            }
            Tag::TableHead => {
                self.head = true;
                self.cell = 0;
                self.push("thead");
                self.push("tr");
            }
            Tag::TableRow => {
                self.cell = 0;
                self.push("tr");
            }
            Tag::TableCell => {
                let cell = self.push(if self.head { "th" } else { "td" });
                let align = match self.alignments.get(self.cell) {
                    Some(Alignment::Left) => "left",
                    Some(Alignment::Center) => "center",
                    Some(Alignment::Right) => "right",
                    _ => "",
                };
                if !align.is_empty() {
                    self.attr(cell, "style", &format!("text-align: {align}"));
                }
            }
            Tag::BlockQuote(kind) => {
                let quote = self.push("blockquote");
                let class = match kind {
                    None => "",
                    Some(BlockQuoteKind::Note) => "markdown-alert-note",
                    Some(BlockQuoteKind::Tip) => "markdown-alert-tip",
                    Some(BlockQuoteKind::Important) => "markdown-alert-important",
                    Some(BlockQuoteKind::Warning) => "markdown-alert-warning",
                    Some(BlockQuoteKind::Caution) => "markdown-alert-caution",
                };
                if !class.is_empty() {
                    self.attr(quote, "class", class);
                }
            }
            Tag::CodeBlock(kind) => {
                self.push("pre");
                let code = self.push("code");
                if let CodeBlockKind::Fenced(info) = kind {
                    let language = info.split(' ').next().unwrap_or("");
                    if !language.is_empty() {
                        self.attr(code, "class", &format!("language-{language}"));
                    }
                }
            }
            Tag::List(Some(1)) => {
                self.push("ol");
            }
            Tag::List(Some(first)) => {
                let list = self.push("ol");
                self.attr(list, "start", &first.to_string());
            }
            Tag::List(None) => {
                self.push("ul");
            }
            Tag::Item => {
                self.push("li");
            }
            Tag::DefinitionList => {
                self.push("dl");
            }
            Tag::DefinitionListTitle => {
                self.push("dt");
            }
            Tag::DefinitionListDefinition => {
                self.push("dd");
            }
            Tag::Subscript => {
                self.push("sub");
            }
            Tag::Superscript => {
                self.push("sup");
            }
            Tag::Emphasis => {
                self.push("em");
            }
            Tag::Strong => {
                self.push("strong");
            }
            Tag::Strikethrough => {
                self.push("del");
            }
            Tag::Link { link_type, dest_url, title, .. } => {
                let link = self.push("a");
                let target = href(&dest_url);
                if link_type == LinkType::Email {
                    self.attr(link, "href", &format!("mailto:{target}"));
                } else {
                    self.attr(link, "href", &target);
                }
                if !title.is_empty() {
                    self.attr(link, "title", &title);
                }
            }
            Tag::Image { dest_url, title, .. } => {
                let image = self.push("img");
                self.attr(image, "src", &href(&dest_url));
                let alt = alt_text(events);
                self.attr(image, "alt", &alt);
                if !title.is_empty() {
                    self.attr(image, "title", &title);
                }
                self.pop();
            }
            Tag::FootnoteDefinition(name) => {
                let note = self.push("div");
                self.attr(note, "class", "footnote-definition");
                self.attr(note, "id", &name);
            }
            Tag::MetadataBlock(_) => {
                for event in events.by_ref() {
                    if matches!(event, Event::End(TagEnd::MetadataBlock(_))) {
                        break;
                    }
                }
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::HtmlBlock => self.html = false,
            TagEnd::Image | TagEnd::MetadataBlock(_) => {}
            TagEnd::CodeBlock => {
                self.pop();
                self.pop();
            }
            TagEnd::Table => {
                self.pop();
                self.pop();
            }
            TagEnd::TableHead => {
                self.pop();
                self.pop();
                self.head = false;
                self.push("tbody");
            }
            TagEnd::TableCell => {
                self.pop();
                self.cell += 1;
            }
            _ => self.pop(),
        }
    }
}

/// The plain text of an image's description, up to the end of the image.
fn alt_text(events: &mut Parser) -> String {
    let mut alt = String::new();
    let mut depth = 0;
    for event in events.by_ref() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Event::Html(_) => {}
            Event::InlineHtml(text) | Event::Code(text) | Event::Text(text) => alt.push_str(&text),
            Event::InlineMath(text) => alt.push_str(&format!("${text}$")),
            Event::DisplayMath(text) => alt.push_str(&format!("$${text}$$")),
            Event::SoftBreak | Event::HardBreak | Event::Rule => alt.push(' '),
            Event::FootnoteReference(name) => alt.push_str(&format!("[^{name}]")),
            Event::TaskListMarker(checked) => alt.push_str(if checked { "[x]" } else { "[ ]" }),
        }
    }
    alt
}

/// Parses text into a document held by a handle. Returns the handle, or -1 for text that is not UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn markdown_parse(text: *const u8, length: i64, options: i64) -> i64 {
    let bytes = if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(text, length as usize) } };
    let Ok(text) = std::str::from_utf8(bytes) else { return FAILED };
    let mut flags = Options::empty();
    if options & TABLES != 0 {
        flags.insert(Options::ENABLE_TABLES);
    }
    if options & STRIKETHROUGH != 0 {
        flags.insert(Options::ENABLE_STRIKETHROUGH);
    }
    if options & TASKLISTS != 0 {
        flags.insert(Options::ENABLE_TASKLISTS);
    }
    if options & SMART_PUNCTUATION != 0 {
        flags.insert(Options::ENABLE_SMART_PUNCTUATION);
    }
    let mut build = Build::new();
    build.run(&mut Parser::new_ext(text, flags));
    Box::into_raw(Box::new(build.doc)) as i64
}

/// The byte form of a parsed document to `out`; returns its full length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn markdown_bytes(handle: i64, out: *mut u8, capacity: i64) -> i64 {
    let form = unsafe { &*(handle as *const Doc) }.to_bytes();
    if capacity > 0 {
        let count = form.len().min(capacity as usize);
        unsafe { std::ptr::copy_nonoverlapping(form.as_ptr(), out, count) };
    }
    form.len() as i64
}

/// Frees a parsed document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn markdown_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Doc) });
    }
    0
}
