use crate::kind::{Element, Field, Kind, Object, PairPayload, Slot, TypeId, Variant};
use crate::layout::{
    align_up, Layout, BOX_VALUE_OFFSET, CLOSURE_CAPTURES_OFFSET, ELEMENTS_OFFSET, HEADER_SIZE, LIST_BUFFER_OFFSET, WORD,
};
use crate::scalar::Scalar;

/// Bytes of the tag of an optional that is not a reference.
const OPTIONAL_TAG_SIZE: u32 = 8;

/// Bytes of a list object: header, length, capacity and buffer.
const LIST_SIZE: u32 = 32;

/// What the compiler and runtime know about one concrete type.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeInfo {
    pub name: String,
    pub kind: Kind,
    /// A value of this type: a reference for every object kind.
    pub layout: Layout,
    /// Struct fields, from the start of the value.
    pub fields: Vec<Field>,
    pub variants: Vec<Variant>,
    /// Bytes of the tag of an enum, else 0.
    pub tag_size: u32,
    /// Where a value of this type holds references, from the start of the value.
    pub slots: Vec<Slot>,
    /// The heap shape, for object kinds.
    pub object: Option<Object>,
    pending: bool,
}

impl TypeInfo {
    fn new(name: String, kind: Kind, layout: Layout) -> Self {
        Self { name, kind, layout, fields: Vec::new(), variants: Vec::new(), tag_size: 0, slots: Vec::new(), object: None, pending: false }
    }

    /// Whether a value of this type is one pointer, null when it is an absent optional.
    pub fn is_reference(&self) -> bool {
        self.kind.is_object() || matches!(self.kind, Kind::Scalar(Scalar::Ref)) || (matches!(self.kind, Kind::Optional(_)) && self.variants.is_empty())
    }
}

