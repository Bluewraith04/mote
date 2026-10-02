use std::ptr::NonNull;
use isa::value::{ObjectHeader, TypeDescriptor};

/// Interface implemented by the VM's memory manager (`GCController`).
pub trait ObjectAllocator {
    /// Allocates an uninitialized block, writes the `ObjectHeader`, and zero/null-initializes all fields.
    fn alloc_object(&mut self, type_ptr: NonNull<TypeDescriptor>) -> NonNull<ObjectHeader>;

    fn allocated_bytes(&self) -> usize;
}
