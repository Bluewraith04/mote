//! The block pool: 32 KiB blocks of 128-byte lines in 4 MiB chunks, each block with its line and object mark maps.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::sys::{self, SysError};

/// Bytes of a block, the unit the heap hands out and takes back.
pub const BLOCK_SIZE: usize = 32 * 1024;

/// Bytes of a line, the unit of marking and of a hole.
pub const LINE_SIZE: usize = 128;

pub const LINES_PER_BLOCK: usize = BLOCK_SIZE / LINE_SIZE;

/// Blocks in one mapping from the system.
pub const CHUNK_BLOCKS: usize = 128;

pub const CHUNK_SIZE: usize = BLOCK_SIZE * CHUNK_BLOCKS;

/// Words of a block: one object mark bit for each.
const BLOCK_WORDS: usize = BLOCK_SIZE / 8;

const OBJECT_MARK_WORDS: usize = BLOCK_WORDS / 64;

/// A block of the pool, by position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(u32);

impl BlockId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("more than u32::MAX blocks"))
    }
}

/// A run of free lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hole {
    pub first: usize,
    pub lines: usize,
}

impl Hole {
    pub fn bytes(self) -> usize {
        self.lines * LINE_SIZE
    }

    /// The address of the hole in the block at `base`.
    pub fn start(self, base: usize) -> usize {
        base + self.first * LINE_SIZE
    }
}

/// One mark byte for each line of a block.
pub struct LineMap([AtomicU8; LINES_PER_BLOCK]);

impl LineMap {
    fn new() -> Self {
        Self(std::array::from_fn(|_| AtomicU8::new(0)))
    }

    /// Marks the lines `first` to `last`, both included.
    pub fn mark_range(&self, first: usize, last: usize) {
        for line in &self.0[first..=last] {
            line.store(1, Ordering::Relaxed);
        }
    }

    pub fn is_marked(&self, line: usize) -> bool {
        self.0[line].load(Ordering::Relaxed) != 0
    }

    pub fn marked(&self) -> usize {
        self.0.iter().filter(|line| line.load(Ordering::Relaxed) != 0).count()
    }

    pub fn clear(&self) {
        for line in &self.0 {
            line.store(0, Ordering::Relaxed);
        }
    }

    /// The first run of at least `min` free lines that starts at line `from` or later.
    pub fn next_hole(&self, from: usize, min: usize) -> Option<Hole> {
        let mut line = from;
        while line < LINES_PER_BLOCK {
            if self.is_marked(line) {
                line += 1;
                continue;
            }
            let first = line;
            while line < LINES_PER_BLOCK && !self.is_marked(line) {
                line += 1;
            }
            if line - first >= min {
                return Some(Hole { first, lines: line - first });
            }
        }
        None
    }

    /// Every run of free lines, in order.
    pub fn holes(&self) -> Vec<Hole> {
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(hole) = self.next_hole(from, 1) {
            from = hole.first + hole.lines;
            found.push(hole);
        }
        found
    }
}

/// One mark bit for each word of a block; an object is marked at its first word.
pub struct ObjectMap([AtomicU64; OBJECT_MARK_WORDS]);

impl ObjectMap {
    fn new() -> Self {
        Self(std::array::from_fn(|_| AtomicU64::new(0)))
    }

    /// Sets the bit of the word; true when it was clear.
    pub fn set(&self, word: usize) -> bool {
        let bit = 1u64 << (word % 64);
        self.0[word / 64].fetch_or(bit, Ordering::Relaxed) & bit == 0
    }

    pub fn is_set(&self, word: usize) -> bool {
        self.0[word / 64].load(Ordering::Relaxed) & (1 << (word % 64)) != 0
    }

    pub fn clear(&self) {
        for word in &self.0 {
            word.store(0, Ordering::Relaxed);
        }
    }

    pub fn count(&self) -> usize {
        self.0.iter().map(|word| word.load(Ordering::Relaxed).count_ones() as usize).sum()
    }
}