/// Every concrete type of a program, indexed by `TypeId`.
#[derive(Debug, PartialEq)]
pub struct TypeTable {
    types: Vec<TypeInfo>,
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeTable {
    pub fn new() -> Self {
        Self { types: vec![TypeInfo::new("<none>".to_string(), Kind::Unit, Layout::ZERO)] }
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.len() == 1
    }

    pub fn get(&self, id: TypeId) -> &TypeInfo {
        &self.types[id.index()]
    }

    pub fn get_raw(&self, raw: u32) -> Option<&TypeInfo> {
        self.types.get(raw as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (TypeId, &TypeInfo)> {
        self.types.iter().enumerate().map(|(index, info)| (TypeId::from_raw(index as u32), info))
    }

    pub fn layout(&self, id: TypeId) -> Layout {
        self.get(id).layout
    }

    /// Adds a type whose members are already in the table.
    pub fn add(&mut self, name: impl Into<String>, kind: Kind) -> TypeId {
        if kind.is_object() {
            let id = self.reserve_object(name);
            self.define(id, kind);
            return id;
        }
        let id = self.next_id();
        let info = self.value_info(id, name.into(), kind);
        self.types.push(info);
        id
    }

    /// Reserves an object type so a type it refers to, itself included, can name it before it is defined.
    pub fn reserve_object(&mut self, name: impl Into<String>) -> TypeId {
        let id = self.next_id();
        let mut info = TypeInfo::new(name.into(), Kind::Reserved, Layout::REFERENCE);
        info.slots.push(Slot::Ref(0));
        info.pending = true;
        self.types.push(info);
        id
    }

    /// Defines a reserved object type.
    pub fn define(&mut self, id: TypeId, kind: Kind) {
        assert!(self.get(id).pending, "type is not a reserved object");
        assert!(kind.is_object(), "a reserved type is defined as an object");
        let object = self.object_info(&kind);
        let info = &mut self.types[id.index()];
        info.kind = kind;
        info.object = Some(object);
        info.pending = false;
    }

    pub fn pair_payload(&self, id: TypeId) -> PairPayload {
        let info = self.get(id);
        match info.kind {
            Kind::Unit => PairPayload::Inline,
            Kind::Scalar(scalar) if !scalar.is_reference() => PairPayload::Inline,
            _ if info.is_reference() => PairPayload::Reference,
            _ => PairPayload::Boxed,
        }
    }

    fn next_id(&self) -> TypeId {
        TypeId::from_raw(u32::try_from(self.types.len()).expect("more than u32::MAX types"))
    }

    fn value_info(&self, id: TypeId, name: String, kind: Kind) -> TypeInfo {
        match &kind {
            Kind::Unit => TypeInfo::new(name, kind, Layout::ZERO),
            Kind::Scalar(scalar) => {
                let mut info = TypeInfo::new(name, kind.clone(), scalar.layout());
                if scalar.is_reference() {
                    info.slots.push(Slot::Ref(0));
                }
                info
            }
            Kind::Struct(types) => {
                let placed = self.place(types, 0);
                let mut info = TypeInfo::new(name, kind.clone(), Layout::new(align_up(placed.end, placed.align), placed.align));
                info.fields = placed.fields;
                info.slots = placed.slots;
                info
            }
            Kind::Enum(variants) => self.enum_info(id, name, kind.clone(), variants, enum_tag_size(variants.len())),
            Kind::Optional(inner) => {
                if self.get(*inner).kind.is_object() {
                    let mut info = TypeInfo::new(name, kind.clone(), Layout::REFERENCE);
                    info.slots.push(Slot::Ref(0));
                    info
                } else {
                    self.enum_info(id, name, kind.clone(), &[Vec::new(), vec![*inner]], OPTIONAL_TAG_SIZE)
                }
            }
            Kind::Pair => {
                let mut info = TypeInfo::new(name, kind, Layout::new(2 * WORD, WORD));
                info.slots.push(Slot::Pair(0));
                info
            }
            Kind::FixedArray(element, count) => {
                let layout = self.layout(*element);
                let stride = align_up(layout.size, layout.align);
                let size = stride.checked_mul(*count).expect("array larger than 4 GiB");
                let mut info = TypeInfo::new(name, kind.clone(), Layout::new(size, layout.align));
                if *count > 0 && !self.get(*element).slots.is_empty() {
                    info.slots.push(Slot::Repeat { offset: 0, count: *count, stride, ty: *element });
                }
                info
            }
            _ => unreachable!("object kinds are added through reserve_object"),
        }
    }

    fn enum_info(&self, id: TypeId, name: String, kind: Kind, variants: &[Vec<TypeId>], tag_size: u32) -> TypeInfo {
        if variants.is_empty() {
            return TypeInfo::new(name, kind, Layout::ZERO);
        }
        let (mut align, mut end, mut live) = (tag_size, tag_size, false);
        let mut placed_variants = Vec::new();
        for types in variants {
            let placed = self.place(types, tag_size);
            align = align.max(placed.align);
            end = end.max(placed.end);
            live |= !placed.slots.is_empty();
            placed_variants.push(Variant { fields: placed.fields, slots: placed.slots });
        }
        let mut info = TypeInfo::new(name, kind, Layout::new(align_up(end, align), align));
        info.variants = placed_variants;
        info.tag_size = tag_size;
        if live {
            info.slots.push(Slot::Enum { offset: 0, ty: id });
        }
        info
    }

    fn object_info(&self, kind: &Kind) -> Object {
        match kind {
            Kind::Class(fields) => self.placed_object(fields, HEADER_SIZE),
            Kind::Closure(captures) => self.placed_object(captures, CLOSURE_CAPTURES_OFFSET),
            Kind::Box(inner) => self.placed_object(&[*inner], BOX_VALUE_OFFSET),
            Kind::String => Object { size: ELEMENTS_OFFSET, fields: Vec::new(), slots: Vec::new(), element: Some(Element { ty: None, stride: 1, offset: ELEMENTS_OFFSET }) },
            Kind::Array(element) => {
                let layout = self.layout(*element);
                let stride = align_up(layout.size, layout.align);
                Object { size: ELEMENTS_OFFSET, fields: Vec::new(), slots: Vec::new(), element: Some(Element { ty: Some(*element), stride, offset: ELEMENTS_OFFSET }) }
            }
            Kind::List(_) => Object { size: LIST_SIZE, fields: Vec::new(), slots: vec![Slot::Ref(LIST_BUFFER_OFFSET)], element: None },
            Kind::Handle(handle, payload) => {
                assert_eq!(payload.len(), handle.payloads(), "handle payload of {handle:?}");
                let mut object = self.placed_object(payload, handle.fixed_size());
                object.slots.splice(0..0, handle.slots().iter().copied());
                object
            }
            _ => unreachable!("only object kinds have a heap shape"),
        }
    }

    fn placed_object(&self, types: &[TypeId], start: u32) -> Object {
        let placed = self.place(types, start);
        Object { size: align_up(placed.end, WORD), fields: placed.fields, slots: placed.slots, element: None }
    }

    fn place(&self, types: &[TypeId], start: u32) -> Placed {
        let mut placed = Placed { fields: Vec::new(), slots: Vec::new(), end: start, align: 1 };
        for &ty in types {
            let info = self.get(ty);
            let offset = align_up(placed.end, info.layout.align);
            placed.fields.push(Field { ty, offset });
            placed.slots.extend(info.slots.iter().map(|slot| slot.shifted(offset)));
            placed.end = offset.checked_add(info.layout.size).expect("type larger than 4 GiB");
            placed.align = placed.align.max(info.layout.align);
        }
        placed
    }
}

struct Placed {
    fields: Vec<Field>,
    slots: Vec<Slot>,
    end: u32,
    align: u32,
}

fn enum_tag_size(variants: usize) -> u32 {
    match variants {
        0..=256 => 1,
        257..=65536 => 2,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(table: &mut TypeTable, name: &str, scalar: Scalar) -> TypeId {
        table.add(name, Kind::Scalar(scalar))
    }

    fn off(ty: TypeId, offset: u32) -> Field {
        Field { ty, offset }
    }

    #[test]
    fn none_is_the_reserved_first_type() {
        let table = TypeTable::new();
        assert!(table.is_empty());
        assert_eq!(table.get(TypeId::NONE).layout, Layout::ZERO);
        assert!(table.get_raw(1).is_none());
    }

    #[test]
    fn struct_fields_follow_declaration_order_with_padding() {
        let mut table = TypeTable::new();
        let byte = scalar(&mut table, "Int8", Scalar::I8);
        let int = scalar(&mut table, "Int", Scalar::I64);
        let id = table.add("S", Kind::Struct(vec![byte, int, byte]));
        let info = table.get(id);
        assert_eq!(info.fields, [off(byte, 0), off(int, 8), off(byte, 16)]);
        assert_eq!(info.layout, Layout::new(24, 8));
        assert!(info.slots.is_empty());
    }

    #[test]
    fn nested_structs_are_flat_and_keep_their_reference_slots() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let string = table.add("String", Kind::String);
        let inner = table.add("Inner", Kind::Struct(vec![int, string]));
        let outer = table.add("Outer", Kind::Struct(vec![int, inner]));
        let info = table.get(outer);
        assert_eq!(info.layout, Layout::new(24, 8));
        assert_eq!(info.slots, [Slot::Ref(16)]);
    }

    #[test]
    fn unit_and_empty_structs_take_no_room() {
        let mut table = TypeTable::new();
        let unit = table.add("Null", Kind::Unit);
        let empty = table.add("Empty", Kind::Struct(vec![unit]));
        assert_eq!(table.layout(unit), Layout::ZERO);
        assert_eq!(table.layout(empty), Layout::ZERO);
    }

    #[test]
    fn a_unit_only_enum_is_its_tag() {
        let mut table = TypeTable::new();
        let id = table.add("Light", Kind::Enum(vec![vec![], vec![], vec![]]));
        let info = table.get(id);
        assert_eq!(info.layout, Layout::new(1, 1));
        assert_eq!(info.tag_size, 1);
    }

    #[test]
    fn an_enum_holds_its_largest_variant_after_the_tag() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let id = table.add("Shape", Kind::Enum(vec![vec![int], vec![int, int], vec![]]));
        let info = table.get(id);
        assert_eq!(info.layout, Layout::new(24, 8));
        assert_eq!(info.variants[1].fields, [off(int, 8), off(int, 16)]);
    }

    #[test]
    fn a_small_payload_sits_right_after_the_tag() {
        let mut table = TypeTable::new();
        let small = scalar(&mut table, "Int32", Scalar::I32);
        let id = table.add("E", Kind::Enum(vec![vec![small]]));
        let info = table.get(id);
        assert_eq!(info.variants[0].fields, [off(small, 4)]);
        assert_eq!(info.layout, Layout::new(8, 4));
    }

    #[test]
    fn an_enum_with_references_has_one_dynamic_slot() {
        let mut table = TypeTable::new();
        let string = table.add("String", Kind::String);
        let int = scalar(&mut table, "Int", Scalar::I64);
        let id = table.add("E", Kind::Enum(vec![vec![string], vec![int]]));
        let info = table.get(id);
        assert_eq!(info.slots, [Slot::Enum { offset: 0, ty: id }]);
        assert_eq!(info.variants[0].slots, [Slot::Ref(8)]);
        assert!(info.variants[1].slots.is_empty());
    }

    #[test]
    fn more_than_256_variants_widen_the_tag() {
        let mut table = TypeTable::new();
        let id = table.add("Wide", Kind::Enum(vec![vec![]; 257]));
        assert_eq!(table.get(id).tag_size, 2);
        assert_eq!(table.layout(id), Layout::new(2, 2));
    }

    #[test]
    fn an_optional_of_a_scalar_is_a_tag_word_and_a_payload() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let id = table.add("Int?", Kind::Optional(int));
        let info = table.get(id);
        assert_eq!(info.layout, Layout::new(16, 8));
        assert_eq!(info.variants[1].fields, [off(int, 8)]);
        assert!(!info.is_reference());
    }

    #[test]
    fn an_optional_of_an_object_is_the_pointer_itself() {
        let mut table = TypeTable::new();
        let string = table.add("String", Kind::String);
        let id = table.add("String?", Kind::Optional(string));
        let info = table.get(id);
        assert_eq!(info.layout, Layout::REFERENCE);
        assert_eq!(info.slots, [Slot::Ref(0)]);
        assert!(info.is_reference());
    }

    #[test]
    fn an_optional_of_an_optional_object_needs_a_tag() {
        let mut table = TypeTable::new();
        let string = table.add("String", Kind::String);
        let once = table.add("String?", Kind::Optional(string));
        let twice = table.add("String??", Kind::Optional(once));
        assert_eq!(table.layout(twice), Layout::new(16, 8));
        assert_eq!(table.get(twice).variants[1].slots, [Slot::Ref(8)]);
    }

    #[test]
    fn a_pair_is_a_type_word_and_a_payload_word() {
        let mut table = TypeTable::new();
        let id = table.add("Int | String", Kind::Pair);
        assert_eq!(table.layout(id), Layout::new(16, 8));
        assert_eq!(table.get(id).slots, [Slot::Pair(0)]);
    }

    #[test]
    fn pair_members_are_inline_references_or_boxed() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let string = table.add("String", Kind::String);
        let point = table.add("Point", Kind::Struct(vec![int, int]));
        let unit = table.add("Null", Kind::Unit);
        assert_eq!(table.pair_payload(int), PairPayload::Inline);
        assert_eq!(table.pair_payload(unit), PairPayload::Inline);
        assert_eq!(table.pair_payload(string), PairPayload::Reference);
        assert_eq!(table.pair_payload(point), PairPayload::Boxed);
    }

