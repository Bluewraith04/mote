use mote_base::{Arena, Id, Span};

use crate::bytes::{BytesError, Reader, Writer};

/// Why a compiled program stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    IntegerOverflow,
    DivideByZero,
    ShiftOutOfRange,
    IndexOutOfRange,
    /// `!` on a `None` or an `Err`.
    UnwrapFailed,
    /// An external library function returned NULL where the type is not optional.
    NullPointer,
    /// `std` stopped the program with a message of its own, such as a missing map key.
    Raised,
    /// The heap limit was reached.
    MemoryLimit,
    /// A task used its whole stack.
    StackDepth,
    /// Every task waits and nothing can wake one.
    Deadlock,
}

impl FaultKind {
    /// The position of a kind is its number in a byte form; new kinds go at the end.
    pub const ALL: [FaultKind; 10] = [
        FaultKind::IntegerOverflow,
        FaultKind::DivideByZero,
        FaultKind::ShiftOutOfRange,
        FaultKind::IndexOutOfRange,
        FaultKind::UnwrapFailed,
        FaultKind::NullPointer,
        FaultKind::Raised,
        FaultKind::MemoryLimit,
        FaultKind::StackDepth,
        FaultKind::Deadlock,
    ];

    /// What the stop says; a raised fault says the message it was given instead.
    pub fn message(self) -> &'static str {
        match self {
            FaultKind::IntegerOverflow => "integer overflow",
            FaultKind::DivideByZero => "division by zero",
            FaultKind::ShiftOutOfRange => "shift out of range",
            FaultKind::IndexOutOfRange => "index out of range",
            FaultKind::UnwrapFailed => "unwrapped a missing value",
            FaultKind::NullPointer => "external library returned a null pointer",
            FaultKind::Raised => "the message the program gives",
            FaultKind::MemoryLimit => "memory limit reached",
            FaultKind::StackDepth => "call stack too deep",
            FaultKind::Deadlock => "deadlock: every task waits",
        }
    }

    pub fn tag(self) -> u8 {
        FaultKind::ALL.iter().position(|kind| *kind == self).expect("every kind is listed") as u8
    }

    pub fn from_tag(tag: u8) -> Option<FaultKind> {
        FaultKind::ALL.get(usize::from(tag)).copied()
    }
}

/// One check in the program: what it tests and where it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultInfo {
    pub kind: FaultKind,
    pub span: Span,
}

/// The id compiled code passes to `Entry::Fault`.
pub type FaultSite = Id<FaultInfo>;

/// Every fault site of a program.
#[derive(Default)]
pub struct FaultTable {
    sites: Arena<FaultInfo>,
}

impl FaultTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, kind: FaultKind, span: Span) -> FaultSite {
        self.sites.alloc(FaultInfo { kind, span })
    }

    pub fn get(&self, site: FaultSite) -> &FaultInfo {
        &self.sites[site]
    }

    /// Finds a site from the raw id the runtime receives.
    pub fn get_raw(&self, raw: u32) -> Option<&FaultInfo> {
        self.sites.as_slice().get(raw as usize)
    }

    pub fn len(&self) -> usize {
        self.sites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// The magic, the version, the number of sites, then each site as its kind number and the two ends of its span.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.raw(MAGIC);
        writer.u32(VERSION);
        writer.count(self.len());
        for info in self.sites.as_slice() {
            writer.u8(info.kind.tag());
            writer.u32(info.span.start);
            writer.u32(info.span.end);
        }
        writer.finish()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BytesError> {
        let mut reader = Reader::new(bytes);
        reader.expect(MAGIC)?;
        match reader.u32()? {
            VERSION => {}
            other => return Err(BytesError::Version(other)),
        }
        let mut table = FaultTable::new();
        for _ in 0..reader.count(9)? {
            let tag = reader.u8()?;
            let kind = FaultKind::from_tag(tag).ok_or(BytesError::Tag(tag))?;
            let (start, end) = (reader.u32()?, reader.u32()?);
            if start > end {
                return Err(BytesError::Invalid("a span starts after it ends"));
            }
            table.add(kind, Span::new(start, end));
        }
        reader.end()?;
        Ok(table)
    }
}

/// The first bytes of a fault table.
pub const MAGIC: &[u8; 8] = b"MOTEFAUL";

/// The version of the byte form.
pub const VERSION: u32 = 1;

