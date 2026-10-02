use crate::slot::ValueSlot;
use isa::value::ObjectHeader;
use std::ptr::NonNull;

/// An object's slot count and which slots hold references.
pub trait TypeLayout {
    fn slot_count(&self, object: NonNull<ObjectHeader>) -> usize;

    fn visit_refs(&self, object: NonNull<ObjectHeader>, visitor: &mut dyn FnMut(ValueSlot));
}
