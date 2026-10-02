use std::mem::size_of;
use std::ptr::NonNull;
use isa::value::{ObjectHeader, Value};

/// Alignment constants for the object allocator.
pub(crate) const MIN_ALIGNMENT: usize = 8;
pub(crate) const HEADER_SIZE: usize = size_of::<ObjectHeader>();

/// Bit positions within `ObjectHeader.gc_state`.
pub(crate) const GC_MARK_BIT: usize = 1 << 0;
pub(crate) const GC_PINNED_BIT: usize = 1 << 2;
pub(crate) const GC_OLD_GEN_BIT: usize = 1 << 3;
/// Set on a block in a size-class chunk that holds no object (free for reuse).
pub(crate) const GC_FREE_BIT: usize = 1 << 4;

/// Full allocated size in bytes of a heap object (header + slots), from its descriptor's layout.
#[inline(always)]
pub(crate) fn get_object_size(header_ptr: NonNull<ObjectHeader>) -> usize {
    // SAFETY: the caller passes a live object whose descriptor outlives it.
    unsafe {
        let header = header_ptr.as_ref();
        let type_desc = header.type_ptr.as_ref();
        let slot_count = type_desc.layout.slot_count(header, type_desc.slots as usize);
        HEADER_SIZE + (slot_count * size_of::<Value>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use isa::value::TypeDescriptor;

    #[test]
    fn test_object_size_calculation() {
        let mut type_empty = TypeDescriptor::new(1, vec![]);
        let mut type_point = TypeDescriptor::new(
            2,
            vec![Some("x".into()), Some("y".into())],
        );
        let mut type_large = TypeDescriptor::with_field_count(3, 10);

        let mut obj_empty = ObjectHeader {
            type_ptr: NonNull::new(&mut type_empty).unwrap(),
            gc_state: std::sync::atomic::AtomicUsize::new(0),
        };
        let mut obj_point = ObjectHeader {
            type_ptr: NonNull::new(&mut type_point).unwrap(),
            gc_state: std::sync::atomic::AtomicUsize::new(0),
        };
        let mut obj_large = ObjectHeader {
            type_ptr: NonNull::new(&mut type_large).unwrap(),
            gc_state: std::sync::atomic::AtomicUsize::new(0),
        };

        assert_eq!(
            get_object_size(NonNull::new(&mut obj_empty).unwrap()),
            HEADER_SIZE
        );
        assert_eq!(
            get_object_size(NonNull::new(&mut obj_point).unwrap()),
            HEADER_SIZE + (2 * size_of::<Value>())
        );
        assert_eq!(
            get_object_size(NonNull::new(&mut obj_large).unwrap()),
            HEADER_SIZE + (10 * size_of::<Value>())
        );
    }
}