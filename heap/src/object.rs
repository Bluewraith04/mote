//! A reference to a heap object, and the reads and writes of its words.

use std::ptr::NonNull;

use mote_abi::{Header, LENGTH_OFFSET};

/// One reference: the address of the first byte of an object, its header.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Object(NonNull<u8>);

// SAFETY: an `Object` is an address; what it points to is shared by the tasks of a program and guarded by the language.
unsafe impl Send for Object {}
// SAFETY: as above.
unsafe impl Sync for Object {}

impl std::fmt::Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Object({:#x})", self.address())
    }
}

impl Object {
    /// The object at the address; `None` for null.
    pub fn from_address(address: usize) -> Option<Self> {
        NonNull::new(address as *mut u8).map(Self)
    }

    pub fn address(self) -> usize {
        self.0.as_ptr() as usize
    }

    /// The word at `offset` bytes; the object is live and `offset` is inside it.
    pub fn word(self, offset: u32) -> u64 {
        read_word(self.address() + offset as usize)
    }

    /// Writes the word at `offset` bytes; the object is live and `offset` is inside it.
    pub fn set_word(self, offset: u32, value: u64) {
        // SAFETY: the caller keeps `offset` inside a live object, whose memory is mapped and word-aligned.
        unsafe { ((self.address() + offset as usize) as *mut u64).write(value) };
    }

    pub fn header(self) -> Header {
        Header::from_bits(self.word(0))
    }

    /// The element count of a string or an array: the byte length of a string.
    pub fn length(self) -> u64 {
        self.word(LENGTH_OFFSET)
    }
}

/// The word at the address.
pub(crate) fn read_word(address: usize) -> u64 {
    // SAFETY: the heap reads only addresses inside live objects and frames it was given, which are word-aligned.
    unsafe { (address as *const u64).read() }
}

/// The unsigned integer of `bytes` (1, 2, 4 or 8) at the address.
pub(crate) fn read_uint(address: usize, bytes: u32) -> u64 {
    // SAFETY: as `read_word`; an enum tag is aligned to its size.
    unsafe {
        match bytes {
            1 => u64::from((address as *const u8).read()),
            2 => u64::from((address as *const u16).read()),
            4 => u64::from((address as *const u32).read()),
            _ => (address as *const u64).read(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_null_is_no_object_and_an_address_round_trips() {
        assert_eq!(Object::from_address(0), None);
        let mut words = [0u64; 3];
        let object = Object::from_address(words.as_mut_ptr() as usize).unwrap();
        object.set_word(8, 11);
        object.set_word(16, 5);
        assert_eq!((object.word(8), object.length(), words[1]), (11, 11, 11));
        assert_eq!(object.address(), words.as_ptr() as usize);
    }
}
