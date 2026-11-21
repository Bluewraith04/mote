use std::collections::HashSet;

use ::object::{Architecture, BinaryFormat, File, Object, ObjectKind, ObjectSection, ObjectSymbol, RelocationTarget, SectionIndex};
use mote::{Options, Pipeline, Target};
use mote_abi::{Entry, FaultKind, FaultTable, Slot, TypeImage};
use mote_packages::testing::Scratch;

use super::*;
use mote_runtime::{helpers, loader};

use crate::typetable;

fn built(source: &str) -> Pipeline {
    let scratch = Scratch::new();
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    Pipeline::build(&dir, Target::Run, &Options { optimize: false, ..Options::default() }).unwrap_or_else(|failure| panic!("{failure}"))
}

fn object_of(source: &str) -> Vec<u8> {
    let mut pipeline = built(source);
    compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, Vec::new(), &|_| None).unwrap_or_else(|error| panic!("{error}"))
}

const CHECKS: &str = "fn grow(a: Int8, b: Float32) -> Int8 {\n    if b > 1.5 {\n        return a\n    }\n    return a + 1\n}\n\nfn main() {\n    let n = 4\n    println(grow(1, 2.0))\n    println(100 / n)\n    println(1 << n)\n    println(n * n)\n}\n";

const HELD: &str = "fn pick(names: List<String>, at: Int) -> String {\n    let first = names[0]\n    println(at)\n    return if at == 0 { first } else { names[at] }\n}\n\nfn main() {\n    var names: List<String> = List.new()\n    names.push(\"a\")\n    names.push(\"b\")\n    println(pick(names, 1))\n}\n";

const GLOBAL: &str = "var greeting = \"hello\"\n\nfn main() {\n    println(greeting)\n}\n";

/// The place a relocation of the section at the offset refers to: its section and the offset in it.
fn target(file: &File, section: SectionIndex, offset: u64) -> (SectionIndex, u64) {
    let (_, relocation) = file.section_by_index(section).unwrap().relocations().find(|(at, _)| *at == offset).unwrap_or_else(|| panic!("no relocation at {offset}"));
    match relocation.target() {
        RelocationTarget::Symbol(index) => {
            let symbol = file.symbol_by_index(index).unwrap();
            (symbol.section_index().expect("a defined symbol"), symbol.address().wrapping_add_signed(relocation.addend()))
        }
        RelocationTarget::Section(index) => (index, relocation.addend() as u64),
        other => panic!("a relocation to {other:?}"),
    }
}

fn data_at<'d>(file: &File<'d>, section: SectionIndex, offset: u64, length: usize) -> Vec<u8> {
    file.section_by_index(section).unwrap().data_range(file.section_by_index(section).unwrap().address() + offset, length as u64).unwrap().expect("bytes in the section").to_vec()
}

fn word(file: &File, section: SectionIndex, offset: u64) -> u64 {
    u64::from_le_bytes(data_at(file, section, offset, 8).try_into().unwrap())
}

/// The bytes of each table, which the directory `mote_tables` leads to, and the section and offset of the address words.
struct Tables {
    types: Vec<u8>,
    faults: Vec<u8>,
    code: Vec<u8>,
    words: (SectionIndex, u64, usize),
    meta: Vec<u8>,
    link: Vec<u8>,
    debug: Vec<u8>,
    flags: u64,
    /// The cell of the run state and the native slots, when the directory points at them.
    failure: Option<(SectionIndex, u64)>,
    natives: Option<(SectionIndex, u64)>,
}

