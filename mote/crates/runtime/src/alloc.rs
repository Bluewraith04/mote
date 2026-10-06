use std::ptr::NonNull;
use isa::value::{ObjectHeader, TypeDescriptor, Value};

/// Bump allocator providing sequential allocation out of chunked memory buffers.
pub struct BumpAllocator {
    chunks: Vec<Vec<u8>>,
    current_chunk_idx: usize,
    current_offset: usize,
    chunk_size: usize,
}

impl Default for BumpAllocator {
    fn default() -> Self {
        Self::new(64 * 1024)
    }
}

impl BumpAllocator {
    pub fn new(chunk_size: usize) -> Self {
        let chunk = vec![0u8; chunk_size];
        Self {
            chunks: vec![chunk],
            current_chunk_idx: 0,
            current_offset: 0,
            chunk_size,
        }
    }

    pub fn alloc_object(&mut self, type_ptr: NonNull<TypeDescriptor>, field_count: usize) -> NonNull<ObjectHeader> {
        let header_size = std::mem::size_of::<ObjectHeader>();
        let header_align = std::mem::align_of::<ObjectHeader>();
        let value_size = std::mem::size_of::<Value>();
        let total_size = header_size + field_count * value_size;

        let aligned_offset = (self.current_offset + header_align - 1) & !(header_align - 1);

        if aligned_offset + total_size > self.chunk_size {
            let new_chunk_size = self.chunk_size.max(total_size * 2);
            self.chunks.push(vec![0u8; new_chunk_size]);
            self.current_chunk_idx += 1;
            self.current_offset = 0;
        }

        let aligned_offset = (self.current_offset + header_align - 1) & !(header_align - 1);
        // SAFETY: the chunk has room for the header at `aligned_offset`, ensured above.
        let ptr = unsafe {
            let chunk_ptr = self.chunks[self.current_chunk_idx].as_mut_ptr();
            let obj_ptr = chunk_ptr.add(aligned_offset) as *mut ObjectHeader;

            std::ptr::write(
                obj_ptr,
                ObjectHeader {
                    type_ptr,
                    gc_state: std::sync::atomic::AtomicUsize::new(0),
                },
            );

            let fields_ptr = chunk_ptr.add(aligned_offset + header_size) as *mut Value;
            for i in 0..field_count {
                std::ptr::write(fields_ptr.add(i), Value::null());
            }

            NonNull::new_unchecked(obj_ptr)
        };

        self.current_offset = aligned_offset + total_size;
        ptr
    }
}

/// The heap a `Runtime` starts with: bump-allocates and never frees; replaced by `set_heap`.
#[derive(Default)]
pub struct LeakingHeap {
    retired: std::sync::Arc<std::sync::Mutex<Vec<BumpAllocator>>>,
}

struct LeakingMutator {
    bump: Option<BumpAllocator>,
    retired: std::sync::Arc<std::sync::Mutex<Vec<BumpAllocator>>>,
}

impl contracts::Mutator for LeakingMutator {
    fn alloc(&mut self, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader> {
        self.bump.as_mut().expect("present until drop").alloc_object(type_ptr, slot_count)
    }
}

impl Drop for LeakingMutator {
    fn drop(&mut self) {
        if let Some(bump) = self.bump.take() {
            self.retired.lock().unwrap().push(bump);
        }
    }
}

impl contracts::Heap for LeakingHeap {
    fn mutator(&self) -> Box<dyn contracts::Mutator> {
        Box::new(LeakingMutator { bump: Some(BumpAllocator::default()), retired: self.retired.clone() })
    }

    fn should_collect(&self) -> bool {
        false
    }

    fn collect(&self, _roots: &mut dyn contracts::RootSource) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::Heap;

    #[test]
    fn test_objects_outlive_the_mutator_that_allocated_them() {
        let mut type_node = TypeDescriptor::new(1, vec![Some("val".into())]);
        let type_ptr = NonNull::new(&mut type_node).unwrap();
        let heap = LeakingHeap::default();

        let obj = {
            let mut m = heap.mutator();
            let obj = m.alloc(type_ptr, 1);
            unsafe { (*obj.as_ptr()).set_field(0, Value::small_int(41)) };
            obj
        };

        assert_eq!(unsafe { obj.as_ref().get_field(0) }.as_int(), Some(41));
        assert!(!heap.should_collect());
    }
}
