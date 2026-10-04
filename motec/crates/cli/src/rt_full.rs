//! `mote-rt-full`: the runtime with TLS and no GUI.

fn main() {
    cli::standalone::main::<{ cli::builtins::TIER_FULL }>()
}
