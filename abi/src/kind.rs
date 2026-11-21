use crate::handle::HandleKind;
use crate::scalar::Scalar;

/// A run-time type, as stored in an object header and a union pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(u32);

impl TypeId {
    /// Reserved so that a zeroed word is never a type.
    pub const NONE: TypeId = TypeId(0);

    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What a concrete type is made of; every type it names is already in the table.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `Null`, and any type with one value.
    Unit,
    Scalar(Scalar),
    /// A value struct or a tuple, with fields inline.
    Struct(Vec<TypeId>),
    /// A value enum: each variant lists its payload types.
    Enum(Vec<Vec<TypeId>>),
    /// `T?`, a reference itself when `T` is a reference object.
    Optional(TypeId),
    /// A union: a type-id word and a payload word.
    Pair,
    /// `[T; N]`, inline.
    FixedArray(TypeId, u32),
    /// A reference object with the given fields.
    Class(Vec<TypeId>),
    /// A heap cell holding one value.
    Box(TypeId),
    /// An immutable UTF-8 string.
    String,
    /// `[T]`, and the buffer behind a list.
    Array(TypeId),
    List(TypeId),
    /// A function value: a code pointer and the given captures.
    Closure(Vec<TypeId>),
    /// A reference to an object the runtime owns, with the payload types its kind lists.
    Handle(HandleKind, Vec<TypeId>),
    /// An object type named before it is defined.
    Reserved,
}

impl Kind {
    /// Whether a value of this kind is a reference to a heap object.
    pub fn is_object(&self) -> bool {
        matches!(self, Kind::Class(_) | Kind::Box(_) | Kind::String | Kind::Array(_) | Kind::List(_) | Kind::Closure(_) | Kind::Handle(..) | Kind::Reserved)
    }
}

/// A field and where it sits from the start of its value or object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub ty: TypeId,
    pub offset: u32,
}

/// One variant of an enum, with offsets from the start of the enum.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub fields: Vec<Field>,
    pub slots: Vec<Slot>,
}

/// A place the collector looks for references, as an offset from the start of its value or object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Ref(u32),
    /// An enum whose live variant's slots apply, found by reading its tag.
    Enum { offset: u32, ty: TypeId },
    /// A union pair whose payload is a reference when its type id says so.
    Pair(u32),
    /// `count` values of `ty`, `stride` bytes apart.
    Repeat { offset: u32, count: u32, stride: u32, ty: TypeId },
}

impl Slot {
    pub fn shifted(self, by: u32) -> Slot {
        match self {
            Slot::Ref(offset) => Slot::Ref(offset + by),
            Slot::Enum { offset, ty } => Slot::Enum { offset: offset + by, ty },
            Slot::Pair(offset) => Slot::Pair(offset + by),
            Slot::Repeat { offset, count, stride, ty } => Slot::Repeat { offset: offset + by, count, stride, ty },
        }
    }
}

/// The elements an object holds after its fixed part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Element {
    /// `None` for the bytes of a string.
    pub ty: Option<TypeId>,
    pub stride: u32,
    pub offset: u32,
}

/// How a union pair carries a member in its payload word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairPayload {
    /// The scalar itself.
    Inline,
    /// The reference itself.
    Reference,
    /// A reference to a box holding the value.
    Boxed,
}

/// The heap shape of a reference type.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    /// Bytes before any elements, header included.
    pub size: u32,
    pub fields: Vec<Field>,
    pub slots: Vec<Slot>,
    pub element: Option<Element>,
}

impl Object {
    /// Bytes an object with `count` elements takes, rounded to a word.
    pub fn byte_size(&self, count: u64) -> u64 {
        let stride = self.element.map_or(0, |element| u64::from(element.stride));
        let bytes = u64::from(self.size) + stride * count;
        bytes.div_ceil(8) * 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_shift_every_offset() {
        let ty = TypeId::from_raw(3);
        assert_eq!(Slot::Ref(4).shifted(8), Slot::Ref(12));
        assert_eq!(Slot::Pair(0).shifted(16), Slot::Pair(16));
        assert_eq!(Slot::Enum { offset: 8, ty }.shifted(8), Slot::Enum { offset: 16, ty });
        assert_eq!(Slot::Repeat { offset: 0, count: 2, stride: 8, ty }.shifted(8), Slot::Repeat { offset: 8, count: 2, stride: 8, ty });
    }

    #[test]
    fn object_size_rounds_to_a_word() {
        let object = Object { size: 16, fields: Vec::new(), slots: Vec::new(), element: Some(Element { ty: None, stride: 1, offset: 16 }) };
        assert_eq!(object.byte_size(0), 16);
        assert_eq!(object.byte_size(5), 24);
        assert_eq!(object.byte_size(8), 24);
        assert_eq!(object.byte_size(9), 32);
    }

    #[test]
    fn only_reference_kinds_are_objects() {
        assert!(Kind::String.is_object());
        assert!(Kind::Handle(HandleKind::Port, Vec::new()).is_object());
        assert!(Kind::Reserved.is_object());
        assert!(!Kind::Pair.is_object());
        assert!(!Kind::Optional(TypeId::NONE).is_object());
    }
}
