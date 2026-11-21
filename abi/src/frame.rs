use crate::bytes::{BytesError, Reader, Writer};
use crate::kind::Slot;
use crate::layout::WORD;

/// What the collector reads from one frame at one safepoint: where it holds references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDescriptor {
    /// Bytes of the frame, a whole number of words.
    pub frame_size: u32,
    /// Offsets from the frame base of the live `Ref` values, ascending.
    pub refs: Vec<u32>,
    /// The live values whose references depend on their contents, by offset from the frame base, ascending; never a `Ref`.
    pub dynamic: Vec<Slot>,
}

fn offset_of(slot: &Slot) -> u32 {
    match *slot {
        Slot::Ref(offset) | Slot::Pair(offset) | Slot::Enum { offset, .. } | Slot::Repeat { offset, .. } => offset,
    }
}

impl FrameDescriptor {
    /// Sorts the slots by offset and checks that each lies whole inside the frame, on a word, once.
    pub fn new(frame_size: u32, mut refs: Vec<u32>, mut dynamic: Vec<Slot>) -> Result<Self, BytesError> {
        if !frame_size.is_multiple_of(WORD) {
            return Err(BytesError::Invalid("a frame is a whole number of words"));
        }
        if dynamic.iter().any(|slot| matches!(slot, Slot::Ref(_))) {
            return Err(BytesError::Invalid("a plain reference is listed among the references"));
        }
        refs.sort_unstable();
        dynamic.sort_by_key(offset_of);
        let mut offsets: Vec<u32> = refs.iter().copied().chain(dynamic.iter().map(offset_of)).collect();
        offsets.sort_unstable();
        if offsets.iter().any(|offset| offset % WORD != 0 || offset.checked_add(WORD).is_none_or(|end| end > frame_size)) {
            return Err(BytesError::Invalid("a slot lies off a word or outside the frame"));
        }
        if offsets.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(BytesError::Invalid("two slots share an offset"));
        }
        Ok(Self { frame_size, refs, dynamic })
    }

    /// Every slot, plain references and dynamic ones, by offset.
    pub fn slots(&self) -> Vec<Slot> {
        let mut slots: Vec<Slot> = self.refs.iter().map(|offset| Slot::Ref(*offset)).chain(self.dynamic.iter().copied()).collect();
        slots.sort_by_key(offset_of);
        slots
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u32(self.frame_size);
        writer.count(self.refs.len());
        for offset in &self.refs {
            writer.u32(*offset);
        }
        writer.count(self.dynamic.len());
        for slot in &self.dynamic {
            writer.slot(*slot);
        }
        writer.finish()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BytesError> {
        let mut reader = Reader::new(bytes);
        let frame_size = reader.u32()?;
        let refs = (0..reader.count(4)?).map(|_| reader.u32()).collect::<Result<Vec<_>, _>>()?;
        let dynamic = (0..reader.count(5)?).map(|_| reader.slot()).collect::<Result<Vec<_>, _>>()?;
        reader.end()?;
        let descriptor = Self::new(frame_size, refs.clone(), dynamic.clone())?;
        if descriptor.refs != refs || descriptor.dynamic != dynamic {
            return Err(BytesError::Invalid("the slots are not in order"));
        }
        Ok(descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::TypeId;

    fn ty() -> TypeId {
        TypeId::from_raw(4)
    }

    #[test]
    fn frames_a_descriptor_keeps_the_references_and_the_dynamic_slots_apart() {
        let descriptor = FrameDescriptor::new(64, vec![24, 8], vec![Slot::Pair(32), Slot::Enum { offset: 48, ty: ty() }]).unwrap();
        assert_eq!(descriptor.refs, [8, 24]);
        assert_eq!(descriptor.dynamic, [Slot::Pair(32), Slot::Enum { offset: 48, ty: ty() }]);
        assert_eq!(descriptor.slots(), [Slot::Ref(8), Slot::Ref(24), Slot::Pair(32), Slot::Enum { offset: 48, ty: ty() }]);
    }

    #[test]
    fn frames_the_byte_form_is_the_size_then_the_references_then_the_dynamic_slots() {
        let descriptor = FrameDescriptor::new(32, vec![8], vec![Slot::Pair(16)]).unwrap();
        assert_eq!(descriptor.to_bytes(), [32, 0, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0, 1, 0, 0, 0, 2, 16, 0, 0, 0]);
    }

    #[test]
    fn frames_a_descriptor_reads_back_equal_and_gives_one_byte_string() {
        let descriptor = FrameDescriptor::new(
            128,
            vec![56, 0, 8],
            vec![Slot::Repeat { offset: 64, count: 2, stride: 16, ty: ty() }, Slot::Enum { offset: 16, ty: ty() }, Slot::Pair(96)],
        )
        .unwrap();
        let bytes = descriptor.to_bytes();
        let read = FrameDescriptor::from_bytes(&bytes).unwrap();
        assert_eq!(read, descriptor);
        assert_eq!(read.to_bytes(), bytes);
        let other = FrameDescriptor::new(128, vec![8, 56, 0], descriptor.dynamic.iter().rev().copied().collect()).unwrap();
        assert_eq!(other.to_bytes(), bytes);
    }

    #[test]
    fn frames_an_empty_frame_is_three_zero_words_of_four_bytes() {
        let descriptor = FrameDescriptor::new(0, vec![], vec![]).unwrap();
        assert_eq!(descriptor.to_bytes(), [0; 12]);
        assert_eq!(FrameDescriptor::from_bytes(&descriptor.to_bytes()), Ok(descriptor));
    }

    #[test]
    fn frames_a_slot_off_a_word_outside_the_frame_or_twice_is_refused() {
        assert!(FrameDescriptor::new(32, vec![4], vec![]).is_err());
        assert!(FrameDescriptor::new(32, vec![32], vec![]).is_err());
        assert!(FrameDescriptor::new(30, vec![], vec![]).is_err());
        assert!(FrameDescriptor::new(32, vec![8, 8], vec![]).is_err());
        assert!(FrameDescriptor::new(32, vec![8], vec![Slot::Pair(8)]).is_err());
        assert!(FrameDescriptor::new(32, vec![], vec![Slot::Ref(8)]).is_err());
    }

    #[test]
    fn frames_bytes_out_of_order_or_cut_short_are_refused() {
        let unordered = [32, 0, 0, 0, 2, 0, 0, 0, 16, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0];
        assert!(FrameDescriptor::from_bytes(&unordered).is_err());
        let good = FrameDescriptor::new(32, vec![8], vec![]).unwrap().to_bytes();
        assert_eq!(FrameDescriptor::from_bytes(&good[..good.len() - 1]), Err(BytesError::Truncated));
        let mut long = good.clone();
        long.push(0);
        assert_eq!(FrameDescriptor::from_bytes(&long), Err(BytesError::Trailing));
    }
}