fn tables(file: &File) -> Tables {
    let directory = file.symbol_by_name("mote_tables").expect("mote_tables");
    let section = directory.section_index().expect("defined");
    let at = directory.address();
    if file.format() == BinaryFormat::Elf {
        assert_eq!(directory.size(), DIRECTORY_WORDS as u64 * 8);
    }
    let read = |position: u64| {
        let (target_section, offset) = target(file, section, at + position * 16);
        (target_section, offset, word(file, section, at + position * 16 + 8) as usize)
    };
    let bytes = |position: u64| {
        let (section, offset, length) = read(position);
        data_at(file, section, offset, length)
    };
    let cell = |word: u64| {
        let place = at + word * 8;
        file.section_by_index(section).unwrap().relocations().any(|(offset, _)| offset == place).then(|| target(file, section, place))
    };
    let (section, offset, count) = read(3);
    Tables { types: bytes(0), faults: bytes(1), code: bytes(2), words: (section, offset, count), meta: bytes(4), link: bytes(5), debug: bytes(8), flags: word(file, section, at + DIRECTORY_FLAGS as u64 * 8), failure: cell(12), natives: cell(13) }
}

#[test]
fn object_is_a_relocatable_x86_64_elf_file_that_defines_the_run_function_and_the_tables() {
    let bytes = object_of(CHECKS);
    let file = File::parse(&bytes[..]).unwrap();
    let expected_format = if cfg!(target_os = "windows") { BinaryFormat::Coff } else { BinaryFormat::Elf };
    assert_eq!(file.format(), expected_format);
    assert_eq!(file.architecture(), Architecture::X86_64);
    assert_eq!(file.kind(), ObjectKind::Relocatable);
    for name in ["mote_run", "mote_tables"] {
        let symbol = file.symbol_by_name(name).unwrap_or_else(|| panic!("no symbol {name}"));
        assert!(symbol.is_global() && !symbol.is_undefined(), "{name}");
    }
    assert!(file.symbol_by_name("mote_failure").is_none());
    assert!(file.symbol_by_name("mote_natives").is_none());
}

#[test]
fn object_the_symbols_it_does_not_define_are_the_runtime_entries_and_helpers() {
    let bytes = object_of(CHECKS);
    let file = File::parse(&bytes[..]).unwrap();
    let known: HashSet<&str> = Entry::ALL.iter().map(|entry| entry.name()).chain([helpers::PRINT, helpers::FMOD, helpers::SHOW, helpers::COMPARE, helpers::CUT, helpers::MATH, helpers::MEM, helpers::TEXT, helpers::SYSTEM, helpers::VALUE, loader::NATIVE]).collect();
    let undefined: Vec<&str> = file.symbols().filter(|symbol| symbol.is_undefined()).filter_map(|symbol| symbol.name().ok()).filter(|name| !name.is_empty()).collect();
    assert!(undefined.contains(&Entry::Fault.name()) && undefined.contains(&helpers::PRINT), "{undefined:?}");
    for name in &undefined {
        assert!(known.contains(name), "{name} is not an entry or a helper");
    }
}

#[test]
fn object_the_directory_leads_to_a_type_table_that_reads_back() {
    let mut pipeline = built(CHECKS);
    let expected = typetable::build(&pipeline.module, &mut pipeline.program, &pipeline.interner);
    let bytes = compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, Vec::new(), &|_| None).unwrap();
    let file = File::parse(&bytes[..]).unwrap();
    let image = TypeImage::from_bytes(&tables(&file).types).unwrap();
    assert_eq!(image.table, expected.table);
}

#[test]
fn object_the_directory_leads_to_a_fault_table_with_a_site_for_each_check() {
    let bytes = object_of(CHECKS);
    let file = File::parse(&bytes[..]).unwrap();
    let faults = FaultTable::from_bytes(&tables(&file).faults).unwrap();
    let kinds: Vec<FaultKind> = (0..faults.len() as u32).map(|raw| faults.get_raw(raw).unwrap().kind).collect();
    for kind in [FaultKind::StackDepth, FaultKind::IntegerOverflow, FaultKind::DivideByZero, FaultKind::ShiftOutOfRange] {
        assert!(kinds.contains(&kind), "no site of kind {kind:?} in {kinds:?}");
    }
}