/// The raw id of a site, as passed to the runtime.
pub fn raw(site: FaultSite) -> u32 {
    site.index() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_each_kind_once_with_its_own_message() {
        let messages: std::collections::HashSet<&str> = FaultKind::ALL.iter().map(|kind| kind.message()).collect();
        assert_eq!(messages.len(), FaultKind::ALL.len());
        let names: std::collections::HashSet<String> = FaultKind::ALL.iter().map(|kind| format!("{kind:?}")).collect();
        assert_eq!(names.len(), FaultKind::ALL.len());
    }

    #[test]
    fn a_raw_id_finds_its_site() {
        let mut table = FaultTable::new();
        let first = table.add(FaultKind::DivideByZero, Span::new(3, 8));
        let second = table.add(FaultKind::IndexOutOfRange, Span::new(10, 14));
        assert_eq!(raw(first), 0);
        assert_eq!(table.get_raw(raw(second)).unwrap().kind, FaultKind::IndexOutOfRange);
        assert_eq!(table.get(first).span, Span::new(3, 8));
        assert!(table.get_raw(2).is_none());
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn every_kind_has_a_message() {
        assert!(FaultKind::ALL.iter().all(|kind| !kind.message().is_empty()));
    }

    fn sample() -> FaultTable {
        let mut table = FaultTable::new();
        table.add(FaultKind::DivideByZero, Span::new(3, 8));
        table.add(FaultKind::Raised, Span::new(10, 10));
        table.add(FaultKind::Deadlock, Span::new(0, u32::MAX));
        table
    }

    fn same(left: &FaultTable, right: &FaultTable) -> bool {
        left.len() == right.len() && (0..left.len() as u32).all(|raw| left.get_raw(raw) == right.get_raw(raw))
    }

    #[test]
    fn faults_a_kind_is_numbered_by_its_place_and_new_kinds_go_last() {
        for (index, kind) in FaultKind::ALL.iter().enumerate() {
            assert_eq!(kind.tag() as usize, index);
            assert_eq!(FaultKind::from_tag(kind.tag()), Some(*kind));
        }
        assert_eq!(FaultKind::from_tag(FaultKind::ALL.len() as u8), None);
        assert_eq!(FaultKind::IntegerOverflow.tag(), 0);
        assert_eq!(FaultKind::NullPointer.tag(), 5);
    }

    #[test]
    fn faults_a_table_is_the_magic_the_version_the_count_and_a_nine_byte_record_per_site() {
        let bytes = sample().to_bytes();
        assert_eq!(&bytes[..8], b"MOTEFAUL");
        assert_eq!(bytes[8..12], [1, 0, 0, 0]);
        assert_eq!(bytes[12..16], [3, 0, 0, 0]);
        assert_eq!(bytes.len(), 16 + 3 * 9);
        assert_eq!(bytes[16..25], [1, 3, 0, 0, 0, 8, 0, 0, 0]);
    }

    #[test]
    fn faults_a_table_reads_back_equal_and_gives_one_byte_string() {
        let bytes = sample().to_bytes();
        let read = FaultTable::from_bytes(&bytes).unwrap();
        assert!(same(&read, &sample()));
        assert_eq!(read.to_bytes(), bytes);
        assert!(FaultTable::from_bytes(&FaultTable::new().to_bytes()).unwrap().is_empty());
    }

    #[test]
    fn faults_a_bad_table_is_refused() {
        let bytes = sample().to_bytes();
        for cut in 0..bytes.len() {
            assert!(FaultTable::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut wrong = bytes.clone();
        wrong[0] = b'X';
        assert_eq!(FaultTable::from_bytes(&wrong).err(), Some(BytesError::Magic));
        let mut unknown = bytes.clone();
        unknown[16] = 99;
        assert_eq!(FaultTable::from_bytes(&unknown).err(), Some(BytesError::Tag(99)));
        let mut backwards = bytes.clone();
        backwards[17] = 9;
        assert_eq!(FaultTable::from_bytes(&backwards).err(), Some(BytesError::Invalid("a span starts after it ends")));
        let mut long = bytes;
        long.push(0);
        assert_eq!(FaultTable::from_bytes(&long).err(), Some(BytesError::Trailing));
    }

    #[test]
    fn faults_the_kinds_a_compiled_program_raises_are_these() {
        let names: Vec<String> = FaultKind::ALL.iter().map(|kind| format!("{kind:?}")).collect();
        assert_eq!(
            names,
            ["IntegerOverflow", "DivideByZero", "ShiftOutOfRange", "IndexOutOfRange", "UnwrapFailed", "NullPointer", "Raised", "MemoryLimit", "StackDepth", "Deadlock"]
        );
    }
}
