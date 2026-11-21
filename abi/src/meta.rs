use crate::bytes::{BytesError, Reader, Writer};
use crate::kind::TypeId;

/// What a type is to `std.meta`, in the order `std.meta.Kind` declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reflected {
    Null,
    Bool,
    Int,
    Float,
    String,
    Tuple,
    Array,
    List,
    Map,
    Set,
    Optional,
    Union,
    Struct,
    Class,
    Enum,
    Function,
    Handle,
}

impl Reflected {
    /// The position of a kind is its number in a byte form; new kinds go at the end.
    pub const ALL: [Reflected; 17] = [
        Reflected::Null,
        Reflected::Bool,
        Reflected::Int,
        Reflected::Float,
        Reflected::String,
        Reflected::Tuple,
        Reflected::Array,
        Reflected::List,
        Reflected::Map,
        Reflected::Set,
        Reflected::Optional,
        Reflected::Union,
        Reflected::Struct,
        Reflected::Class,
        Reflected::Enum,
        Reflected::Function,
        Reflected::Handle,
    ];

    pub fn tag(self) -> u8 {
        Reflected::ALL.iter().position(|kind| *kind == self).expect("every kind is listed") as u8
    }

    pub fn from_tag(tag: u8) -> Option<Reflected> {
        Reflected::ALL.get(usize::from(tag)).copied()
    }
}

/// How a variant carries its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carries {
    Nothing,
    Tuple,
    Named,
}

impl Carries {
    const ALL: [Carries; 3] = [Carries::Nothing, Carries::Tuple, Carries::Named];

    fn tag(self) -> u8 {
        Carries::ALL.iter().position(|known| *known == self).expect("every way is listed") as u8
    }
}

/// How the value of a type is written by `print`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// A type that nothing prints.
    Opaque,
    Null,
    Bool,
    Signed,
    Unsigned,
    Float32,
    Float64,
    Text,
    Tuple,
    FixedArray,
    Array,
    List,
    Optional,
    Union,
    Closure,
    Handle,
    Range,
    Map,
    Set,
    /// A struct or class: its name and the names of its fields.
    Record(String, Vec<String>),
    /// An enum: for each variant its name, how it carries its payload and the names of the payload.
    Enum(Vec<(String, Carries, Vec<String>)>),
}

const SHOWN_PLAIN: [Shown; 19] = [
    Shown::Opaque,
    Shown::Null,
    Shown::Bool,
    Shown::Signed,
    Shown::Unsigned,
    Shown::Float32,
    Shown::Float64,
    Shown::Text,
    Shown::Tuple,
    Shown::FixedArray,
    Shown::Array,
    Shown::List,
    Shown::Optional,
    Shown::Union,
    Shown::Closure,
    Shown::Handle,
    Shown::Range,
    Shown::Map,
    Shown::Set,
];
const SHOWN_RECORD: u8 = 19;
const SHOWN_ENUM: u8 = 20;

/// A type `std.meta` describes, with the numbers of the types it is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Described {
    pub kind: Reflected,
    pub ty: TypeId,
    /// The `Box` a `Dyn` holds a value of the type in.
    pub boxed: TypeId,
    /// The element type of a list, array or optional, or the members of a tuple.
    pub args: Vec<u32>,
    /// The fields of a struct or class.
    pub fields: Vec<u32>,
    /// The payload of each variant of an enum.
    pub variants: Vec<Vec<u32>>,
}

/// The program as the helpers of compiled code read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub string: TypeId,
    pub dyn_type: Option<TypeId>,
    /// Indexed by the raw `TypeId`.
    pub shown: Vec<Shown>,
    /// Indexed by the number `std.meta` knows a type by.
    pub described: Vec<Described>,
}

/// The first bytes of a meta table.
pub const MAGIC: &[u8; 8] = b"MOTEMETA";

/// The version of the byte form.
pub const VERSION: u32 = 1;