#[test]
fn object_the_code_table_places_each_function_by_a_relocation_and_keeps_its_sites_inside_the_function() {
    let bytes = object_of(HELD);
    let file = File::parse(&bytes[..]).unwrap();
    let tables = tables(&file);
    let code = CodeTable::from_bytes(&tables.code).unwrap();
    assert_eq!(tables.words.2, code.words());
    assert!(!code.functions.is_empty());
    for (at, sites) in code.functions.iter().enumerate() {
        let (section, offset) = target(&file, tables.words.0, tables.words.1 + at as u64 * 8);
        let function = file.symbols().find(|symbol| symbol.section_index() == Some(section) && symbol.address() == offset && symbol.name().is_ok_and(|name| name.starts_with('f'))).expect("a function symbol");
        if file.format() == BinaryFormat::Elf {
            assert!(sites.iter().all(|(site, _)| u64::from(*site) < function.size()), "{sites:?} in {}", function.size());
        }
    }
}

#[test]
fn object_the_frames_are_the_ones_the_jit_gives() {
    let mut pipeline = built(HELD);
    let compiled = crate::compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).unwrap();
    let mut jit: Vec<String> = compiled.code.descriptors().map(|(_, frame)| format!("{frame:?}")).collect();
    let bytes = compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, Vec::new(), &|_| None).unwrap();
    let file = File::parse(&bytes[..]).unwrap();
    let code = CodeTable::from_bytes(&tables(&file).code).unwrap();
    let mut object: Vec<String> = code.functions.iter().flatten().map(|(_, frame)| format!("{frame:?}")).collect();
    assert!(!object.is_empty());
    jit.sort();
    object.sort();
    assert_eq!(object, jit);
}

#[test]
fn object_a_global_that_holds_a_reference_is_a_root_placed_by_a_relocation() {
    let bytes = object_of(GLOBAL);
    let file = File::parse(&bytes[..]).unwrap();
    let tables = tables(&file);
    let code = CodeTable::from_bytes(&tables.code).unwrap();
    assert_eq!(code.roots, [vec![Slot::Ref(0)]]);
    let (section, offset) = target(&file, tables.words.0, tables.words.1 + code.functions.len() as u64 * 8);
    let cell = file.symbols().find(|symbol| symbol.section_index() == Some(section) && symbol.address() == offset).expect("the cell");
    if file.format() == BinaryFormat::Elf {
        assert_eq!(cell.size(), 8);
    }
    assert!(file.section_by_index(section).unwrap().kind() == ::object::SectionKind::UninitializedData || file.section_by_index(section).unwrap().kind() == ::object::SectionKind::Data);
}

#[test]
fn object_an_entry_that_returns_an_err_defines_the_failure_cell() {
    let bytes = object_of("fn main() -> Result<(), Error> {\n    return Err(Error.other(\"boom\"))\n}\n");
    let file = File::parse(&bytes[..]).unwrap();
    let symbol = file.symbol_by_name("mote_failure").expect("mote_failure");
    assert!(symbol.is_global() && !symbol.is_undefined());
    if file.format() == BinaryFormat::Elf {
        assert_eq!(symbol.size(), 24);
    }
}

#[test]
fn object_the_code_in_the_file_is_position_independent() {
    let bytes = object_of(HELD);
    let file = File::parse(&bytes[..]).unwrap();
    let text = file.section_by_name(".text").expect(".text");
    assert!(text.relocations().count() > 0);
    for (_, relocation) in text.relocations() {
        assert!(!matches!(relocation.kind(), ::object::RelocationKind::Absolute), "an absolute relocation in the code");
    }
}

const RECORDS: &str = "struct Point {\n    x: Int\n    y: Int\n}\n\nenum Shape {\n    Dot\n    Circle(Int)\n    Box { w: Int, h: Int }\n}\n\nfn main() {\n    println(Point { x: 1, y: 2 })\n    println(Shape.Circle(3))\n}\n";

const NATIVE: &str = "extern \"demo\" {\n    fn add_i64(a: Int, b: Int) -> Int\n}\n\nfn main() {\n    println(add_i64(40, 2))\n}\n";

