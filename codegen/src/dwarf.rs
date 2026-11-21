//! The DWARF line tables of an object file, made from its debug table.

use cranelift_object::ObjectProduct;
use gimli::write::{Address, AttributeValue, EndianVec, LineProgram, LineString, Range, RangeList, Relocation, RelocationTarget, RelocateWriter, Sections, DwarfUnit};
use gimli::{constants, Encoding, Format, LineEncoding, LittleEndian};
use mote_abi::DebugTable;
use object::write::SymbolId;
use object::{RelocationEncoding, RelocationFlags, RelocationKind, SectionKind};

use crate::CompileError;

/// A section as it is written, with the places that need a symbol's address.
#[derive(Clone)]
struct Section {
    data: EndianVec<LittleEndian>,
    relocations: Vec<Relocation>,
}

impl Default for Section {
    fn default() -> Self {
        Section { data: EndianVec::new(LittleEndian), relocations: Vec::new() }
    }
}

impl RelocateWriter for Section {
    type Writer = EndianVec<LittleEndian>;

    fn writer(&self) -> &Self::Writer {
        &self.data
    }

    fn writer_mut(&mut self) -> &mut Self::Writer {
        &mut self.data
    }

    fn relocate(&mut self, relocation: Relocation) {
        self.relocations.push(relocation);
    }
}

fn address(function: usize) -> Address {
    Address::Symbol { symbol: function, addend: 0 }
}

/// Adds `.debug_line` and the sections it needs to the object; `functions` holds the symbol of each function of the table.
pub(crate) fn add_line_tables(product: &mut ObjectProduct, table: &DebugTable, functions: &[SymbolId]) -> Result<(), CompileError> {
    let encoding = Encoding { format: Format::Dwarf32, version: 4, address_size: 8 };
    let mut unit = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(encoding, LineEncoding::default(), LineString::String(b".".to_vec()), LineString::String(b"mote".to_vec()), None);
    let directory = program.default_directory();
    let files: Vec<_> = table.files.iter().map(|name| program.add_file(LineString::String(name.clone().into_bytes()), directory, None)).collect();
    let mut ranges = Vec::new();
    for (at, function) in table.functions.iter().enumerate() {
        if function.rows.is_empty() {
            continue;
        }
        program.begin_sequence(Some(address(at)));
        for &(offset, place) in &function.rows {
            let place = &table.places[place as usize];
            let row = program.row();
            row.address_offset = u64::from(offset);
            row.file = files[place.file as usize];
            row.line = u64::from(place.line);
            row.column = u64::from(place.column);
            program.generate_row();
        }
        program.end_sequence(u64::from(function.size));
        ranges.push(Range::StartLength { begin: address(at), length: u64::from(function.size) });
    }
    unit.unit.line_program = program;
    let ranges = unit.unit.ranges.add(RangeList(ranges));
    let root = unit.unit.root();
    let root = unit.unit.get_mut(root);
    root.set(constants::DW_AT_producer, AttributeValue::String(b"mote".to_vec()));
    root.set(constants::DW_AT_name, AttributeValue::String(b"mote".to_vec()));
    root.set(constants::DW_AT_low_pc, AttributeValue::Address(Address::Constant(0)));
    root.set(constants::DW_AT_ranges, AttributeValue::RangeListRef(ranges));

    let mut sections = Sections::new(Section::default());
    unit.write(&mut sections).map_err(CompileError::internal)?;

    let mut ids = Vec::new();
    sections
        .for_each(|id, section| {
            if !section.data.slice().is_empty() {
                let object = &mut product.object;
                let section_id = object.add_section(Vec::new(), id.name().as_bytes().to_vec(), SectionKind::Debug);
                object.set_section_data(section_id, section.data.slice().to_vec(), 1);
                ids.push((id, section_id));
            }
            Ok::<(), CompileError>(())
        })
        .map_err(CompileError::internal)?;
    let mut by_id = ids.iter();
    sections
        .for_each(|id, section| {
            if section.data.slice().is_empty() {
                return Ok::<(), CompileError>(());
            }
            let &(_, section_id) = by_id.find(|(found, _)| *found == id).expect("a section was added");
            for relocation in &section.relocations {
                let (symbol, addend) = match relocation.target {
                    RelocationTarget::Symbol(index) => (functions[index], relocation.addend),
                    RelocationTarget::Section(target) => {
                        let &(_, target) = ids.iter().find(|(found, _)| *found == target).expect("a section was added");
                        (product.object.section_symbol(target), relocation.addend)
                    }
                };
                let flags = RelocationFlags::Generic { kind: RelocationKind::Absolute, encoding: RelocationEncoding::Generic, size: relocation.size * 8 };
                product.object.add_relocation(section_id, object::write::Relocation { offset: relocation.offset as u64, symbol, addend, flags }).map_err(CompileError::internal)?;
            }
            Ok(())
        })
        .map_err(CompileError::internal)?;
    Ok(())
}
