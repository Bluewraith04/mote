//! Allocates and collects objects in a non-moving Immix heap.

mod blocks;
mod heap;
mod mark;
mod object;
mod stats;
mod sys;
mod world;

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;

pub use blocks::{BlockId, Hole, LineMap, Marks, ObjectMap, Pool, BLOCK_SIZE, CHUNK_BLOCKS, CHUNK_SIZE, LINES_PER_BLOCK, LINE_SIZE};
pub use heap::{Collection, Config, Finalizer, Full, Heap, OutOfMemory, LARGE_THRESHOLD};
pub use mark::{Marker, Roots};
pub use object::Object;
pub use stats::{Census, Stats};
pub use sys::SysError;
pub use world::{Mutator, World};