#[test]
fn object_tables_the_meta_table_reads_back_equal_to_the_programs() {
    let mut pipeline = built(RECORDS);
    let bytes = compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, Vec::new(), &|_| None).unwrap();
    let types = typetable::build(&pipeline.module, &mut pipeline.program, &pipeline.interner);
    let expected = meta::build(&pipeline.program, &pipeline.interner, &types);
    let file = File::parse(&bytes[..]).unwrap();
    let found = mote_abi::Meta::from_bytes(&tables(&file).meta).unwrap();
    assert_eq!(found, expected);
    assert!(found.shown.iter().any(|shown| matches!(shown, mote_abi::Shown::Record(name, fields) if name == "Point" && fields == &["x", "y"])));
    assert!(found.shown.iter().any(|shown| matches!(shown, mote_abi::Shown::Enum(variants) if variants.len() == 3)));
}

#[test]
fn object_tables_the_link_table_carries_the_libraries_functions_and_files() {
    let mut pipeline = built(NATIVE);
    let files = vec!["/opt/mote/libdemo.so".to_string()];
    let libraries = vec![ObjectLibrary { file: files[0].clone(), archive: false }];
    let bytes = compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, libraries, &|_| None).unwrap();
    let file = File::parse(&bytes[..]).unwrap();
    let tables = tables(&file);
    let image = mote_abi::LinkImage::from_bytes(&tables.link).unwrap();
    assert_eq!(image.table.libraries, ["demo"]);
    assert_eq!(image.files, files);
    assert_eq!(image.table.functions.len(), 1);
    assert_eq!(image.table.functions[0].symbol, "add_i64");
    let (section, offset) = tables.natives.expect("the directory points at the native slots");
    let slots = file.symbols().find(|symbol| symbol.name() == Ok("mote_natives")).expect("mote_natives");
    assert_eq!((slots.section_index(), slots.address()), (Some(section), offset));
    if file.format() == BinaryFormat::Elf {
        assert_eq!(slots.size(), 8);
    }
}

#[test]
fn object_tables_a_library_linked_in_has_its_symbols_in_the_slots_and_a_weak_init() {
    let mut pipeline = built(NATIVE);
    let libraries = vec![ObjectLibrary { file: "libdemo.so".into(), archive: true }];
    let bytes = compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, libraries, &|_| None).unwrap();
    let file = File::parse(&bytes[..]).unwrap();
    let function = file.symbol_by_name("add_i64").expect("the native's symbol");
    assert!(function.is_undefined() && !function.is_weak());
    let init = file.symbol_by_name("mote_init_demo").expect("the init's symbol");
    if file.format() == BinaryFormat::Elf {
        assert!(init.is_undefined() && init.is_weak());
    } else {
        assert!(init.is_undefined());
    }
    let (section, offset) = tables(&file).natives.expect("the native slots");
    assert!(file.section_by_index(section).unwrap().relocations().any(|(at, relocation)| at == offset && matches!(relocation.target(), RelocationTarget::Symbol(index) if index == function.index())));
    assert_eq!(init_symbol("my-lib.2"), "mote_init_my_lib_2");
}

#[test]
fn object_tables_the_debug_table_names_each_function_and_places_its_code_and_its_checks() {
    let mut pipeline = built(CHECKS);
    let resolve = |position: u32| Some(("src/main.mote".to_string(), position + 1, 1));
    let bytes = compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, Vec::new(), &resolve).unwrap();
    let file = File::parse(&bytes[..]).unwrap();
    let found = tables(&file);
    let debug = mote_abi::DebugTable::from_bytes(&found.debug).unwrap();
    let code = mote_abi::CodeTable::from_bytes(&found.code).unwrap();
    let faults = FaultTable::from_bytes(&found.faults).unwrap();
    assert_eq!(debug.files, ["src/main.mote"]);
    assert_eq!(debug.functions.len(), code.functions.len());
    assert_eq!(debug.faults.len(), faults.len());
    let names: Vec<&str> = debug.functions.iter().map(|function| function.name.as_str()).collect();
    assert!(names.contains(&"main") && names.contains(&"grow"), "{names:?}");
    let main = debug.functions.iter().find(|function| function.name == "main").unwrap();
    assert!(main.size > 0 && !main.rows.is_empty());
    assert!(debug.faults.iter().all(|place| debug.place(*place).is_some()));
    let (file, line, _) = debug.fault_place(0).unwrap();
    assert_eq!((file, line), ("src/main.mote", faults.get_raw(0).unwrap().span.start + 1));
}

