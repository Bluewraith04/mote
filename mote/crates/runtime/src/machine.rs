//! The narrow views of the machine that opcode handlers depend on, so a handler runs against
//! a test double instead of a whole `Runtime`.

use std::ptr::NonNull;

use isa::value::{ObjectHeader, TypeDescriptor};

use crate::CodeObject;

/// Read access to the program's code objects.
pub trait CodeTable {
    fn code_objects(&self) -> &[CodeObject];
}

/// Read access to the program's type descriptors.
pub trait TypeTable {
    fn type_descriptors(&self) -> &[TypeDescriptor];
    fn string_type(&self) -> &TypeDescriptor;
}

/// Where handlers put and barrier heap objects.
pub trait ObjectStore {
    fn alloc_object(&self, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader>;
}

impl CodeTable for crate::Runtime {
    fn code_objects(&self) -> &[CodeObject] {
        &self.code_objects
    }
}

impl TypeTable for crate::Runtime {
    fn type_descriptors(&self) -> &[TypeDescriptor] {
        &self.type_descriptors
    }

    fn string_type(&self) -> &TypeDescriptor {
        &self.string_type
    }
}

impl ObjectStore for crate::Runtime {
    fn alloc_object(&self, type_ptr: NonNull<TypeDescriptor>, slot_count: usize) -> NonNull<ObjectHeader> {
        self.with_mutator(|m| m.alloc(type_ptr, slot_count))
    }
}