    #[test]
    fn a_fixed_array_is_inline_with_a_repeating_slot() {
        let mut table = TypeTable::new();
        let small = scalar(&mut table, "Int32", Scalar::I32);
        let string = table.add("String", Kind::String);
        let numbers = table.add("[Int32; 3]", Kind::FixedArray(small, 3));
        let strings = table.add("[String; 2]", Kind::FixedArray(string, 2));
        assert_eq!(table.layout(numbers), Layout::new(12, 4));
        assert!(table.get(numbers).slots.is_empty());
        assert_eq!(table.get(strings).slots, [Slot::Repeat { offset: 0, count: 2, stride: 8, ty: string }]);
    }

    #[test]
    fn a_class_starts_after_the_header() {
        let mut table = TypeTable::new();
        let byte = scalar(&mut table, "Int8", Scalar::I8);
        let string = table.add("String", Kind::String);
        let id = table.add("C", Kind::Class(vec![byte, string]));
        let info = table.get(id);
        assert_eq!(info.layout, Layout::REFERENCE);
        let object = info.object.as_ref().unwrap();
        assert_eq!(object.fields, [off(byte, 8), off(string, 16)]);
        assert_eq!(object.slots, [Slot::Ref(16)]);
        assert_eq!(object.size, 24);
    }