#[test]
fn object_the_line_table_has_a_sequence_for_each_function_placed_by_a_relocation_with_the_rows_of_the_debug_table() {
    let mut pipeline = built(CHECKS);
    let resolve = |position: u32| Some(("src/main.mote".to_string(), position + 1, 3));
    let bytes = compile_object(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, None, Vec::new(), &resolve).unwrap();
    if cfg!(target_os = "windows") {
        return;
    }
    let file = File::parse(&bytes[..]).unwrap();
    let debug = mote_abi::DebugTable::from_bytes(&tables(&file).debug).unwrap();
    let section = file.section_by_name(".debug_line").expect("a line section");
    let data = section.data().unwrap();
    let program = gimli::DebugLine::new(data, gimli::LittleEndian).program(gimli::DebugLineOffset(0), 8, None, None).unwrap();
    let mut sequences: Vec<Vec<(u64, u64, u64)>> = vec![Vec::new()];
    let mut rows = program.rows();
    while let Some((header, row)) = rows.next_row().unwrap() {
        let name = match row.file(header).unwrap().path_name() {
            gimli::AttributeValue::String(name) => name.to_vec(),
            other => panic!("{other:?}"),
        };
        assert_eq!(name, b"src/main.mote");
        if row.end_sequence() {
            sequences.push(Vec::new());
            continue;
        }
        let column = match row.column() {
            gimli::ColumnType::Column(column) => column.get(),
            gimli::ColumnType::LeftEdge => 0,
        };
        sequences.last_mut().unwrap().push((row.address(), row.line().unwrap().get(), column));
    }
    sequences.pop();
    let expected: Vec<Vec<(u64, u64, u64)>> = debug
        .functions
        .iter()
        .filter(|function| !function.rows.is_empty())
        .map(|function| function.rows.iter().map(|&(offset, place)| (u64::from(offset), u64::from(debug.places[place as usize].line), 3)).collect())
        .collect();
    assert!(expected.len() > 1);
    assert_eq!(sequences, expected);
    let starts: Vec<_> = section.relocations().collect();
    assert_eq!(starts.len(), expected.len());
    for (_, relocation) in starts {
        let RelocationTarget::Symbol(index) = relocation.target() else { panic!("{relocation:?}") };
        let symbol = file.symbol_by_index(index).unwrap();
        assert_eq!(file.section_by_index(symbol.section_index().unwrap()).unwrap().name().unwrap(), ".text");
    }
}

#[test]
fn object_tables_the_directory_points_at_the_failure_cell_only_when_the_entry_returns_a_result() {
    let with = object_of("fn main() -> Result<(), Error> {\n    return Err(Error.other(\"boom\"))\n}\n");
    let file = File::parse(&with[..]).unwrap();
    let found = tables(&file);
    let cell = file.symbol_by_name("mote_failure").expect("mote_failure");
    assert_eq!(found.failure, Some((cell.section_index().unwrap(), cell.address())));
    assert_eq!(found.natives, None);
    let without = object_of(CHECKS);
    assert_eq!(tables(&File::parse(&without[..]).unwrap()).failure, None);
}

#[test]
fn object_tables_the_directory_sets_single_threaded_flag_only_without_spawn_or_blocking() {
    let single = object_of(CHECKS);
    let file = File::parse(&single[..]).unwrap();
    assert_eq!(tables(&file).flags & DIRECTORY_FLAG_SINGLE_THREADED as u64, DIRECTORY_FLAG_SINGLE_THREADED as u64);

    let spawned = object_of("fn main() {\n    let t = spawn { 1 + 1 }\n    println(t.await())\n}\n");
    let file = File::parse(&spawned[..]).unwrap();
    assert_eq!(tables(&file).flags & DIRECTORY_FLAG_SINGLE_THREADED as u64, 0);
}
