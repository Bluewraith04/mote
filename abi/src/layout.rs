/// Bytes in a machine word and in every pointer.
pub const WORD: u32 = 8;

/// Bytes of the header that starts every heap object.
pub const HEADER_SIZE: u32 = 8;

/// Where a string, array or list keeps its length.
pub const LENGTH_OFFSET: u32 = 8;

/// Where the bytes of a string and the elements of an array start.
pub const ELEMENTS_OFFSET: u32 = 16;

/// Where a list keeps its capacity.
pub const LIST_CAPACITY_OFFSET: u32 = 16;

/// Where a list keeps the reference to its buffer.
pub const LIST_BUFFER_OFFSET: u32 = 24;

/// Where a closure keeps its code pointer.
pub const CLOSURE_CODE_OFFSET: u32 = 8;

/// Where a closure's captured values start.
pub const CLOSURE_CAPTURES_OFFSET: u32 = 16;

/// Where the payload of a boxed value starts.
pub const BOX_VALUE_OFFSET: u32 = 8;

/// Where the payload word of a union pair starts, after the type-id word.
pub const PAIR_PAYLOAD_OFFSET: u32 = 8;

/// The size and alignment of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: u32,
    pub align: u32,
}

impl Layout {
    pub const ZERO: Layout = Layout { size: 0, align: 1 };

    pub const REFERENCE: Layout = Layout { size: WORD, align: WORD };

    pub const fn new(size: u32, align: u32) -> Self {
        Self { size, align }
    }
}

/// Rounds `offset` up to a multiple of `align`, a power of two.
pub const fn align_up(offset: u32, align: u32) -> u32 {
    (offset + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_up_rounds_to_the_next_multiple() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 4), 12);
        assert_eq!(align_up(5, 1), 5);
    }
}
