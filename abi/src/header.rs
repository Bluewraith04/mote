use crate::kind::TypeId;

/// The object lives in the large-object space.
pub const FLAG_LARGE: u32 = 1 << 0;

/// The object owns an OS resource and is finalized when it dies.
pub const FLAG_FINALIZE: u32 = 1 << 1;

/// The object lives in static data outside the heap and holds references only to other static objects.
pub const FLAG_STATIC: u32 = 1 << 2;

/// The flag bits no flag uses; they stay zero.
pub const FLAGS_RESERVED: u32 = !(FLAG_LARGE | FLAG_FINALIZE | FLAG_STATIC);

/// The word that starts every heap object: a type id and flag bits the heap owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header(u64);

impl Header {
    pub const fn new(ty: TypeId, flags: u32) -> Self {
        assert!(flags & FLAGS_RESERVED == 0, "header flag is not defined");
        Self(((flags as u64) << 32) | ty.raw() as u64)
    }

    pub const fn has(self, flag: u32) -> bool {
        self.flags() & flag != 0
    }

    pub const fn with(self, flag: u32) -> Self {
        Self::new(self.type_id(), self.flags() | flag)
    }

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn type_id(self) -> TypeId {
        TypeId::from_raw(self.0 as u32)
    }

    pub const fn flags(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_type_id_and_flags_round_trip() {
        let header = Header::new(TypeId::from_raw(7), 0b11);
        assert_eq!(header.type_id(), TypeId::from_raw(7));
        assert_eq!(header.flags(), 0b11);
        assert_eq!(Header::from_bits(header.bits()), header);
    }

    #[test]
    fn header_flags_each_flag_is_one_bit_of_the_high_half() {
        assert_eq!([FLAG_LARGE, FLAG_FINALIZE, FLAG_STATIC], [1, 2, 4]);
        let header = Header::new(TypeId::from_raw(5), FLAG_FINALIZE);
        assert_eq!(header.bits(), (2u64 << 32) | 5);
        assert!(header.has(FLAG_FINALIZE) && !header.has(FLAG_LARGE));
        assert!(header.with(FLAG_LARGE).has(FLAG_LARGE));
        assert_eq!(header.with(FLAG_LARGE).type_id(), TypeId::from_raw(5));
    }

    #[test]
    fn header_flags_every_other_bit_is_reserved() {
        assert_eq!(FLAGS_RESERVED, !7);
    }

    #[test]
    #[should_panic(expected = "header flag is not defined")]
    fn header_flags_a_reserved_bit_is_rejected() {
        Header::new(TypeId::from_raw(1), 1 << 3);
    }

    #[test]
    fn a_zeroed_word_names_no_type() {
        assert_eq!(Header::from_bits(0).type_id(), TypeId::NONE);
    }

    #[test]
    fn the_top_type_id_does_not_reach_the_flags() {
        let header = Header::new(TypeId::from_raw(u32::MAX), 0);
        assert_eq!(header.flags(), 0);
        assert_eq!(header.type_id().raw(), u32::MAX);
    }
}
