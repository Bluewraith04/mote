//! Compiles a program to a relocatable object file.

use std::collections::HashMap;

use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId as ClifFunc, Linkage, Module as ClifModule};
use cranelift_object::{ObjectBuilder, ObjectModule};
use mote_abi::{
    CodeTable, DebugFunction, DebugTable, LinkImage, Place as DebugPlace, Reflection, TypeImage, DIRECTORY_CODE, DIRECTORY_DEBUG, DIRECTORY_FAILURE, DIRECTORY_FAULTS, DIRECTORY_INITS, DIRECTORY_LINK, DIRECTORY_META, DIRECTORY_NATIVES, DIRECTORY_TYPES, DIRECTORY_WORDS,
    DIRECTORY_WORDS_OF_CODE,
};
use mote_base::Interner;
use mote_ir::{EntryKind, FuncId, Module};
use mote_semantics::Program;

use crate::debug::Debug;
use crate::dwarf;
use crate::jit::{c_signature, isa, lower, Lowered};
use crate::meta;
use crate::CompileError;

/// What an address word of the code table is filled with.
enum Place {
    Function(ClifFunc),
    Data(DataId),
}

fn define(module: &mut ObjectModule, name: &str, linkage: Linkage, description: &DataDescription) -> Result<DataId, CompileError> {
    let data = module.declare_data(name, linkage, false, false).map_err(CompileError::internal)?;
    module.define_data(data, description).map_err(CompileError::internal)?;
    Ok(data)
}

fn bytes(module: &mut ObjectModule, name: &str, contents: Vec<u8>) -> Result<DataId, CompileError> {
    let mut description = DataDescription::new();
    description.define(contents.into_boxed_slice());
    description.set_align(8);
    define(module, name, Linkage::Local, &description)
}

/// A library of the program's link table, as the object file is told of it.
pub struct ObjectLibrary {
    /// The file to open when the library is not linked in.
    pub file: String,
    /// Whether the linker adds its archive, so its functions are symbols of the executable.
    pub archive: bool,
}

/// The symbol of the optional init of a library linked in: `mote_init_` and the name with every character but letters and digits as `_`.
pub fn init_symbol(library: &str) -> String {
    format!("mote_init_{}", library.chars().map(|found| if found.is_ascii_alphanumeric() { found } else { '_' }).collect::<String>())
}

/// Where a span start lies as a file name, a line and a column, or none when it lies in no file.
pub type Resolve<'a> = &'a dyn Fn(u32) -> Option<(String, u32, u32)>;

/// The debug table as it is built: each place is listed once.
struct DebugBuilder<'a> {
    resolve: Resolve<'a>,
    table: DebugTable,
    index: HashMap<(String, u32, u32), u32>,
}

impl DebugBuilder<'_> {
    /// The index of the place a span start lies in, or `NO_PLACE`.
    fn place(&mut self, position: u32) -> u32 {
        let Some(found) = (self.resolve)(position) else { return mote_abi::NO_PLACE };
        if let Some(&index) = self.index.get(&found) {
            return index;
        }
        let file = match self.table.files.iter().position(|name| *name == found.0) {
            Some(file) => file as u32,
            None => {
                self.table.files.push(found.0.clone());
                (self.table.files.len() - 1) as u32
            }
        };
        self.table.places.push(DebugPlace { file, line: found.1, column: found.2 });
        let index = (self.table.places.len() - 1) as u32;
        self.index.insert(found, index);
        index
    }

    fn function(&mut self, info: Debug) {
        let mut rows: Vec<(u32, u32)> = Vec::new();
        for (offset, position) in info.rows {
            let place = self.place(position);
            if place != mote_abi::NO_PLACE && rows.last().is_none_or(|(_, last)| *last != place) {
                rows.push((offset, place));
            }
        }
        self.table.functions.push(DebugFunction { name: info.name, size: info.size, rows });
    }
}

