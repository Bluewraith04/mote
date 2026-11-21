use crate::bytes::{BytesError, Reader, Writer};
use crate::handle::HandleKind;
use crate::kind::{Kind, TypeId};
use crate::reflect::Reflection;
use crate::scalar::Scalar;
use crate::table::TypeTable;

/// The first bytes of a type table.
pub const MAGIC: &[u8; 8] = b"MOTETYPE";

/// The version of the byte form.
pub const VERSION: u32 = 1;

/// The most bytes a value or object may take; a table that asks for more is refused.
const LIMIT: u64 = 1 << 31;

const UNIT: u8 = 0;
const SCALAR: u8 = 1;
const STRUCT: u8 = 2;
const ENUM: u8 = 3;
const OPTIONAL: u8 = 4;
const PAIR: u8 = 5;
const FIXED_ARRAY: u8 = 6;
const CLASS: u8 = 7;
const BOX: u8 = 8;
const STRING: u8 = 9;
const ARRAY: u8 = 10;
const LIST: u8 = 11;
const CLOSURE: u8 = 12;
const HANDLE: u8 = 13;

/// A type table and the reflection data that travel with it.
#[derive(Debug, PartialEq)]
pub struct TypeImage {
    pub table: TypeTable,
    pub reflection: Reflection,
}

impl TypeImage {
    pub fn new(table: TypeTable) -> Self {
        Self { table, reflection: Reflection::default() }
    }

    /// The magic, the version, the types after `NONE` in id order, then the reflection section.
    pub fn to_bytes(&self) -> Result<Vec<u8>, BytesError> {
        let mut writer = Writer::new();
        writer.raw(MAGIC);
        writer.u32(VERSION);
        writer.count(self.table.len() - 1);
        for (_, info) in self.table.iter().skip(1) {
            writer.text(&info.name);
            write_kind(&mut writer, &info.kind).map_err(|()| BytesError::Invalid("a type is reserved and not defined"))?;
        }
        self.reflection.write(&mut writer);
        Ok(writer.finish())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BytesError> {
        let mut reader = Reader::new(bytes);
        reader.expect(MAGIC)?;
        match reader.u32()? {
            VERSION => {}
            other => return Err(BytesError::Version(other)),
        }
        let count = reader.count(5)?;
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            let name = reader.text()?;
            records.push((name, read_kind(&mut reader)?));
        }
        let reflection = Reflection::read(&mut reader)?;
        reader.end()?;
        Ok(Self { table: build(records)?, reflection })
    }
}

fn write_kind(writer: &mut Writer, kind: &Kind) -> Result<(), ()> {
    match kind {
        Kind::Unit => writer.u8(UNIT),
        Kind::Scalar(scalar) => {
            writer.u8(SCALAR);
            writer.u8(scalar.tag());
        }
        Kind::Struct(types) => ids(writer, STRUCT, types),
        Kind::Enum(variants) => {
            writer.u8(ENUM);
            writer.count(variants.len());
            for variant in variants {
                writer.count(variant.len());
                variant.iter().for_each(|ty| writer.type_id(*ty));
            }
        }
        Kind::Optional(inner) => one(writer, OPTIONAL, *inner),
        Kind::Pair => writer.u8(PAIR),
        Kind::FixedArray(element, count) => {
            writer.u8(FIXED_ARRAY);
            writer.type_id(*element);
            writer.u32(*count);
        }
        Kind::Class(types) => ids(writer, CLASS, types),
        Kind::Box(inner) => one(writer, BOX, *inner),
        Kind::String => writer.u8(STRING),
        Kind::Array(element) => one(writer, ARRAY, *element),
        Kind::List(element) => one(writer, LIST, *element),
        Kind::Closure(types) => ids(writer, CLOSURE, types),
        Kind::Handle(handle, payload) => {
            writer.u8(HANDLE);
            writer.u8(HandleKind::ALL.iter().position(|known| known == handle).expect("every handle kind is listed") as u8);
            writer.count(payload.len());
            payload.iter().for_each(|ty| writer.type_id(*ty));
        }
        Kind::Reserved => return Err(()),
    }
    Ok(())
}

fn ids(writer: &mut Writer, tag: u8, types: &[TypeId]) {
    writer.u8(tag);
    writer.count(types.len());
    types.iter().for_each(|ty| writer.type_id(*ty));
}

fn one(writer: &mut Writer, tag: u8, ty: TypeId) {
    writer.u8(tag);
    writer.type_id(ty);
}

fn read_ids(reader: &mut Reader) -> Result<Vec<TypeId>, BytesError> {
    (0..reader.count(4)?).map(|_| reader.type_id()).collect()
}

