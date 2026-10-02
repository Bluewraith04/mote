use std::ptr::NonNull;
use isa::value::{ObjectHeader, Value};

/// Represents a memory location that holds a `Value` (which may be an object reference).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValueSlot(pub *mut Value);

// SAFETY: a slot is a root location the collector reads and writes only while every mutator is stopped.
unsafe impl Send for ValueSlot {}
unsafe impl Sync for ValueSlot {}

impl ValueSlot {
    #[inline(always)]
    pub fn new(ptr: *mut Value) -> Self {
        Self(ptr)
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *mut Value {
        self.0
    }

    /// Checks if this slot contains a heap-allocated boxed object pointer.
    #[inline(always)]
    pub fn is_boxed_object(&self) -> bool {
        if self.0.is_null() {
            return false;
        }
        // SAFETY: the slot pointer is non-null and points at a live `Value`.
        unsafe { matches!(*self.0, Value::ObjectPtr(_)) }
    }

    /// Loads the raw object header pointer if the slot holds a `ObjectPtr`.
    #[inline(always)]
    pub fn load_object_ptr(&self) -> Option<NonNull<ObjectHeader>> {
        if self.0.is_null() {
            return None;
        }
        // SAFETY: the slot pointer is non-null and points at a live `Value`.
        unsafe { (*self.0).as_object_ptr() }
    }

    /// Stores a new object reference into this slot, ensuring the value is `Value::ObjectPtr`.
    #[inline(always)]
    pub fn store_object_ptr(&self, new_ptr: NonNull<ObjectHeader>) {
        assert!(!self.0.is_null(), "Cannot store into null ValueSlot");
        // SAFETY: the slot pointer is non-null and points at a live `Value`.
        unsafe {
            *self.0 = Value::boxed(new_ptr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use isa::value::TypeDescriptor;

    #[test]
    fn test_slot_round_trip() {
        let mut dummy_type = TypeDescriptor::new(42, vec![]);
        let mut dummy_obj = ObjectHeader {
            type_ptr: NonNull::new(&mut dummy_type).unwrap(),
            gc_state: 0.into(),
        };
        let obj_ptr = NonNull::new(&mut dummy_obj).unwrap();

        let mut val = Value::small_int(100);
        let slot = ValueSlot::new(&mut val);

        assert!(!slot.is_boxed_object());
        assert_eq!(slot.load_object_ptr(), None);

        slot.store_object_ptr(obj_ptr);
        assert!(slot.is_boxed_object());
        assert_eq!(slot.load_object_ptr(), Some(obj_ptr));

        assert!(matches!(val, Value::ObjectPtr(_)));
        assert_eq!(val.as_object_ptr(), Some(obj_ptr));
    }

}