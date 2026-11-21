//! The table `std.meta` reads: the described types of a program as two flat arrays.

use std::collections::HashMap;

use mote_semantics::{Kind, Program, Shape, TyId};

/// The sections of [`Table::ints`], in the order `std/src/meta.mote` reads them. The array starts with `COUNT` base offsets, one per section, then the number of types.
pub mod section {
    pub const KIND: usize = 0;
    pub const NAME: usize = 1;
    pub const DOC: usize = 2;
    pub const ARG_START: usize = 3;
    pub const ARG_COUNT: usize = 4;
    pub const GENERIC_START: usize = 5;
    pub const GENERIC_COUNT: usize = 6;
    pub const TRAIT_START: usize = 7;
    pub const TRAIT_COUNT: usize = 8;
    pub const METHOD_START: usize = 9;
    pub const METHOD_COUNT: usize = 10;
    pub const ATTR_START: usize = 11;
    pub const ATTR_COUNT: usize = 12;
    pub const VARIANT_START: usize = 13;
    pub const VARIANT_COUNT: usize = 14;
    pub const FIELD_START: usize = 15;
    pub const FIELD_COUNT: usize = 16;
    /// Type numbers.
    pub const ARGS: usize = 17;
    /// Text numbers.
    pub const GENERICS: usize = 18;
    /// Text numbers.
    pub const TRAITS: usize = 19;
    /// Rows of name, signature and flags.
    pub const METHODS: usize = 20;
    /// Rows of name, first argument row and argument count.
    pub const ATTRS: usize = 21;
    /// Rows of key and value.
    pub const ATTR_ARGS: usize = 22;
    /// Rows of name, doc, shape, has discriminant, discriminant, first field row and field count.
    pub const VARIANTS: usize = 23;
    /// Rows of name, type number, flags and doc.
    pub const FIELDS: usize = 24;
    /// Text numbers; one per type, like the sections above `ARGS`.
    pub const SHORT: usize = 25;
    pub const COUNT: usize = 26;
}

pub const METHOD_ROW: usize = 3;
pub const ATTR_ROW: usize = 3;
pub const ATTR_ARG_ROW: usize = 2;
pub const VARIANT_ROW: usize = 7;
pub const FIELD_ROW: usize = 4;

/// The described types as numbers and text; a text is named by its position in `texts`, and the first is empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub ints: Vec<i128>,
    pub texts: Vec<String>,
}

impl From<Table> for mote_abi::Reflection {
    fn from(table: Table) -> Self {
        Self { ints: table.ints, texts: table.texts }
    }
}

impl From<mote_abi::Reflection> for Table {
    fn from(reflection: mote_abi::Reflection) -> Self {
        Self { ints: reflection.ints, texts: reflection.texts }
    }
}

#[derive(Default)]
struct Texts {
    all: Vec<String>,
    found: HashMap<String, i128>,
}

impl Texts {
    fn at(&mut self, text: &str) -> i128 {
        if let Some(&found) = self.found.get(text) {
            return found;
        }
        let at = self.all.len() as i128;
        self.all.push(text.to_string());
        self.found.insert(text.to_string(), at);
        at
    }
}

/// The table for every type the program described.
pub fn encode(program: &Program) -> Table {
    let mut texts = Texts::default();
    texts.at("");
    let mut sections: Vec<Vec<i128>> = vec![Vec::new(); section::COUNT];
    let number = |ty: TyId| i128::from(program.reflect_index[&ty]);
    for entry in &program.reflected {
        sections[section::KIND].push(kind_code(entry.kind));
        sections[section::NAME].push(texts.at(&entry.name));
        sections[section::DOC].push(texts.at(&entry.doc));
        sections[section::SHORT].push(texts.at(&entry.short));
        let counts = [
            (section::ARG_START, section::ARG_COUNT, section::ARGS, entry.args.len()),
            (section::GENERIC_START, section::GENERIC_COUNT, section::GENERICS, entry.generics.len()),
            (section::TRAIT_START, section::TRAIT_COUNT, section::TRAITS, entry.traits.len()),
            (section::METHOD_START, section::METHOD_COUNT, section::METHODS, entry.methods.len()),
            (section::ATTR_START, section::ATTR_COUNT, section::ATTRS, entry.attributes.len()),
            (section::VARIANT_START, section::VARIANT_COUNT, section::VARIANTS, entry.variants.len()),
            (section::FIELD_START, section::FIELD_COUNT, section::FIELDS, entry.fields.len()),
        ];
        let rows = [1, 1, 1, METHOD_ROW, ATTR_ROW, VARIANT_ROW, FIELD_ROW];
        for ((start, count, body, length), row) in counts.into_iter().zip(rows) {
            let first = (sections[body].len() / row) as i128;
            sections[start].push(first);
            sections[count].push(length as i128);
        }
        for &arg in &entry.args {
            sections[section::ARGS].push(number(arg));
        }
        for name in &entry.generics {
            let at = texts.at(name);
            sections[section::GENERICS].push(at);
        }
        for name in &entry.traits {
            let at = texts.at(name);
            sections[section::TRAITS].push(at);
        }
        for method in &entry.methods {
            let row = [texts.at(&method.name), texts.at(&method.signature), method.flags];
            sections[section::METHODS].extend(row);
        }
        for attribute in &entry.attributes {
            let first = (sections[section::ATTR_ARGS].len() / ATTR_ARG_ROW) as i128;
            for (key, value) in &attribute.args {
                let row = [texts.at(key), texts.at(value)];
                sections[section::ATTR_ARGS].extend(row);
            }
            let row = [texts.at(&attribute.name), first, attribute.args.len() as i128];
            sections[section::ATTRS].extend(row);
        }
        for field in &entry.fields {
            let row = [texts.at(&field.name), number(field.ty), field.flags, texts.at(&field.doc)];
            sections[section::FIELDS].extend(row);
        }
        for variant in &entry.variants {
            let first = (sections[section::FIELDS].len() / FIELD_ROW) as i128;
            for field in &variant.fields {
                let row = [texts.at(&field.name), number(field.ty), field.flags, texts.at(&field.doc)];
                sections[section::FIELDS].extend(row);
            }
            let row = [
                texts.at(&variant.name),
                texts.at(&variant.doc),
                shape_code(variant.shape),
                i128::from(variant.discriminant.is_some()),
                variant.discriminant.unwrap_or(0),
                first,
                variant.fields.len() as i128,
            ];
            sections[section::VARIANTS].extend(row);
        }
    }
    let header = section::COUNT + 1;
    let mut ints = Vec::with_capacity(header + sections.iter().map(Vec::len).sum::<usize>());
    let mut base = header as i128;
    for body in &sections {
        ints.push(base);
        base += body.len() as i128;
    }
    ints.push(program.reflected.len() as i128);
    for body in sections {
        ints.extend(body);
    }
    Table { ints, texts: texts.all }
}

