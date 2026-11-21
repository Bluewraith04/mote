//! A document as a tree of nodes in one arena, with interned strings and a byte form.
use std::collections::HashMap;

pub const NONE: u32 = u32::MAX;
pub const DOCUMENT: u8 = 0;
pub const ELEMENT: u8 = 1;
pub const TEXT: u8 = 2;
pub const COMMENT: u8 = 3;
pub const RAW: u8 = 4;

const MAGIC: &[u8; 4] = b"MDOC";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    kind: u8,
    name: u32,
    text: u32,
    parent: u32,
    children: Vec<u32>,
    attrs: Vec<(u32, u32)>,
}

/// The nodes of a document by id (0 is the document node) and the strings they share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    strings: Vec<Box<[u8]>>,
    index: HashMap<Box<[u8]>, u32>,
    nodes: Vec<Node>,
}

impl Default for Doc {
    fn default() -> Self {
        Self::new()
    }
}

impl Doc {
    pub fn new() -> Doc {
        let mut doc = Doc { strings: Vec::new(), index: HashMap::new(), nodes: Vec::new() };
        doc.intern(b"");
        doc.nodes.push(Node { kind: DOCUMENT, name: 0, text: 0, parent: NONE, children: Vec::new(), attrs: Vec::new() });
        doc
    }

    /// The id of a string, adding it on first use.
    pub fn intern(&mut self, bytes: &[u8]) -> u32 {
        if let Some(&found) = self.index.get(bytes) {
            return found;
        }
        let id = self.strings.len() as u32;
        let owned: Box<[u8]> = bytes.into();
        self.strings.push(owned.clone());
        self.index.insert(owned, id);
        id
    }