    #[test]
    fn a_class_can_refer_to_itself_through_an_optional() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let node = table.reserve_object("Node");
        let next = table.add("Node?", Kind::Optional(node));
        table.define(node, Kind::Class(vec![int, next]));
        let object = table.get(node).object.as_ref().unwrap();
        assert_eq!(object.fields, [off(int, 8), off(next, 16)]);
        assert_eq!(object.slots, [Slot::Ref(16)]);
        assert_eq!(table.layout(next), Layout::REFERENCE);
    }

    #[test]
    fn a_recursive_enum_holds_its_children_in_boxes() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let child = table.reserve_object("Box<Tree>");
        let tree = table.add("Tree", Kind::Enum(vec![vec![int], vec![child, child]]));
        table.define(child, Kind::Box(tree));
        let info = table.get(tree);
        assert_eq!(info.layout, Layout::new(24, 8));
        assert_eq!(info.variants[1].slots, [Slot::Ref(8), Slot::Ref(16)]);
        let object = table.get(child).object.as_ref().unwrap();
        assert_eq!(object.size, 32);
        assert_eq!(object.fields, [off(tree, 8)]);
        assert_eq!(object.slots, [Slot::Enum { offset: 8, ty: tree }]);
    }

    #[test]
    fn a_string_keeps_its_length_then_its_bytes() {
        let mut table = TypeTable::new();
        let id = table.add("String", Kind::String);
        let object = table.get(id).object.as_ref().unwrap();
        assert_eq!(object.size, 16);
        assert_eq!(object.byte_size(5), 24);
        assert_eq!(object.element, Some(Element { ty: None, stride: 1, offset: 16 }));
    }

    #[test]
    fn an_array_strides_by_its_element() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let byte = scalar(&mut table, "Int8", Scalar::I8);
        let longs = table.add("[Int]", Kind::Array(int));
        let bytes = table.add("[Int8]", Kind::Array(byte));
        assert_eq!(table.get(longs).object.as_ref().unwrap().byte_size(3), 40);
        assert_eq!(table.get(bytes).object.as_ref().unwrap().byte_size(3), 24);
        assert_eq!(table.get(longs).object.as_ref().unwrap().element.unwrap().ty, Some(int));
    }

    #[test]
    fn a_list_points_at_a_separate_buffer() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let id = table.add("List<Int>", Kind::List(int));
        let object = table.get(id).object.as_ref().unwrap();
        assert_eq!(object.size, 32);
        assert_eq!(object.slots, [Slot::Ref(24)]);
    }

    #[test]
    fn a_closure_keeps_its_code_pointer_before_its_captures() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let plain = table.add("fn", Kind::Closure(vec![]));
        let capturing = table.add("fn#2", Kind::Closure(vec![int]));
        assert_eq!(table.get(plain).object.as_ref().unwrap().size, 16);
        let object = table.get(capturing).object.as_ref().unwrap();
        assert_eq!(object.fields, [off(int, 16)]);
        assert_eq!(object.size, 24);
    }

    #[test]
    fn layout_every_type_aligns_to_a_power_of_two_of_at_most_eight_and_every_object_is_whole_words() {
        let image = crate::sample::everything();
        for (id, info) in image.table.iter().skip(1) {
            let layout = image.table.layout(id);
            assert!(layout.align.is_power_of_two() && layout.align <= 8, "{}", info.name);
            assert_eq!(layout.size % layout.align, 0, "{}", info.name);
            if let Some(object) = &info.object {
                assert_eq!(object.size % 8, 0, "{}", info.name);
                assert!(object.size >= HEADER_SIZE, "{}", info.name);
                assert_eq!(layout, Layout::REFERENCE, "{}", info.name);
            }
        }
    }

    #[test]
    fn layout_the_offsets_the_documents_name_are_these() {
        use crate::layout::{LENGTH_OFFSET, LIST_CAPACITY_OFFSET, PAIR_PAYLOAD_OFFSET};
        assert_eq!([WORD, HEADER_SIZE, LENGTH_OFFSET, ELEMENTS_OFFSET], [8, 8, 8, 16]);
        assert_eq!([LIST_CAPACITY_OFFSET, LIST_BUFFER_OFFSET, crate::layout::CLOSURE_CODE_OFFSET, CLOSURE_CAPTURES_OFFSET], [16, 24, 8, 16]);
        assert_eq!([BOX_VALUE_OFFSET, PAIR_PAYLOAD_OFFSET], [8, 8]);
    }

    #[test]
    fn kinds_each_kind_has_the_layout_the_table_says() {
        let mut table = TypeTable::new();
        let byte = scalar(&mut table, "Int8", Scalar::I8);
        let int = scalar(&mut table, "Int", Scalar::I64);
        let string = table.add("String", Kind::String);
        let unit = table.add("Null", Kind::Unit);
        let rows = [
            (unit, Layout::new(0, 1)),
            (byte, Layout::new(1, 1)),
            (table.add("S", Kind::Struct(vec![byte, int, byte])), Layout::new(24, 8)),
            (table.add("E", Kind::Enum(vec![vec![int], vec![]])), Layout::new(16, 8)),
            (table.add("Int?", Kind::Optional(int)), Layout::new(16, 8)),
            (table.add("String?", Kind::Optional(string)), Layout::new(8, 8)),
            (table.add("U", Kind::Pair), Layout::new(16, 8)),
            (table.add("[Int8; 5]", Kind::FixedArray(byte, 5)), Layout::new(5, 1)),
        ];
        for (id, layout) in rows {
            assert_eq!(table.layout(id), layout, "{}", table.get(id).name);
        }
        let objects = [
            table.add("C", Kind::Class(vec![int])),
            table.add("B", Kind::Box(int)),
            table.add("A", Kind::Array(int)),
            table.add("L", Kind::List(int)),
            table.add("F", Kind::Closure(vec![])),
            table.add("H", Kind::Handle(crate::handle::HandleKind::Port, vec![])),
            string,
        ];
        for id in objects {
            assert_eq!(table.layout(id), Layout::REFERENCE);
            assert!(table.get(id).kind.is_object());
        }
    }

    #[test]
    #[should_panic]
    fn kinds_a_value_type_cannot_name_itself() {
        let mut table = TypeTable::new();
        let next = TypeId::from_raw(table.len() as u32);
        table.add("S", Kind::Struct(vec![next]));
    }

    #[test]
    fn kinds_a_type_holds_what_typeof_and_the_collector_need() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let string = table.add("String", Kind::String);
        let shape = table.add("Shape", Kind::Enum(vec![vec![string], vec![int]]));
        let class = table.add("C", Kind::Class(vec![int, string]));
        let info = table.get(shape);
        assert_eq!(info.name, "Shape");
        assert!(matches!(info.kind, Kind::Enum(_)));
        assert_eq!(info.layout, Layout::new(16, 8));
        assert_eq!(info.variants.len(), 2);
        assert_eq!(info.slots.len(), 1);
        let info = table.get(class);
        assert_eq!(info.object.as_ref().unwrap().fields.len(), 2);
        assert_eq!(info.object.as_ref().unwrap().slots, [Slot::Ref(16)]);
        assert!(table.get(int).object.is_none());
    }

    #[test]
    fn enum_the_tag_is_one_two_or_four_bytes_by_the_variant_count() {
        let sizes: Vec<u32> = [0, 1, 256, 257, 65_536, 65_537].into_iter().map(enum_tag_size).collect();
        assert_eq!(sizes, [1, 1, 1, 2, 2, 4]);
    }

    #[test]
    fn enum_with_no_variants_has_no_size_and_the_tag_is_the_variants_position() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let never = table.add("Never", Kind::Enum(vec![]));
        assert_eq!(table.layout(never), Layout::ZERO);
        let id = table.add("E", Kind::Enum(vec![vec![], vec![int], vec![int, int]]));
        let counts: Vec<usize> = table.get(id).variants.iter().map(|variant| variant.fields.len()).collect();
        assert_eq!(counts, [0, 1, 2]);
    }

    #[test]
    fn optional_of_a_union_an_enum_or_a_struct_uses_the_tag_form() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let union = table.add("U", Kind::Pair);
        let shape = table.add("E", Kind::Enum(vec![vec![int]]));
        let point = table.add("P", Kind::Struct(vec![int, int]));
        for (inner, size) in [(union, 24), (shape, 24), (point, 24)] {
            let id = table.add("T?", Kind::Optional(inner));
            let info = table.get(id);
            assert_eq!(info.layout, Layout::new(size, 8));
            assert_eq!(info.variants.len(), 2);
            assert!(info.variants[0].fields.is_empty());
            assert_eq!(info.variants[1].fields[0].offset, 8);
            assert!(!info.is_reference());
        }
    }

    #[test]
    fn pair_payload_is_inline_a_reference_or_a_box_by_the_member_kind() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let float32 = scalar(&mut table, "Float32", Scalar::F32);
        let string = table.add("String", Kind::String);
        let maybe_string = table.add("String?", Kind::Optional(string));
        let maybe_int = table.add("Int?", Kind::Optional(int));
        let shape = table.add("E", Kind::Enum(vec![vec![int]]));
        let fixed = table.add("[Int; 2]", Kind::FixedArray(int, 2));
        let union = table.add("U", Kind::Pair);
        let rows = [
            (float32, PairPayload::Inline),
            (maybe_string, PairPayload::Reference),
            (maybe_int, PairPayload::Boxed),
            (shape, PairPayload::Boxed),
            (fixed, PairPayload::Boxed),
            (union, PairPayload::Boxed),
        ];
        for (id, payload) in rows {
            assert_eq!(table.pair_payload(id), payload, "{}", table.get(id).name);
        }
    }

    #[test]
    #[should_panic(expected = "array larger than 4 GiB")]
    fn fixed_array_over_4_gib_panics_when_the_type_is_added() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        table.add("[Int; many]", Kind::FixedArray(int, u32::MAX));
    }

    #[test]
    fn zero_a_zeroed_value_is_none_the_first_variant_null_or_the_none_pair() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let maybe = table.add("Int?", Kind::Optional(int));
        let shape = table.add("E", Kind::Enum(vec![vec![], vec![int]]));
        assert!(table.get(maybe).variants[0].fields.is_empty(), "tag 0 is None");
        assert!(table.get(shape).variants[0].fields.is_empty(), "tag 0 is the first variant");
        assert_eq!(TypeId::NONE.raw(), 0, "a zeroed pair names no type");
        assert_eq!(crate::layout::PAIR_PAYLOAD_OFFSET, 8, "the type word is the first word of the pair");
    }

    #[test]
    fn array_of_structs_is_one_block_of_inline_elements() {
        let mut table = TypeTable::new();
        let int = scalar(&mut table, "Int", Scalar::I64);
        let point = table.add("Point", Kind::Struct(vec![int, int]));
        let points = table.add("[Point]", Kind::Array(point));
        let list = table.add("List<Point>", Kind::List(point));
        let element = table.get(points).object.as_ref().unwrap().element.unwrap();
        assert_eq!((element.ty, element.stride, element.offset), (Some(point), 16, 16));
        assert_eq!(table.get(points).object.as_ref().unwrap().byte_size(3), 16 + 48);
        assert_eq!(table.get(list).object.as_ref().unwrap().size, 32);
    }

    #[test]
    #[should_panic(expected = "type is not a reserved object")]
    fn define_rejects_a_type_that_was_not_reserved() {
        let mut table = TypeTable::new();
        let string = table.add("String", Kind::String);
        table.define(string, Kind::String);
    }
}