/// Compiles start-up and the entry to an object file whose `mote_run` runs them and whose `mote_tables` leads to the tables of the program; `libraries` has one entry for each library of the program, in the order of its link table, and `resolve` says where a span lies.
pub fn compile_object(module: &Module, program: &mut Program, interner: &Interner, entry: EntryKind, error_text: Option<FuncId>, libraries: Vec<ObjectLibrary>, resolve: Resolve) -> Result<Vec<u8>, CompileError> {
    let builder = ObjectBuilder::new(isa(true)?, "mote", default_libcall_names()).map_err(CompileError::internal)?;
    let statics: Vec<bool> = libraries.iter().map(|library| library.archive).collect();
    let Lowered { module: mut out, faults, types, safepoints, debug, roots, link, native_slots, failure, .. } = lower(ObjectModule::new(builder), module, program, interner, Some(entry), error_text, &statics)?;
    let files: Vec<String> = libraries.iter().map(|library| library.file.clone()).collect();
    let meta = meta::build(program, interner, &types);

    let mut code = CodeTable::default();
    let mut debug_table = DebugBuilder { resolve, table: DebugTable::default(), index: HashMap::new() };
    let mut places = Vec::new();
    for ((function, mut sites), (_, info)) in safepoints.into_iter().zip(debug) {
        if sites.is_empty() {
            continue;
        }
        sites.sort_by_key(|(offset, _)| *offset);
        code.functions.push(sites);
        debug_table.function(info);
        places.push(Place::Function(function));
    }
    for site in 0..faults.len() as u32 {
        let span = faults.get_raw(site).expect("a site of the table").span;
        let place = debug_table.place(span.start);
        debug_table.table.faults.push(place);
    }
    for (data, slots) in roots {
        code.roots.push(slots);
        places.push(Place::Data(data));
    }
    let inits: Vec<String> = link.libraries.iter().zip(&statics).filter(|(_, linked)| **linked).map(|(name, _)| init_symbol(name)).collect();
    let image = TypeImage { table: types.table, reflection: Reflection::default() }.to_bytes().map_err(CompileError::internal)?;
    let tables = [
        (DIRECTORY_TYPES, "table_types", image),
        (DIRECTORY_FAULTS, "table_faults", faults.to_bytes()),
        (DIRECTORY_CODE, "table_code", code.to_bytes()),
        (DIRECTORY_META, "table_meta", meta.to_bytes()),
        (DIRECTORY_LINK, "table_link", LinkImage { table: link, files }.to_bytes()),
        (DIRECTORY_DEBUG, "table_debug", debug_table.table.to_bytes()),
    ];
    let mut directory = DataDescription::new();
    let mut contents = vec![0u8; DIRECTORY_WORDS * 8];
    let mut pointers = Vec::new();
    for (word, name, table) in tables {
        contents[(word + 1) * 8..(word + 2) * 8].copy_from_slice(&(table.len() as u64).to_le_bytes());
        pointers.push((word, bytes(&mut out, name, table)?));
    }

    let mut words = DataDescription::new();
    words.define(vec![0; places.len().max(1) * 8].into_boxed_slice());
    words.set_align(8);
    for (at, place) in places.iter().enumerate() {
        match *place {
            Place::Function(function) => {
                let target = out.declare_func_in_data(function, &mut words);
                words.write_function_addr((at * 8) as u32, target);
            }
            Place::Data(data) => {
                let target = out.declare_data_in_data(data, &mut words);
                words.write_data_addr((at * 8) as u32, target, 0);
            }
        }
    }
    pointers.push((DIRECTORY_WORDS_OF_CODE, define(&mut out, "table_words", Linkage::Local, &words)?));
    contents[(DIRECTORY_WORDS_OF_CODE + 1) * 8..(DIRECTORY_WORDS_OF_CODE + 2) * 8].copy_from_slice(&(places.len() as u64).to_le_bytes());

    let mut init_words = DataDescription::new();
    init_words.define(vec![0; inits.len().max(1) * 8].into_boxed_slice());
    init_words.set_align(8);
    let signature = c_signature(&[mote_abi::Scalar::I64], &[], out.isa().default_call_conv());
    for (at, name) in inits.iter().enumerate() {
        let init = out.declare_function(name, Linkage::Preemptible, &signature).map_err(CompileError::internal)?;
        let target = out.declare_func_in_data(init, &mut init_words);
        init_words.write_function_addr((at * 8) as u32, target);
    }
    pointers.push((DIRECTORY_INITS, define(&mut out, "table_inits", Linkage::Local, &init_words)?));
    contents[(DIRECTORY_INITS + 1) * 8..(DIRECTORY_INITS + 2) * 8].copy_from_slice(&(inits.len() as u64).to_le_bytes());

    directory.define(contents.into_boxed_slice());
    directory.set_align(8);
    for (word, cell) in pointers.into_iter().chain(failure.map(|data| (DIRECTORY_FAILURE, data))).chain(native_slots.map(|data| (DIRECTORY_NATIVES, data))) {
        let target = out.declare_data_in_data(cell, &mut directory);
        directory.write_data_addr((word * 8) as u32, target, 0);
    }
    define(&mut out, "mote_tables", Linkage::Export, &directory)?;

    let mut product = out.finish();
    let symbols: Vec<_> = places
        .iter()
        .filter_map(|place| match *place {
            Place::Function(function) => Some(product.functions[function].expect("a defined function").0),
            Place::Data(_) => None,
        })
        .collect();
    dwarf::add_line_tables(&mut product, &debug_table.table, &symbols)?;
    product.emit().map_err(CompileError::internal)
}

#[cfg(test)]
mod tests;
