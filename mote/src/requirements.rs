//! Decides the `std.meta.require` calls of a program before it runs, on compiled code.

use mote_base::Interner;
use mote_ir::requirements::{checks, Unmet};
use mote_ir::{Entry, EntryKind, Module};
use mote_semantics::Program;

use crate::pipeline::Failure;
use crate::run::compile_failure;

/// The `require` calls that do not hold: each is decided by running what leads up to it, and a fault of that run is its message.
pub fn unmet(module: &Module, program: &mut Program, interner: &Interner) -> Result<Vec<Unmet>, Failure> {
    let (mut scratch, found) = checks(module, interner);
    let mut unmet = Vec::new();
    for check in found {
        scratch.entries = vec![Entry { kind: EntryKind::Main, func: check.thunk }];
        let compiled = mote_codegen::compile(&scratch, program, interner, EntryKind::Main).map_err(compile_failure)?;
        let ran = compiled.with_scheduler(mote_scheduler::Config::seeded(0)).run();
        if ran.link.is_none()
            && let Some(fault) = ran.fault
        {
            unmet.extend(check.unmet(module, program, fault.message));
        }
    }
    Ok(unmet)
}

#[cfg(test)]
mod tests {
    use mote_semantics::compile;

    use super::*;

    const TRAIT: &str = "import { require, describe, Kind } from std.meta\ntrait OnlyStructs {\n fn tag(self) -> String {\n  require(describe<Self>().kind() == Kind.Struct, \"needs a struct\")\n  return \"ok\"\n }\n}\n";

    /// The error texts of `unmet` over the program.
    fn texts(source: &str) -> Vec<String> {
        let mut compiled = compile(source);
        assert_eq!(compiled.errors, Vec::<String>::new());
        let lowered = mote_ir::lower(&mut compiled.program, &mut compiled.interner);
        assert_eq!(lowered.unsupported, []);
        let found = unmet(&lowered.module, &mut compiled.program, &compiled.interner).unwrap_or_else(|failure| panic!("{failure}"));
        found.iter().map(|found| found.text(&compiled.program, &compiled.interner)).collect()
    }

    /// The message of the fault `main` of the program ends with, if it does.
    fn fault_of(source: &str) -> Option<String> {
        let mut compiled = compile(source);
        assert_eq!(compiled.errors, Vec::<String>::new());
        let lowered = mote_ir::lower(&mut compiled.program, &mut compiled.interner);
        let ran = mote_codegen::compile(&lowered.module, &mut compiled.program, &compiled.interner, EntryKind::Main).unwrap_or_else(|error| panic!("{error}")).run();
        ran.fault.map(|fault| fault.message)
    }

    #[test]
    fn require_that_holds_for_the_listing_type_is_no_error() {
        assert_eq!(texts(&format!("{TRAIT}struct S: (OnlyStructs) {{\n x: Int\n}}")), Vec::<String>::new());
    }

    #[test]
    fn require_that_fails_is_an_error_naming_the_type_the_trait_and_the_message_though_nothing_calls_the_method() {
        let source = format!("{TRAIT}enum E: (OnlyStructs) {{\n A\n}}\nstruct S: (OnlyStructs) {{\n x: Int\n}}");
        assert_eq!(texts(&source), ["`E` cannot list `OnlyStructs`: needs a struct"]);
    }

    #[test]
    fn require_points_at_the_listing_type() {
        let source = format!("{TRAIT}enum E: (OnlyStructs) {{\n A\n}}");
        let mut compiled = compile(&source);
        let lowered = mote_ir::lower(&mut compiled.program, &mut compiled.interner);
        let found = unmet(&lowered.module, &mut compiled.program, &compiled.interner).unwrap();
        assert_eq!(found.len(), 1);
        let main = compiled.program.modules.iter().find(|(_, module)| module.path.len() == 1 && compiled.interner.resolve(module.path[0]) == "main").map(|(id, _)| id);
        assert_eq!(Some(found[0].module), main);
        let (owner, _) = found[0].listing.expect("a listing");
        assert_eq!(compiled.interner.resolve(compiled.program.defs[owner].name), "E");
        assert_eq!(found[0].span, compiled.program.defs[owner].span);
    }

    #[test]
    fn require_in_a_type_with_parameters_is_checked_where_the_type_is_used() {
        let declared = format!("{TRAIT}enum Maybe<T>: (OnlyStructs) {{\n None\n Some(T)\n}}\n");
        assert_eq!(texts(&declared), Vec::<String>::new());
        let used = format!("{declared}fn main() -> String {{\n let m: Maybe<Int> = Maybe.None\n return m.tag()\n}}");
        assert_eq!(texts(&used), ["`Maybe` cannot list `OnlyStructs`: needs a struct"]);
    }

    #[test]
    fn require_in_a_plain_function_is_an_error_with_only_its_message() {
        let source = "import { require } from std.meta\nfn guard() {\n require(false, \"never holds\")\n}\nfn main() {\n guard()\n}";
        assert_eq!(texts(source), ["never holds"]);
    }

    #[test]
    fn require_that_holds_in_a_plain_function_is_no_error_and_the_program_runs() {
        let source = "import { require } from std.meta\nfn main() -> Int {\n require(1 + 1 == 2, \"arithmetic\")\n return 7\n}";
        assert_eq!(texts(source), Vec::<String>::new());
        assert_eq!(fault_of(source), None);
    }

    #[test]
    fn require_that_reads_a_parameter_is_left_to_fault_when_it_runs() {
        let source = "import { require } from std.meta\nfn positive(n: Int) {\n require(n > 0, \"positive\")\n}\nfn main() {\n positive(0)\n}";
        assert_eq!(texts(source), Vec::<String>::new());
        assert_eq!(fault_of(source), Some("positive".to_string()));
    }

    #[test]
    fn require_after_a_branch_is_left_to_fault_when_it_runs() {
        let source = "import { require } from std.meta\nfn main() {\n if 1 < 2 {\n  require(false, \"late\")\n }\n}";
        assert_eq!(texts(source), Vec::<String>::new());
        assert_eq!(fault_of(source), Some("late".to_string()));
    }

    #[test]
    fn require_with_a_message_built_by_a_call_reports_the_built_text() {
        let source = "import { require } from std.meta\nfn name() -> String { return \"built\" }\nfn main() {\n require(false, \"from \" + name())\n}";
        assert_eq!(texts(source), ["from built"]);
    }

    #[test]
    fn require_whose_condition_faults_reports_the_fault() {
        let source = "import { require } from std.meta\nfn main() {\n let xs: List<Int> = []\n require(xs[0] == 1, \"first\")\n}";
        assert_eq!(texts(source), ["index out of range"]);
    }
}
