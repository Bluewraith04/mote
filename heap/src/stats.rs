//! Counters of the heap, and the report `--mem-stats` prints.

use std::fmt;

/// What the heap has done since it was made, and how it stands now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Objects allocated by a call to the heap; compiled code bumps small objects from its region and does not count them.
    pub allocations: usize,
    /// Holes handed to regions.
    pub holes: usize,
    pub blocks_handed: usize,
    pub large_allocated: usize,
    pub large_freed: usize,
    pub collections: usize,
    pub swept_blocks: usize,
    pub discarded_blocks: usize,
    pub finalized: usize,
    /// What the checks after collections found wrong.
    pub problems: usize,
    /// Objects and bytes the last collection kept.
    pub marked_objects: usize,
    pub marked_bytes: usize,
    pub peak_used: usize,
    /// Bytes of blocks in use plus large objects now.
    pub used_bytes: usize,
    /// Bytes of the large objects now.
    pub large_bytes: usize,
    /// Bytes of address space the block pool holds.
    pub pool_bytes: usize,
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rows = [
            ("allocations", self.allocations),
            ("holes", self.holes),
            ("blocks handed", self.blocks_handed),
            ("large allocated", self.large_allocated),
            ("large freed", self.large_freed),
            ("collections", self.collections),
            ("blocks swept", self.swept_blocks),
            ("blocks discarded", self.discarded_blocks),
            ("finalized", self.finalized),
            ("problems", self.problems),
            ("marked objects", self.marked_objects),
            ("marked bytes", self.marked_bytes),
            ("peak used bytes", self.peak_used),
            ("used bytes", self.used_bytes),
            ("large bytes", self.large_bytes),
            ("pool bytes", self.pool_bytes),
        ];
        for (name, value) in rows {
            writeln!(f, "{name:<18}{value:>14}")?;
        }
        Ok(())
    }
}

/// Blocks by state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub free: usize,
    pub unswept: usize,
    pub recyclable: usize,
    pub used: usize,
    pub large: usize,
}
