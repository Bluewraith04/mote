//! Re-exports `ffi::builtins`.

pub use ffi::builtins::{
    dispatch_hook, full_registry, install, install_full, install_lean, lean_registry, registry, render, tier_of, TIER_FULL,
    TIER_GUI, TIER_LEAN,
};
