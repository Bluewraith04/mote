use crate::layout::Layout;

/// A machine value that fits one register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scalar {
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Ref,
}

impl Scalar {
    pub fn size(self) -> u32 {
        match self {
            Scalar::I8 => 1,
            Scalar::I16 => 2,
            Scalar::I32 | Scalar::F32 => 4,
            Scalar::I64 | Scalar::F64 | Scalar::Ref => 8,
        }
    }

    pub fn layout(self) -> Layout {
        Layout::new(self.size(), self.size())
    }

    pub fn is_reference(self) -> bool {
        self == Scalar::Ref
    }

    /// The scalar's number in a byte form; the order is `ALL`.
    pub fn tag(self) -> u8 {
        Scalar::ALL.iter().position(|scalar| *scalar == self).expect("every scalar is listed") as u8
    }

    pub fn from_tag(tag: u8) -> Option<Scalar> {
        Scalar::ALL.get(usize::from(tag)).copied()
    }

    pub const ALL: [Scalar; 7] = [Scalar::I8, Scalar::I16, Scalar::I32, Scalar::I64, Scalar::F32, Scalar::F64, Scalar::Ref];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_the_machine_types() {
        assert_eq!(Scalar::I8.size(), 1);
        assert_eq!(Scalar::F32.size(), 4);
        assert_eq!(Scalar::Ref.size(), 8);
        assert_eq!(Scalar::I16.layout(), Layout::new(2, 2));
    }

    #[test]
    fn a_scalar_aligns_to_its_size() {
        for scalar in Scalar::ALL {
            assert_eq!(scalar.layout(), Layout::new(scalar.size(), scalar.size()));
        }
        let sizes: Vec<u32> = Scalar::ALL.iter().map(|scalar| scalar.size()).collect();
        assert_eq!(sizes, [1, 2, 4, 8, 4, 8, 8]);
    }

    #[test]
    fn tags_are_the_positions_in_all() {
        for (index, scalar) in Scalar::ALL.iter().enumerate() {
            assert_eq!(scalar.tag() as usize, index);
            assert_eq!(Scalar::from_tag(scalar.tag()), Some(*scalar));
        }
        assert_eq!(Scalar::from_tag(7), None);
    }

    #[test]
    fn only_ref_is_a_reference() {
        assert!(Scalar::Ref.is_reference());
        assert!(!Scalar::I64.is_reference());
    }
}