fn numbers(writer: &mut Writer, numbers: &[u32]) {
    writer.count(numbers.len());
    numbers.iter().for_each(|number| writer.u32(*number));
}

fn read_numbers(reader: &mut Reader) -> Result<Vec<u32>, BytesError> {
    (0..reader.count(4)?).map(|_| reader.u32()).collect()
}

fn texts(writer: &mut Writer, texts: &[String]) {
    writer.count(texts.len());
    texts.iter().for_each(|text| writer.text(text));
}

fn read_texts(reader: &mut Reader) -> Result<Vec<String>, BytesError> {
    (0..reader.count(4)?).map(|_| reader.text()).collect()
}

impl Meta {
    /// The magic, the version, the string type and the `Dyn` type (0 for none), then how each type is shown and each type `std.meta` describes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.raw(MAGIC);
        writer.u32(VERSION);
        writer.type_id(self.string);
        writer.u32(self.dyn_type.map_or(0, TypeId::raw));
        writer.count(self.shown.len());
        for shown in &self.shown {
            match shown {
                Shown::Record(name, fields) => {
                    writer.u8(SHOWN_RECORD);
                    writer.text(name);
                    texts(&mut writer, fields);
                }
                Shown::Enum(variants) => {
                    writer.u8(SHOWN_ENUM);
                    writer.count(variants.len());
                    for (name, carries, payload) in variants {
                        writer.text(name);
                        writer.u8(carries.tag());
                        texts(&mut writer, payload);
                    }
                }
                plain => writer.u8(SHOWN_PLAIN.iter().position(|known| known == plain).expect("every plain way is listed") as u8),
            }
        }
        writer.count(self.described.len());
        for entry in &self.described {
            writer.u8(entry.kind.tag());
            writer.type_id(entry.ty);
            writer.type_id(entry.boxed);
            numbers(&mut writer, &entry.args);
            numbers(&mut writer, &entry.fields);
            writer.count(entry.variants.len());
            entry.variants.iter().for_each(|variant| numbers(&mut writer, variant));
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
        let string = reader.type_id()?;
        let dyn_type = match reader.u32()? {
            0 => None,
            raw => Some(TypeId::from_raw(raw)),
        };
        let mut shown = Vec::new();
        for _ in 0..reader.count(1)? {
            shown.push(match reader.u8()? {
                SHOWN_RECORD => {
                    let name = reader.text()?;
                    Shown::Record(name, read_texts(&mut reader)?)
                }
                SHOWN_ENUM => {
                    let mut variants = Vec::new();
                    for _ in 0..reader.count(9)? {
                        let name = reader.text()?;
                        let carries = match reader.u8()? {
                            tag if usize::from(tag) < Carries::ALL.len() => Carries::ALL[usize::from(tag)],
                            tag => return Err(BytesError::Tag(tag)),
                        };
                        variants.push((name, carries, read_texts(&mut reader)?));
                    }
                    Shown::Enum(variants)
                }
                tag => SHOWN_PLAIN.get(usize::from(tag)).cloned().ok_or(BytesError::Tag(tag))?,
            });
        }
        let mut described = Vec::new();
        for _ in 0..reader.count(21)? {
            let tag = reader.u8()?;
            let kind = Reflected::from_tag(tag).ok_or(BytesError::Tag(tag))?;
            let (ty, boxed) = (reader.type_id()?, reader.type_id()?);
            let (args, fields) = (read_numbers(&mut reader)?, read_numbers(&mut reader)?);
            let variants = (0..reader.count(4)?).map(|_| read_numbers(&mut reader)).collect::<Result<Vec<_>, _>>()?;
            described.push(Described { kind, ty, boxed, args, fields, variants });
        }
        reader.end()?;
        if described.iter().flat_map(|entry| entry.args.iter().chain(&entry.fields).chain(entry.variants.iter().flatten())).any(|&number| number as usize >= described.len()) {
            return Err(BytesError::Invalid("a described type names one that is missing"));
        }
        Ok(Self { string, dyn_type, shown, described })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        let named = |kind, ty, args: &[u32], fields: &[u32], variants: &[&[u32]]| Described {
            kind,
            ty: TypeId::from_raw(ty),
            boxed: TypeId::from_raw(ty + 1),
            args: args.to_vec(),
            fields: fields.to_vec(),
            variants: variants.iter().map(|variant| variant.to_vec()).collect(),
        };
        Meta {
            string: TypeId::from_raw(3),
            dyn_type: Some(TypeId::from_raw(9)),
            shown: vec![
                Shown::Opaque,
                Shown::Signed,
                Shown::Text,
                Shown::Record("Point".into(), vec!["x".into(), "y".into()]),
                Shown::Enum(vec![("Full".into(), Carries::Nothing, vec![]), ("Missing".into(), Carries::Tuple, vec![String::new()]), ("At".into(), Carries::Named, vec!["row".into(), "col".into()])]),
                Shown::Map,
            ],
            described: vec![named(Reflected::Int, 4, &[], &[], &[]), named(Reflected::Struct, 6, &[], &[0, 0], &[]), named(Reflected::Enum, 8, &[], &[], &[&[], &[0], &[0, 1]]), named(Reflected::List, 10, &[1], &[], &[])],
        }
    }