/// The mark state of one block.
pub struct Marks {
    pub lines: LineMap,
    pub objects: ObjectMap,
}

impl Marks {
    fn new() -> Self {
        Self { lines: LineMap::new(), objects: ObjectMap::new() }
    }

    pub fn clear(&self) {
        self.lines.clear();
        self.objects.clear();
    }
}

struct Chunk {
    base: usize,
    marks: Box<[Marks]>,
}

/// The blocks of the heap: chunks of address space from the system, numbered in the order they came.
#[derive(Default)]
pub struct Pool {
    chunks: Vec<Chunk>,
    /// The chunk bases and their positions in `chunks`, by address.
    by_address: Vec<(usize, usize)>,
}

impl Pool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a chunk of zeroed blocks; the first of them.
    pub fn grow(&mut self) -> Result<BlockId, SysError> {
        let memory = sys::map_aligned(CHUNK_SIZE, BLOCK_SIZE)?;
        let base = memory.as_ptr() as usize;
        let first = BlockId(u32::try_from(self.chunks.len() * CHUNK_BLOCKS).expect("more than u32::MAX blocks"));
        self.chunks.push(Chunk { base, marks: (0..CHUNK_BLOCKS).map(|_| Marks::new()).collect() });
        let at = self.by_address.partition_point(|&(other, _)| other < base);
        self.by_address.insert(at, (base, self.chunks.len() - 1));
        Ok(first)
    }

    pub fn blocks(&self) -> usize {
        self.chunks.len() * CHUNK_BLOCKS
    }

    pub fn ids(&self) -> impl Iterator<Item = BlockId> {
        (0..self.blocks() as u32).map(BlockId)
    }

    /// The address of the first byte of the block.
    pub fn base(&self, block: BlockId) -> usize {
        self.chunks[block.index() / CHUNK_BLOCKS].base + (block.index() % CHUNK_BLOCKS) * BLOCK_SIZE
    }

    pub fn marks(&self, block: BlockId) -> &Marks {
        &self.chunks[block.index() / CHUNK_BLOCKS].marks[block.index() % CHUNK_BLOCKS]
    }

    /// The block that holds the address.
    pub fn block_of(&self, address: usize) -> Option<BlockId> {
        let at = self.by_address.partition_point(|&(base, _)| base <= address).checked_sub(1)?;
        let (base, chunk) = self.by_address[at];
        (address < base + CHUNK_SIZE).then(|| BlockId((chunk * CHUNK_BLOCKS + (address - base) / BLOCK_SIZE) as u32))
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        for chunk in &self.chunks {
            let memory = std::ptr::NonNull::new(chunk.base as *mut u8).expect("a chunk is mapped");
            // SAFETY: the chunk was mapped by `grow` and the pool owns it.
            unsafe { sys::unmap(memory, CHUNK_SIZE) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_the_sizes_are_32_kib_blocks_of_128_byte_lines_in_4_mib_chunks() {
        assert_eq!([BLOCK_SIZE, LINE_SIZE, LINES_PER_BLOCK, CHUNK_BLOCKS, CHUNK_SIZE], [32768, 128, 256, 128, 4 << 20]);
        assert!(BLOCK_SIZE.is_power_of_two() && LINE_SIZE.is_power_of_two());
        assert_eq!(OBJECT_MARK_WORDS, 64);
    }

    #[test]
    fn blocks_a_chunk_is_block_aligned_zeroed_and_numbered_from_zero() {
        let mut pool = Pool::new();
        let first = pool.grow().unwrap();
        assert_eq!((first.index(), pool.blocks()), (0, CHUNK_BLOCKS));
        assert_eq!(pool.base(first) % BLOCK_SIZE, 0);
        assert_eq!(pool.base(BlockId(1)), pool.base(first) + BLOCK_SIZE);
        let last = BlockId(CHUNK_BLOCKS as u32 - 1);
        // SAFETY: the first and the last word of the chunk are mapped.
        unsafe {
            assert_eq!((pool.base(first) as *const u64).read(), 0);
            ((pool.base(last) + BLOCK_SIZE - 8) as *mut u64).write(5);
        }
    }

    #[test]
    fn blocks_an_address_finds_its_block_across_chunks() {
        let mut pool = Pool::new();
        let one = pool.grow().unwrap();
        let two = pool.grow().unwrap();
        assert_eq!((one.index(), two.index()), (0, CHUNK_BLOCKS));
        for block in [one, BlockId(77), two, BlockId(CHUNK_BLOCKS as u32 + 5), BlockId(2 * CHUNK_BLOCKS as u32 - 1)] {
            let base = pool.base(block);
            assert_eq!(pool.block_of(base), Some(block));
            assert_eq!(pool.block_of(base + BLOCK_SIZE - 1), Some(block));
        }
        assert_eq!(pool.block_of(0), None);
        assert_eq!(pool.block_of(usize::MAX), None);
        let end = (0..pool.blocks()).map(|at| pool.base(BlockId(at as u32)) + BLOCK_SIZE).max().unwrap();
        assert_eq!(pool.block_of(end), None);
    }

    #[test]
    fn blocks_marked_lines_leave_holes_between_them() {
        let map = LineMap::new();
        assert_eq!(map.holes(), [Hole { first: 0, lines: LINES_PER_BLOCK }]);
        map.mark_range(0, 1);
        map.mark_range(5, 5);
        map.mark_range(LINES_PER_BLOCK - 1, LINES_PER_BLOCK - 1);
        assert_eq!(map.marked(), 4);
        assert_eq!(map.holes(), [Hole { first: 2, lines: 3 }, Hole { first: 6, lines: LINES_PER_BLOCK - 7 }]);
        map.clear();
        assert_eq!(map.marked(), 0);
    }

    #[test]
    fn blocks_a_hole_search_skips_runs_that_are_too_short() {
        let map = LineMap::new();
        for line in [1, 4, 5, 9] {
            map.mark_range(line, line);
        }
        assert_eq!(map.next_hole(0, 1), Some(Hole { first: 0, lines: 1 }));
        assert_eq!(map.next_hole(0, 2), Some(Hole { first: 2, lines: 2 }));
        assert_eq!(map.next_hole(0, 3), Some(Hole { first: 6, lines: 3 }));
        assert_eq!(map.next_hole(0, 4), Some(Hole { first: 10, lines: LINES_PER_BLOCK - 10 }));
        assert_eq!(map.next_hole(3, 2), Some(Hole { first: 6, lines: 3 }));
        assert_eq!(map.next_hole(0, LINES_PER_BLOCK), None);
        assert_eq!(map.next_hole(LINES_PER_BLOCK, 1), None);
    }

    #[test]
    fn blocks_a_hole_knows_its_bytes_and_its_address() {
        let hole = Hole { first: 3, lines: 2 };
        assert_eq!(hole.bytes(), 256);
        assert_eq!(hole.start(0x10000), 0x10000 + 384);
    }

    #[test]
    fn blocks_an_object_mark_is_one_bit_for_each_word_and_set_once() {
        let map = ObjectMap::new();
        assert!(map.set(0));
        assert!(!map.set(0));
        assert!(map.set(63) && map.set(64) && map.set(BLOCK_WORDS - 1));
        assert!(map.is_set(64) && !map.is_set(65));
        assert_eq!(map.count(), 4);
        map.clear();
        assert_eq!(map.count(), 0);
    }

    #[test]
    fn blocks_the_marks_of_each_block_are_apart() {
        let mut pool = Pool::new();
        let first = pool.grow().unwrap();
        let second = BlockId(first.0 + 1);
        pool.marks(first).lines.mark_range(0, 0);
        pool.marks(first).objects.set(3);
        assert!(!pool.marks(second).lines.is_marked(0));
        assert!(!pool.marks(second).objects.is_set(3));
        pool.marks(first).clear();
        assert!(!pool.marks(first).lines.is_marked(0));
    }
}