    pub fn string(&self, id: u32) -> &[u8] {
        &self.strings[id as usize]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn has(&self, id: u32) -> bool {
        (id as usize) < self.nodes.len()
    }

    fn add(&mut self, kind: u8, name: u32, text: u32) -> u32 {
        self.nodes.push(Node { kind, name, text, parent: NONE, children: Vec::new(), attrs: Vec::new() });
        (self.nodes.len() - 1) as u32
    }

    pub fn element(&mut self, name: &[u8]) -> u32 {
        let name = self.intern(name);
        self.add(ELEMENT, name, 0)
    }

    pub fn text(&mut self, text: &[u8]) -> u32 {
        let text = self.intern(text);
        self.add(TEXT, 0, text)
    }

    pub fn comment(&mut self, text: &[u8]) -> u32 {
        let text = self.intern(text);
        self.add(COMMENT, 0, text)
    }

    /// A node of HTML text that is written as it is.
    pub fn raw(&mut self, text: &[u8]) -> u32 {
        let text = self.intern(text);
        self.add(RAW, 0, text)
    }

    pub fn kind(&self, id: u32) -> u8 {
        self.nodes[id as usize].kind
    }

    pub fn name(&self, id: u32) -> &[u8] {
        self.string(self.nodes[id as usize].name)
    }

    pub fn text_of(&self, id: u32) -> &[u8] {
        self.string(self.nodes[id as usize].text)
    }

    pub fn parent(&self, id: u32) -> u32 {
        self.nodes[id as usize].parent
    }

    pub fn children(&self, id: u32) -> &[u32] {
        &self.nodes[id as usize].children
    }

    /// Whether `ancestor` is `id` or above it.
    fn is_within(&self, id: u32, ancestor: u32) -> bool {
        let mut at = id;
        while at != NONE {
            if at == ancestor {
                return true;
            }
            at = self.nodes[at as usize].parent;
        }
        false
    }

    /// Takes a node off its parent.
    pub fn detach(&mut self, id: u32) {
        let parent = self.nodes[id as usize].parent;
        if parent != NONE {
            self.nodes[parent as usize].children.retain(|&child| child != id);
            self.nodes[id as usize].parent = NONE;
        }
    }

    /// Puts `child` in `parent` before `before` (the end when `before` is `NONE`), moving it out of its old parent; false when that is not possible.
    pub fn insert(&mut self, parent: u32, child: u32, before: u32) -> bool {
        if !self.has(parent) || !self.has(child) || child == 0 || self.nodes[parent as usize].kind > ELEMENT {
            return false;
        }
        if self.is_within(parent, child) || (before != NONE && (!self.has(before) || self.nodes[before as usize].parent != parent || before == child)) {
            return false;
        }
        self.detach(child);
        let at = if before == NONE { self.nodes[parent as usize].children.len() } else { self.nodes[parent as usize].children.iter().position(|&found| found == before).unwrap_or(0) };
        self.nodes[parent as usize].children.insert(at, child);
        self.nodes[child as usize].parent = parent;
        true
    }

    pub fn set_text(&mut self, id: u32, text: &[u8]) -> bool {
        if self.nodes[id as usize].kind < TEXT {
            return false;
        }
        self.nodes[id as usize].text = self.intern(text);
        true
    }

    pub fn set_name(&mut self, id: u32, name: &[u8]) -> bool {
        if self.nodes[id as usize].kind != ELEMENT {
            return false;
        }
        self.nodes[id as usize].name = self.intern(name);
        true
    }

    pub fn set_attr(&mut self, id: u32, name: &[u8], value: &[u8]) -> bool {
        if self.nodes[id as usize].kind != ELEMENT {
            return false;
        }
        let (name, value) = (self.intern(name), self.intern(value));
        let attrs = &mut self.nodes[id as usize].attrs;
        match attrs.iter_mut().find(|(found, _)| *found == name) {
            Some(slot) => slot.1 = value,
            None => attrs.push((name, value)),
        }
        true
    }

    pub fn remove_attr(&mut self, id: u32, name: &[u8]) -> bool {
        let Some(&name) = self.index.get(name) else { return false };
        let attrs = &mut self.nodes[id as usize].attrs;
        let before = attrs.len();
        attrs.retain(|(found, _)| *found != name);
        attrs.len() != before
    }

    pub fn attr(&self, id: u32, name: &[u8]) -> Option<&[u8]> {
        let name = *self.index.get(name)?;
        self.nodes[id as usize].attrs.iter().find(|(found, _)| *found == name).map(|&(_, value)| self.string(value))
    }

    pub fn attr_count(&self, id: u32) -> usize {
        self.nodes[id as usize].attrs.len()
    }

    pub fn attr_at(&self, id: u32, at: usize) -> Option<(&[u8], &[u8])> {
        self.nodes[id as usize].attrs.get(at).map(|&(name, value)| (self.string(name), self.string(value)))
    }

    /// Copies the subtree at `id` of `other` into this document, unattached, and gives the id of its copy.
    pub fn import(&mut self, other: &Doc, id: u32) -> u32 {
        let node = &other.nodes[id as usize];
        let copy = match node.kind {
            ELEMENT => self.element(other.string(node.name)),
            TEXT => self.text(other.string(node.text)),
            COMMENT => self.comment(other.string(node.text)),
            RAW => self.raw(other.string(node.text)),
            _ => self.element(b"document"),
        };
        for &(name, value) in &node.attrs {
            self.set_attr(copy, other.string(name), other.string(value));
        }
        for &child in &node.children {
            let inner = self.import(other, child);
            self.insert(copy, inner, NONE);
        }
        copy
    }

    /// Copies the subtree at `id` inside this document, unattached, and gives the id of its copy.
    pub fn duplicate(&mut self, id: u32) -> u32 {
        let node = self.nodes[id as usize].clone();
        let copy = match node.kind {
            DOCUMENT => self.element(b"document"),
            kind => self.add(kind, node.name, node.text),
        };
        self.nodes[copy as usize].attrs = node.attrs;
        for child in node.children {
            let inner = self.duplicate(child);
            self.insert(copy, inner, NONE);
        }
        copy
    }

    /// The byte form: a header, the strings, then each node with its attributes and child ids.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        fn put(out: &mut Vec<u8>, value: u32) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        put(&mut out, VERSION);
        put(&mut out, self.strings.len() as u32);
        for string in &self.strings {
            put(&mut out, string.len() as u32);
            out.extend_from_slice(string);
        }
        put(&mut out, self.nodes.len() as u32);
        for node in &self.nodes {
            out.push(node.kind);
            put(&mut out, node.name);
            put(&mut out, node.text);
            put(&mut out, node.attrs.len() as u32);
            for &(name, value) in &node.attrs {
                put(&mut out, name);
                put(&mut out, value);
            }
            put(&mut out, node.children.len() as u32);
            for &child in &node.children {
                put(&mut out, child);
            }
        }
        out
    }

