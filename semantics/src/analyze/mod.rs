mod init;
mod moves;
mod parking;
mod tasks;
#[cfg(test)]
mod tests;

pub use parking::PARKING_INTRINSICS;

use mote_base::{DiagnosticSink, Interner};

use crate::defs::{DefId, LangItem};
use crate::program::Program;
use crate::tree::Body;

/// Orders start-up and checks what needs the whole program: moved channel ends, `var` globals in tasks and parking inside `Mutex.lock`.
pub fn analyze(program: &mut Program, interner: &Interner, sink: &mut DiagnosticSink) {
    let order = init::order(program, interner, sink);
    program.init = order;
    let bodies = bodies(program);
    moves::check(program, interner, &bodies, sink);
    tasks::check(program, interner, sink);
    parking::check(program, interner, &bodies, sink);
}

/// Every instance, then each global's initializer and each module's top level, in a fixed order.
fn bodies(program: &Program) -> Vec<&Body> {
    let instances = program.order.iter().filter_map(|instance| program.bodies.get(instance));
    let globals = program.defs.iter().filter_map(|(def, _)| program.global_inits.get(&def));
    let top_levels = program.modules.iter().filter_map(|(module, _)| program.top_level.get(&module));
    instances.chain(globals).chain(top_levels).collect()
}

/// The lang item that owns a method, and the method's name.
fn method_of<'a>(program: &Program, interner: &'a Interner, def: DefId) -> Option<(LangItem, &'a str)> {
    let owner = program.defs[def].owner?;
    Some((program.defs[owner].lang?, interner.resolve(program.defs[def].name)))
}
