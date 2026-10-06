//! A mark-and-sweep garbage collector with size-class blocks.
/// Object header bits and sizes.
pub mod object_model;
/// Pointer-field scanning of objects.
pub mod scanning;
/// The standard slot layouts.
pub mod layout;
pub mod chunk;
pub mod slab;
/// The allocation interface.
pub mod allocator;
/// Collector configuration.
pub mod plan;
/// The collector.
pub mod gc;

pub use contracts::ValueSlot;
pub use plan::{GCConfig, PlanType};
pub use gc::GCController;

/// Builds the [`contracts::Heap`] that `config` names.
pub fn build_gc_engine(config: &GCConfig) -> Box<dyn contracts::Heap> {
    match config.plan_type {
        PlanType::NoGC | PlanType::MarkSweep => Box::new(GCController::new(config.clone())),
    }
}