    /// Reads the byte form; nothing is built from bytes that break its rules.
    pub fn from_bytes(bytes: &[u8]) -> Result<Doc, &'static str> {
        let mut cursor = Cursor { bytes, at: 0 };
        if cursor.take(4)? != MAGIC {
            return Err("not a document");
        }
        if cursor.u32()? != VERSION {
            return Err("unknown version");
        }
        let mut doc = Doc { strings: Vec::new(), index: HashMap::new(), nodes: Vec::new() };
        let strings = cursor.u32()?;
        for expected in 0..strings {
            let length = cursor.u32()? as usize;
            let string: Box<[u8]> = cursor.take(length)?.into();
            if doc.index.insert(string.clone(), expected).is_some() {
                return Err("a string twice");
            }
            doc.strings.push(string);
        }
        if doc.strings.first().is_none_or(|first| !first.is_empty()) {
            return Err("no empty string first");
        }
        let count = cursor.u32()?;
        if count == 0 {
            return Err("no document node");
        }
        for _ in 0..count {
            let kind = cursor.take(1)?[0];
            let (name, text) = (cursor.u32()?, cursor.u32()?);
            if kind > RAW || name as usize >= doc.strings.len() || text as usize >= doc.strings.len() {
                return Err("a node out of range");
            }
            let attr_count = cursor.u32()?;
            let mut attrs = Vec::new();
            for _ in 0..attr_count {
                let (name, value) = (cursor.u32()?, cursor.u32()?);
                if name as usize >= doc.strings.len() || value as usize >= doc.strings.len() || attrs.iter().any(|(found, _)| *found == name) {
                    return Err("an attribute out of range");
                }
                attrs.push((name, value));
            }
            let child_count = cursor.u32()?;
            let mut children = Vec::new();
            for _ in 0..child_count {
                children.push(cursor.u32()?);
            }
            doc.nodes.push(Node { kind, name, text, parent: NONE, children, attrs });
        }
        if cursor.at != bytes.len() {
            return Err("bytes after the document");
        }
        if doc.nodes[0].kind != DOCUMENT || doc.nodes[1..].iter().any(|node| node.kind == DOCUMENT) {
            return Err("the document node is not first and alone");
        }
        for id in 0..doc.nodes.len() {
            let children = doc.nodes[id].children.clone();
            if !children.is_empty() && doc.nodes[id].kind > ELEMENT {
                return Err("a leaf with children");
            }
            for child in children {
                if child == 0 || child as usize >= doc.nodes.len() || doc.nodes[child as usize].parent != NONE {
                    return Err("a child out of range or with two parents");
                }
                doc.nodes[child as usize].parent = id as u32;
            }
        }
        for id in 0..doc.nodes.len() as u32 {
            let mut at = id;
            for _ in 0..=doc.nodes.len() {
                if at == NONE {
                    break;
                }
                at = doc.nodes[at as usize].parent;
            }
            if at != NONE {
                return Err("a cycle");
            }
        }
        Ok(doc)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], &'static str> {
        let end = self.at.checked_add(count).filter(|&end| end <= self.bytes.len()).ok_or("the bytes end early")?;
        let found = &self.bytes[self.at..end];
        self.at = end;
        Ok(found)
    }

    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Doc {
        let mut doc = Doc::new();
        let p = doc.element(b"p");
        doc.set_attr(p, b"class", b"lead");
        let hello = doc.text(b"hello");
        doc.insert(p, hello, NONE);
        doc.insert(0, p, NONE);
        doc
    }

    #[test]
    fn the_byte_form_reads_back_equal() {
        let doc = sample();
        let again = Doc::from_bytes(&doc.to_bytes()).unwrap();
        assert_eq!(doc, again);
        assert_eq!(again.to_bytes(), doc.to_bytes());
    }

    #[test]
    fn a_string_is_stored_once() {
        let mut doc = Doc::new();
        let a = doc.text(b"same");
        let b = doc.text(b"same");
        assert_eq!(doc.nodes[a as usize].text, doc.nodes[b as usize].text);
        assert_eq!(doc.strings.len(), 2);
    }

    #[test]
    fn a_node_moves_and_cannot_enter_itself() {
        let mut doc = sample();
        let p = doc.children(0)[0];
        let q = doc.element(b"q");
        assert!(doc.insert(0, q, NONE));
        assert!(doc.insert(q, p, NONE));
        assert_eq!(doc.children(0), [q]);
        assert!(!doc.insert(p, q, NONE));
        assert!(!doc.insert(p, p, NONE));
    }

    #[test]
    fn bytes_that_break_the_rules_are_refused() {
        let good = sample().to_bytes();
        assert!(Doc::from_bytes(&good[..good.len() - 1]).is_err());
        let mut extra = good.clone();
        extra.push(0);
        assert!(Doc::from_bytes(&extra).is_err());
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(Doc::from_bytes(&magic).is_err());
        assert!(Doc::from_bytes(b"").is_err());
    }

    #[test]
    fn a_cycle_or_a_second_parent_is_refused() {
        let mut doc = Doc::new();
        let a = doc.element(b"a");
        let b = doc.element(b"b");
        doc.insert(0, a, NONE);
        doc.insert(a, b, NONE);
        let mut twice = doc.clone();
        twice.nodes[0].children.push(b);
        assert!(Doc::from_bytes(&twice.to_bytes()).is_err());
        let mut cycle = doc.clone();
        cycle.nodes[b as usize].children.push(a);
        assert!(Doc::from_bytes(&cycle.to_bytes()).is_err());
    }

    #[test]
    fn a_subtree_is_imported_with_its_attributes() {
        let from = sample();
        let mut into = Doc::new();
        let copy = into.import(&from, from.children(0)[0]);
        assert_eq!(into.name(copy), b"p");
        assert_eq!(into.attr(copy, b"class"), Some(&b"lead"[..]));
        assert_eq!(into.text_of(into.children(copy)[0]), b"hello");
    }
}
