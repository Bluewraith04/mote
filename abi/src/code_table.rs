use crate::bytes::{BytesError, Reader, Writer};
use crate::frame::FrameDescriptor;
use crate::kind::Slot;

/// The first bytes of a code table.
pub const MAGIC: &[u8; 8] = b"MOTECODE";

/// The version of the byte form.
pub const VERSION: u32 = 1;

/// What the collector knows of an object file's code: the descriptor of each call site and the static places that hold references.
///
/// The addresses are not in the bytes: a function is placed by its start and a root by its address, in the order of the lists, in a separate array of words that the linker fills.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeTable {
    /// For each function, its call sites as the offset of the return address from the function's start, ascending, and the frame at the call.
    pub functions: Vec<Vec<(u32, FrameDescriptor)>>,
    /// For each static place that holds references, where the references are.
    pub roots: Vec<Vec<Slot>>,
}

impl CodeTable {
    /// The words of the address array: one for each function, then one for each root.
    pub fn words(&self) -> usize {
        self.functions.len() + self.roots.len()
    }

    /// The magic, the version, the functions as their sites (an offset, then the length and bytes of the frame), then the roots as their slots.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.raw(MAGIC);
        writer.u32(VERSION);
        writer.count(self.functions.len());
        for sites in &self.functions {
            writer.count(sites.len());
            for (offset, frame) in sites {
                writer.u32(*offset);
                let bytes = frame.to_bytes();
                writer.count(bytes.len());
                writer.raw(&bytes);
            }
        }
        writer.count(self.roots.len());
        for slots in &self.roots {
            writer.count(slots.len());
            slots.iter().for_each(|slot| writer.slot(*slot));
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
        let mut functions = Vec::new();
        for _ in 0..reader.count(4)? {
            let mut sites: Vec<(u32, FrameDescriptor)> = Vec::new();
            for _ in 0..reader.count(8)? {
                let offset = reader.u32()?;
                if sites.last().is_some_and(|(last, _)| *last >= offset) {
                    return Err(BytesError::Invalid("the sites of a function are not in order"));
                }
                let length = reader.count(1)?;
                sites.push((offset, FrameDescriptor::from_bytes(reader.raw(length)?)?));
            }
            functions.push(sites);
        }
        let mut roots = Vec::new();
        for _ in 0..reader.count(4)? {
            roots.push((0..reader.count(5)?).map(|_| reader.slot()).collect::<Result<Vec<_>, _>>()?);
        }
        reader.end()?;
        Ok(Self { functions, roots })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::TypeId;

    fn table() -> CodeTable {
        let small = FrameDescriptor::new(16, vec![8], vec![]).unwrap();
        let big = FrameDescriptor::new(64, vec![0, 24], vec![Slot::Pair(32), Slot::Enum { offset: 48, ty: TypeId::from_raw(5) }]).unwrap();
        CodeTable {
            functions: vec![vec![(12, small.clone()), (40, big)], vec![], vec![(7, small)]],
            roots: vec![vec![Slot::Ref(0)], vec![Slot::Repeat { offset: 8, count: 3, stride: 16, ty: TypeId::from_raw(6) }, Slot::Pair(48)]],
        }
    }

    #[test]
    fn code_a_table_is_the_magic_the_version_the_functions_then_the_roots() {
        let written = CodeTable { functions: vec![vec![(4, FrameDescriptor::new(16, vec![8], vec![]).unwrap())]], roots: vec![vec![Slot::Ref(8)]] }.to_bytes();
        let mut expected = b"MOTECODE".to_vec();
        expected.extend([1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 16, 0, 0, 0]);
        expected.extend([16, 0, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0]);
        expected.extend([1, 0, 0, 0, 1, 0, 0, 0, 0, 8, 0, 0, 0]);
        assert_eq!(written, expected);
    }

    #[test]
    fn code_a_table_reads_back_equal_and_gives_one_byte_string() {
        let bytes = table().to_bytes();
        let read = CodeTable::from_bytes(&bytes).unwrap();
        assert_eq!(read, table());
        assert_eq!(read.to_bytes(), bytes);
        assert_eq!(read.words(), 5);
    }

    #[test]
    fn code_nothing_is_the_header_and_two_zero_counts() {
        assert_eq!(CodeTable::default().to_bytes().len(), 8 + 4 + 4 + 4);
        assert_eq!(CodeTable::from_bytes(&CodeTable::default().to_bytes()), Ok(CodeTable::default()));
    }

    #[test]
    fn code_a_bad_table_is_refused() {
        let bytes = table().to_bytes();
        assert_eq!(CodeTable::from_bytes(b"MOTEFAUL\x01\0\0\0\0\0\0\0\0\0\0\0"), Err(BytesError::Magic));
        let mut other = bytes.clone();
        other[8] = 2;
        assert_eq!(CodeTable::from_bytes(&other), Err(BytesError::Version(2)));
        let mut more = bytes.clone();
        more.push(0);
        assert_eq!(CodeTable::from_bytes(&more), Err(BytesError::Trailing));
        for cut in 0..bytes.len() {
            assert!(CodeTable::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let frame = FrameDescriptor::new(16, vec![8], vec![]).unwrap();
        let unordered = CodeTable { functions: vec![vec![(9, frame.clone()), (9, frame)]], roots: vec![] };
        assert_eq!(CodeTable::from_bytes(&unordered.to_bytes()), Err(BytesError::Invalid("the sites of a function are not in order")));
    }
}
