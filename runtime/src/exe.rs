//! The entry of an executable: reads the tables of the program from its object file and runs it.

use std::io::Write;

use mote_abi::{
    CodeTable, DebugTable, FaultTable, LinkImage, Meta, TaskContext, TypeImage, DIRECTORY_CODE, DIRECTORY_DEBUG, DIRECTORY_FAILURE, DIRECTORY_FAULTS, DIRECTORY_INITS, DIRECTORY_LINK, DIRECTORY_META, DIRECTORY_NATIVES, DIRECTORY_TYPES,
    DIRECTORY_WORDS, DIRECTORY_WORDS_OF_CODE,
};

use crate::args::Args;
use crate::exec::Image;
use crate::loader::{init_linked, load, prefer_beside};
use crate::{CodeMap, FunctionInfo, GlobalRoot, HeapConfig};

unsafe extern "C" {
    /// The directory the compiler wrote: where the tables of the program are.
    safe static mote_tables: [usize; DIRECTORY_WORDS];
    /// Runs start-up and the entry; the compiler wrote it to take the context of the task.
    safe fn mote_run(context: *mut TaskContext);
}

/// The exit code of a command line that does not follow the usage.
const USAGE: i32 = 2;

/// The bytes at an address of the object file.
///
/// # Safety
/// The address and length come from the directory the compiler wrote.
unsafe fn table(word: usize) -> &'static [u8] {
    // SAFETY: the caller's promise.
    unsafe { std::slice::from_raw_parts(mote_tables[word] as *const u8, mote_tables[word + 1]) }
}

fn damaged(what: &str, error: impl std::fmt::Display) -> String {
    format!("the {what} table of this program is damaged: {error}")
}

/// Runs the program and gives the exit code.
fn execute(args: Args) -> Result<i32, String> {
    // SAFETY: the directory was written by the compiler, and its tables live as long as the executable.
    let (types, faults, code, meta, link, debug) = unsafe { (table(DIRECTORY_TYPES), table(DIRECTORY_FAULTS), table(DIRECTORY_CODE), table(DIRECTORY_META), table(DIRECTORY_LINK), table(DIRECTORY_DEBUG)) };
    let types = TypeImage::from_bytes(types).map_err(|error| damaged("type", error))?.table;
    let faults = FaultTable::from_bytes(faults).map_err(|error| damaged("fault", error))?;
    let code = CodeTable::from_bytes(code).map_err(|error| damaged("code", error))?;
    let meta = Meta::from_bytes(meta).map_err(|error| damaged("meta", error))?;
    let link = LinkImage::from_bytes(link).map_err(|error| damaged("link", error))?;
    // SAFETY: the compiler wrote a word for each function and root of the code table.
    let words = unsafe { std::slice::from_raw_parts(mote_tables[DIRECTORY_WORDS_OF_CODE] as *const usize, mote_tables[DIRECTORY_WORDS_OF_CODE + 1]) };
    if words.len() != code.words() {
        return Err(damaged("code", "it and its addresses differ in length"));
    }
    let debug = DebugTable::from_bytes(debug).map_err(|error| damaged("debug", error))?;
    if debug.functions.len() != code.functions.len() {
        return Err(damaged("debug", "it and the code table differ in length"));
    }
    let mut map = CodeMap::new();
    for ((sites, start), function) in code.functions.iter().zip(words).zip(&debug.functions) {
        for (offset, frame) in sites {
            map.add_frame(start + *offset as usize, frame.clone());
        }
        map.add_function(*start, FunctionInfo { name: function.name.clone(), size: function.size as usize, rows: function.rows.clone() });
    }
    for (slots, address) in code.roots.into_iter().zip(&words[code.functions.len()..]) {
        map.add_root(GlobalRoot { address: *address, slots });
    }
    let natives = mote_tables[DIRECTORY_NATIVES];
    if natives != 0 {
        // SAFETY: the slots are a word for each function of the link table, and nothing runs yet.
        let files = match std::env::current_exe() {
            Ok(exe) => prefer_beside(&link.files, exe.parent().unwrap_or(std::path::Path::new("."))),
            Err(_) => link.files.clone(),
        };
        unsafe { load(&link.table, &files, natives as *mut usize) }.map_err(|error| error.message)?;
    }
    // SAFETY: the compiler wrote a word for each library linked in.
    init_linked(unsafe { std::slice::from_raw_parts(mote_tables[DIRECTORY_INITS] as *const usize, mote_tables[DIRECTORY_INITS + 1]) });
    let failure = mote_tables[DIRECTORY_FAILURE];
    let scheduler = args.seed.map_or_else(mote_scheduler::Config::new, mote_scheduler::Config::seeded);
    let image = Image { run: mote_run, types, faults, meta, code: map, failure: (failure != 0).then_some(failure), heap: HeapConfig::default(), scheduler, feeds: args.feeds, args: args.program, sink: Some(Box::new(std::io::stdout())) };
    let ran = image.run();
    let _ = std::io::stdout().flush();
    Ok(match (ran.fault, ran.err) {
        (Some(fault), _) => {
            eprint!("{}", fault.text(&debug));
            1
        }
        (None, Some(text)) => {
            eprintln!("error: {text}");
            1
        }
        (None, None) => ran.exit,
    })
}

/// The program's `main`, which the C start-up code calls.
#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let args = match crate::args::parse(&arguments) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}");
            return USAGE;
        }
    };
    match execute(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}
