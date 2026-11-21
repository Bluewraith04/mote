use crate::kind::{Slot, TypeId};

/// Why a byte string is not a table of the format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BytesError {
    /// The bytes end inside a value.
    Truncated,
    /// The bytes do not start with the magic of the format.
    Magic,
    /// The version is not the one this build reads.
    Version(u32),
    /// A tag names nothing.
    Tag(u8),
    /// A text is not UTF-8.
    Text,
    /// A value breaks a rule of the format.
    Invalid(&'static str),
    /// Bytes remain after the last value.
    Trailing,
}

impl std::fmt::Display for BytesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BytesError::Truncated => write!(f, "the bytes end too soon"),
            BytesError::Magic => write!(f, "the bytes are not of this format"),
            BytesError::Version(version) => write!(f, "version {version} is not read"),
            BytesError::Tag(tag) => write!(f, "tag {tag} names nothing"),
            BytesError::Text => write!(f, "a text is not UTF-8"),
            BytesError::Invalid(what) => write!(f, "{what}"),
            BytesError::Trailing => write!(f, "bytes remain after the last value"),
        }
    }
}

impl std::error::Error for BytesError {}

/// Appends little-endian values to a byte string.
#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn u32(&mut self, value: u32) {
        self.raw(&value.to_le_bytes());
    }

    pub fn i128(&mut self, value: i128) {
        self.raw(&value.to_le_bytes());
    }

    /// A length as a `u32`.
    pub fn count(&mut self, length: usize) {
        self.u32(u32::try_from(length).expect("more than u32::MAX values"));
    }

    /// A length, then the UTF-8 bytes.
    pub fn text(&mut self, text: &str) {
        self.count(text.len());
        self.raw(text.as_bytes());
    }

    pub fn type_id(&mut self, id: TypeId) {
        self.u32(id.raw());
    }

    /// A tag, then the fields the slot has.
    pub fn slot(&mut self, slot: Slot) {
        match slot {
            Slot::Ref(offset) => {
                self.u8(SLOT_REF);
                self.u32(offset);
            }
            Slot::Enum { offset, ty } => {
                self.u8(SLOT_ENUM);
                self.u32(offset);
                self.type_id(ty);
            }
            Slot::Pair(offset) => {
                self.u8(SLOT_PAIR);
                self.u32(offset);
            }
            Slot::Repeat { offset, count, stride, ty } => {
                self.u8(SLOT_REPEAT);
                self.u32(offset);
                self.u32(count);
                self.u32(stride);
                self.type_id(ty);
            }
        }
    }

    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

const SLOT_REF: u8 = 0;
const SLOT_ENUM: u8 = 1;
const SLOT_PAIR: u8 = 2;
const SLOT_REPEAT: u8 = 3;