/// The position of the kind in `std.meta.Kind`.
pub fn kind_code(kind: Kind) -> i128 {
    kind as i128
}

/// The position of the shape in `std.meta.Shape`.
pub fn shape_code(shape: Shape) -> i128 {
    shape as i128
}

#[cfg(test)]
mod tests {
    use mote_semantics::compile;

    use super::*;

    const DESCRIBES_P: &str = "import { describe } from std.meta\nstruct P {\n x: Int\n}\nfn f() -> String { return describe<P>().name() }";

    #[test]
    fn meta_table_has_the_header_the_section_list_says() {
        let compiled = compile(DESCRIBES_P);
        assert_eq!(compiled.errors, Vec::<String>::new());
        let table = encode(&compiled.program);
        let count = compiled.program.reflected.len() as i128;
        assert!(count >= 2, "P and Int are described");
        assert_eq!(table.ints[section::COUNT], count);
        assert_eq!(table.texts[0], "");
        let bases = &table.ints[..section::COUNT];
        assert_eq!(bases[0], section::COUNT as i128 + 1);
        assert!(bases.windows(2).all(|pair| pair[0] <= pair[1]));
        let per_type = [section::KIND, section::NAME, section::DOC, section::ARG_COUNT, section::FIELD_COUNT];
        for index in per_type {
            assert_eq!(bases[index + 1] - bases[index], count, "section {index}");
        }
        let fields = bases[section::SHORT] - bases[section::FIELDS];
        assert_eq!(fields % FIELD_ROW as i128, 0);
        assert!(fields > 0, "P has a field");
        assert_eq!(table.ints.len() as i128 - bases[section::SHORT], count, "one declared name per type");
    }

    #[test]
    fn meta_type_numbers_are_the_positions_in_the_table() {
        let compiled = compile(DESCRIBES_P);
        assert_eq!(compiled.errors, Vec::<String>::new());
        let table = encode(&compiled.program);
        for (&ty, &number) in &compiled.program.reflect_index {
            let entry = &compiled.program.reflected[number as usize];
            assert_eq!(entry.ty, ty);
            let name = table.ints[table.ints[section::NAME] as usize + number as usize] as usize;
            assert_eq!(table.texts[name], entry.name);
        }
    }

    #[test]
    fn meta_type_id_is_a_constant_in_the_lowered_program() {
        let mut compiled = compile(DESCRIBES_P);
        assert_eq!(compiled.errors, Vec::<String>::new());
        let lowered = crate::lower(&mut compiled.program, &mut compiled.interner);
        assert_eq!(lowered.unsupported, []);
        let mut describes = 0;
        for (_, function) in lowered.module.functions.iter() {
            for (_, block) in function.blocks.iter() {
                for inst in &block.insts {
                    if let crate::Op::Call { callee: crate::Callee::Intrinsic(name), .. } = &inst.op {
                        assert_ne!(compiled.interner.resolve(*name), "type_id", "a call to type_id is left");
                    }
                    if let crate::Op::Record(fields) = &inst.op
                        && fields.iter().any(|field| matches!(field, crate::Operand::Const(crate::Const::Int(_))))
                    {
                        describes += 1;
                    }
                }
            }
        }
        assert!(describes > 0, "describe builds a Type from a constant");
    }

    #[test]
    fn reflection_a_real_table_goes_through_the_type_table_bytes_and_comes_back_equal() {
        let compiled = compile(DESCRIBES_P);
        assert_eq!(compiled.errors, Vec::<String>::new());
        let table = encode(&compiled.program);
        assert!(table.ints.len() > section::COUNT && table.texts.len() > 1);
        let mut types = mote_abi::TypeTable::new();
        types.add("Int", mote_abi::Kind::Scalar(mote_abi::Scalar::I64));
        let image = mote_abi::TypeImage { table: types, reflection: table.clone().into() };
        let bytes = image.to_bytes().unwrap();
        let read = mote_abi::TypeImage::from_bytes(&bytes).unwrap();
        assert_eq!(Table::from(read.reflection), table);
    }

    #[test]
    fn meta_a_program_that_never_describes_has_an_empty_table() {
        let compiled = compile("fn f() -> Int { return 1 }");
        assert_eq!(compiled.errors, Vec::<String>::new());
        assert!(compiled.program.reflected.is_empty());
        assert_eq!(encode(&compiled.program).ints.len(), section::COUNT + 1);
    }
}
