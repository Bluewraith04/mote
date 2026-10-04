//! `mote-rt-gui`: the runtime with everything, including the GUI.

fn main() {
    cli::standalone::main::<{ cli::builtins::TIER_GUI }>()
}