/// Takes little-endian values off a byte string.
pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn raw(&mut self, length: usize) -> Result<&'a [u8], BytesError> {
        if self.bytes.len() < length {
            return Err(BytesError::Truncated);
        }
        let (taken, rest) = self.bytes.split_at(length);
        self.bytes = rest;
        Ok(taken)
    }

    pub fn u8(&mut self) -> Result<u8, BytesError> {
        Ok(self.raw(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32, BytesError> {
        Ok(u32::from_le_bytes(self.raw(4)?.try_into().expect("four bytes")))
    }

    pub fn i128(&mut self) -> Result<i128, BytesError> {
        Ok(i128::from_le_bytes(self.raw(16)?.try_into().expect("sixteen bytes")))
    }

    /// A length that the bytes left can hold at `each` bytes a value, so a false length cannot ask for more memory than the input is long.
    pub fn count(&mut self, each: usize) -> Result<usize, BytesError> {
        let count = self.u32()? as usize;
        if count.checked_mul(each).is_none_or(|bytes| bytes > self.bytes.len()) {
            return Err(BytesError::Truncated);
        }
        Ok(count)
    }

    pub fn text(&mut self) -> Result<String, BytesError> {
        let length = self.count(1)?;
        String::from_utf8(self.raw(length)?.to_vec()).map_err(|_| BytesError::Text)
    }

    pub fn type_id(&mut self) -> Result<TypeId, BytesError> {
        Ok(TypeId::from_raw(self.u32()?))
    }

    pub fn slot(&mut self) -> Result<Slot, BytesError> {
        match self.u8()? {
            SLOT_REF => Ok(Slot::Ref(self.u32()?)),
            SLOT_ENUM => Ok(Slot::Enum { offset: self.u32()?, ty: self.type_id()? }),
            SLOT_PAIR => Ok(Slot::Pair(self.u32()?)),
            SLOT_REPEAT => Ok(Slot::Repeat { offset: self.u32()?, count: self.u32()?, stride: self.u32()?, ty: self.type_id()? }),
            tag => Err(BytesError::Tag(tag)),
        }
    }

    pub fn expect(&mut self, magic: &[u8]) -> Result<(), BytesError> {
        if self.raw(magic.len()).map_err(|_| BytesError::Magic)? == magic { Ok(()) } else { Err(BytesError::Magic) }
    }

    /// Fails when bytes remain.
    pub fn end(&self) -> Result<(), BytesError> {
        if self.bytes.is_empty() { Ok(()) } else { Err(BytesError::Trailing) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_values_are_little_endian() {
        let mut writer = Writer::new();
        writer.u32(0x0102_0304);
        writer.text("ab");
        assert_eq!(writer.finish(), [4, 3, 2, 1, 2, 0, 0, 0, b'a', b'b']);
    }

    #[test]
    fn bytes_every_slot_reads_back() {
        let ty = TypeId::from_raw(9);
        let slots = [Slot::Ref(8), Slot::Enum { offset: 16, ty }, Slot::Pair(24), Slot::Repeat { offset: 0, count: 3, stride: 8, ty }];
        let mut writer = Writer::new();
        for slot in slots {
            writer.slot(slot);
        }
        let bytes = writer.finish();
        let mut reader = Reader::new(&bytes);
        for slot in slots {
            assert_eq!(reader.slot(), Ok(slot));
        }
        assert_eq!(reader.end(), Ok(()));
    }

    #[test]
    fn bytes_a_short_input_is_truncated_not_a_panic() {
        assert_eq!(Reader::new(&[1, 2]).u32(), Err(BytesError::Truncated));
        assert_eq!(Reader::new(&[]).u8(), Err(BytesError::Truncated));
        assert_eq!(Reader::new(&[1, 0, 0, 0, b'a']).text(), Ok("a".to_string()));
        assert_eq!(Reader::new(&[9, 0, 0, 0, b'a']).text(), Err(BytesError::Truncated));
    }

    #[test]
    fn bytes_a_false_count_cannot_ask_for_more_than_the_input_holds() {
        assert_eq!(Reader::new(&[0xff, 0xff, 0xff, 0xff]).count(8), Err(BytesError::Truncated));
    }

    #[test]
    fn bytes_an_unknown_slot_tag_and_a_bad_text_are_errors() {
        assert_eq!(Reader::new(&[9, 0, 0, 0, 0]).slot(), Err(BytesError::Tag(9)));
        assert_eq!(Reader::new(&[1, 0, 0, 0, 0xff]).text(), Err(BytesError::Text));
    }

    #[test]
    fn bytes_the_magic_and_the_end_are_checked() {
        assert_eq!(Reader::new(b"AB").expect(b"ABC"), Err(BytesError::Magic));
        assert_eq!(Reader::new(b"ABD").expect(b"ABC"), Err(BytesError::Magic));
        assert_eq!(Reader::new(b"ABC").expect(b"ABC"), Ok(()));
        assert_eq!(Reader::new(&[0]).end(), Err(BytesError::Trailing));
    }
}
