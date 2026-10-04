//! `mote-rt`: the lean runtime, the binary `mote build` attaches programs to.

fn main() {
    cli::standalone::main::<{ cli::builtins::TIER_LEAN }>()
}