fn read_kind(reader: &mut Reader) -> Result<Kind, BytesError> {
    Ok(match reader.u8()? {
        UNIT => Kind::Unit,
        SCALAR => Kind::Scalar(Scalar::from_tag(reader.u8()?).ok_or(BytesError::Invalid("a scalar tag names nothing"))?),
        STRUCT => Kind::Struct(read_ids(reader)?),
        ENUM => Kind::Enum((0..reader.count(4)?).map(|_| read_ids(reader)).collect::<Result<_, _>>()?),
        OPTIONAL => Kind::Optional(reader.type_id()?),
        PAIR => Kind::Pair,
        FIXED_ARRAY => Kind::FixedArray(reader.type_id()?, reader.u32()?),
        CLASS => Kind::Class(read_ids(reader)?),
        BOX => Kind::Box(reader.type_id()?),
        STRING => Kind::String,
        ARRAY => Kind::Array(reader.type_id()?),
        LIST => Kind::List(reader.type_id()?),
        CLOSURE => Kind::Closure(read_ids(reader)?),
        HANDLE => {
            let handle = *HandleKind::ALL.get(usize::from(reader.u8()?)).ok_or(BytesError::Invalid("a handle tag names nothing"))?;
            Kind::Handle(handle, read_ids(reader)?)
        }
        tag => return Err(BytesError::Tag(tag)),
    })
}

/// The types a kind names.
fn members(kind: &Kind) -> Vec<TypeId> {
    match kind {
        Kind::Struct(types) | Kind::Class(types) | Kind::Closure(types) | Kind::Handle(_, types) => types.clone(),
        Kind::Enum(variants) => variants.iter().flatten().copied().collect(),
        Kind::Optional(ty) | Kind::Box(ty) | Kind::Array(ty) | Kind::List(ty) | Kind::FixedArray(ty, _) => vec![*ty],
        Kind::Unit | Kind::Scalar(_) | Kind::Pair | Kind::String | Kind::Reserved => Vec::new(),
    }
}

/// Reserves each object type, adds each value type, then defines the objects, so the sizes come out as they did when the table was made.
fn build(records: Vec<(String, Kind)>) -> Result<TypeTable, BytesError> {
    let total = records.len() + 1;
    let mut table = TypeTable::new();
    let mut objects = Vec::new();
    for (name, kind) in records {
        let id = TypeId::from_raw(table.len() as u32);
        let named = members(&kind);
        if named.iter().any(|ty| ty.index() == 0 || ty.index() >= total) {
            return Err(BytesError::Invalid("a type names one that is not in the table"));
        }
        if let Kind::Handle(handle, payload) = &kind
            && payload.len() != handle.payloads()
        {
            return Err(BytesError::Invalid("a handle lists the wrong number of payloads"));
        }
        if kind.is_object() {
            table.reserve_object(name);
            objects.push((id, kind));
            continue;
        }
        if named.iter().any(|ty| ty.index() >= id.index()) {
            return Err(BytesError::Invalid("a value type names one that follows it"));
        }
        if too_big(&table, &kind) {
            return Err(BytesError::Invalid("a type is larger than the format allows"));
        }
        table.add(name, kind);
    }
    for (id, kind) in objects {
        if too_big_object(&table, &kind) {
            return Err(BytesError::Invalid("a type is larger than the format allows"));
        }
        table.define(id, kind);
    }
    Ok(table)
}

fn size_of_all(table: &TypeTable, types: impl Iterator<Item = TypeId>) -> u64 {
    types.map(|ty| u64::from(table.layout(ty).size) + 8).sum()
}

fn too_big(table: &TypeTable, kind: &Kind) -> bool {
    match kind {
        Kind::FixedArray(element, count) => {
            let layout = table.layout(*element);
            u64::from(layout.size.next_multiple_of(layout.align)) * u64::from(*count) >= LIMIT
        }
        Kind::Enum(variants) => variants.iter().any(|variant| size_of_all(table, variant.iter().copied()) >= LIMIT),
        other => size_of_all(table, members(other).into_iter()) >= LIMIT,
    }
}