    #[test]
    fn meta_a_table_is_the_magic_the_version_the_two_types_then_the_ways_to_show_and_the_described() {
        let written = Meta { string: TypeId::from_raw(3), dyn_type: None, shown: vec![Shown::Bool, Shown::Record("P".into(), vec![])], described: vec![] }.to_bytes();
        let mut expected = b"MOTEMETA".to_vec();
        expected.extend([1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 2, 19, 1, 0, 0, 0, b'P', 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(written, expected);
    }

    #[test]
    fn meta_a_table_reads_back_equal_and_gives_one_byte_string() {
        let bytes = meta().to_bytes();
        let read = Meta::from_bytes(&bytes).unwrap();
        assert_eq!(read, meta());
        assert_eq!(read.to_bytes(), bytes);
    }

    #[test]
    fn meta_every_plain_way_to_show_reads_back() {
        let meta = Meta { string: TypeId::from_raw(1), dyn_type: None, shown: SHOWN_PLAIN.to_vec(), described: vec![] };
        assert_eq!(Meta::from_bytes(&meta.to_bytes()), Ok(meta));
    }

    #[test]
    fn meta_every_kind_has_a_tag_that_reads_back() {
        for (at, kind) in Reflected::ALL.iter().enumerate() {
            assert_eq!(usize::from(kind.tag()), at);
            assert_eq!(Reflected::from_tag(kind.tag()), Some(*kind));
        }
        assert_eq!(Reflected::from_tag(17), None);
    }

    #[test]
    fn meta_a_bad_table_is_refused() {
        let bytes = meta().to_bytes();
        assert_eq!(Meta::from_bytes(b"MOTECODE\x01\0\0\0"), Err(BytesError::Magic));
        let mut other = bytes.clone();
        other[8] = 2;
        assert_eq!(Meta::from_bytes(&other), Err(BytesError::Version(2)));
        let mut more = bytes.clone();
        more.push(0);
        assert_eq!(Meta::from_bytes(&more), Err(BytesError::Trailing));
        for cut in 0..bytes.len() {
            assert!(Meta::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut dangling = meta();
        dangling.described[3].args = vec![4];
        assert_eq!(Meta::from_bytes(&dangling.to_bytes()), Err(BytesError::Invalid("a described type names one that is missing")));
        let mut unknown = Meta { string: TypeId::from_raw(1), dyn_type: None, shown: vec![Shown::Bool], described: vec![] }.to_bytes();
        let last = unknown.len() - 5;
        unknown[last] = 99;
        assert_eq!(Meta::from_bytes(&unknown), Err(BytesError::Tag(99)));
    }
}