fn too_big_object(table: &TypeTable, kind: &Kind) -> bool {
    size_of_all(table, members(kind).into_iter().filter(|ty| table.get(*ty).kind != Kind::Reserved)) >= LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{Slot, TypeId};
    use crate::passing::passing;
    use crate::sample::everything;

    #[test]
    fn table_bytes_start_with_the_magic_and_the_version() {
        let bytes = TypeImage::new(TypeTable::new()).to_bytes().unwrap();
        assert_eq!(&bytes[..8], b"MOTETYPE");
        assert_eq!(bytes[8..12], [1, 0, 0, 0]);
        assert_eq!(bytes[12..16], [0, 0, 0, 0]);
        assert_eq!(bytes.len(), 16 + 8);
    }

    #[test]
    fn table_bytes_one_type_is_its_name_its_kind_tag_and_its_members() {
        let mut table = TypeTable::new();
        table.add("Int", Kind::Scalar(Scalar::I64));
        let bytes = TypeImage::new(table).to_bytes().unwrap();
        assert_eq!(bytes[12..16], [1, 0, 0, 0]);
        assert_eq!(bytes[16..], [3, 0, 0, 0, b'I', b'n', b't', 1, 3, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn table_bytes_every_kind_reads_back_to_an_equal_table() {
        let image = everything();
        let bytes = image.to_bytes().unwrap();
        let read = TypeImage::from_bytes(&bytes).unwrap();
        assert_eq!(read, image);
    }

    #[test]
    fn table_bytes_the_names_and_the_slots_read_back_for_every_type() {
        let image = everything();
        let read = TypeImage::from_bytes(&image.to_bytes().unwrap()).unwrap();
        for ((id, before), (_, after)) in image.table.iter().zip(read.table.iter()) {
            assert_eq!(before.name, after.name);
            assert_eq!(before.slots, after.slots, "{}", before.name);
            assert_eq!(before.object, after.object, "{}", before.name);
            assert_eq!(passing(&image.table, id), passing(&read.table, id), "{}", before.name);
        }
        let tree = read.table.iter().find(|(_, info)| info.name == "Tree").unwrap().0;
        assert_eq!(read.table.get(tree).slots, [Slot::Enum { offset: 0, ty: tree }]);
    }

    #[test]
    fn table_bytes_one_table_gives_one_byte_string() {
        let first = everything().to_bytes().unwrap();
        assert_eq!(everything().to_bytes().unwrap(), first);
        let read = TypeImage::from_bytes(&first).unwrap();
        assert_eq!(read.to_bytes().unwrap(), first);
    }

    #[test]
    fn table_bytes_the_reflection_section_travels_with_the_types() {
        let image = everything();
        let read = TypeImage::from_bytes(&image.to_bytes().unwrap()).unwrap();
        assert_eq!(read.reflection, image.reflection);
    }

    #[test]
    fn table_bytes_a_type_that_is_reserved_and_not_defined_is_not_written() {
        let mut table = TypeTable::new();
        table.reserve_object("Node");
        assert_eq!(TypeImage::new(table).to_bytes(), Err(BytesError::Invalid("a type is reserved and not defined")));
    }

    #[test]
    fn table_bytes_a_wrong_magic_or_version_is_refused() {
        let mut bytes = everything().to_bytes().unwrap();
        bytes[0] = b'X';
        assert_eq!(TypeImage::from_bytes(&bytes), Err(BytesError::Magic));
        let mut bytes = everything().to_bytes().unwrap();
        bytes[8] = 2;
        assert_eq!(TypeImage::from_bytes(&bytes), Err(BytesError::Version(2)));
        assert_eq!(TypeImage::from_bytes(&[]), Err(BytesError::Magic));
    }

    #[test]
    fn table_bytes_every_cut_of_a_table_is_refused_and_none_panics() {
        let bytes = everything().to_bytes().unwrap();
        for cut in 0..bytes.len() {
            assert!(TypeImage::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut long = bytes;
        long.push(0);
        assert_eq!(TypeImage::from_bytes(&long), Err(BytesError::Trailing));
    }

    #[test]
    fn table_bytes_a_type_that_names_a_missing_or_later_value_is_refused() {
        let mut missing = Writer::new();
        missing.raw(MAGIC);
        missing.u32(VERSION);
        missing.count(1);
        missing.text("S");
        missing.u8(STRUCT);
        missing.count(1);
        missing.type_id(TypeId::from_raw(9));
        missing.count(0);
        missing.count(0);
        assert!(TypeImage::from_bytes(&missing.finish()).is_err());

        let mut later = Writer::new();
        later.raw(MAGIC);
        later.u32(VERSION);
        later.count(2);
        later.text("A");
        later.u8(OPTIONAL);
        later.type_id(TypeId::from_raw(2));
        later.text("B");
        later.u8(UNIT);
        later.count(0);
        later.count(0);
        assert_eq!(TypeImage::from_bytes(&later.finish()), Err(BytesError::Invalid("a value type names one that follows it")));
    }

    #[test]
    fn table_bytes_a_handle_with_the_wrong_payload_count_and_a_huge_array_are_refused() {
        let mut handle = Writer::new();
        handle.raw(MAGIC);
        handle.u32(VERSION);
        handle.count(1);
        handle.text("M");
        handle.u8(HANDLE);
        handle.u8(4);
        handle.count(0);
        handle.count(0);
        handle.count(0);
        assert_eq!(TypeImage::from_bytes(&handle.finish()), Err(BytesError::Invalid("a handle lists the wrong number of payloads")));

        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let mut bytes = TypeImage::new(table).to_bytes().unwrap();
        let tail = bytes.split_off(bytes.len() - 8);
        bytes[12] = 2;
        let mut writer = Writer::new();
        writer.text("Big");
        writer.u8(FIXED_ARRAY);
        writer.type_id(int);
        writer.u32(u32::MAX);
        bytes.extend(writer.finish());
        bytes.extend(tail);
        assert_eq!(TypeImage::from_bytes(&bytes), Err(BytesError::Invalid("a type is larger than the format allows")));
    }
}
