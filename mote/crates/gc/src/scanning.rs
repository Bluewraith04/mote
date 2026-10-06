use std::ptr::NonNull;
use contracts::ValueSlot;
use isa::value::{ObjectHeader, SlotLayout};

/// Visitor trait called by the GC tracer when scanning an object's fields.
pub trait ObjectVisitor {
    fn visit_slot(&mut self, slot: ValueSlot);
}

impl<F: FnMut(ValueSlot)> ObjectVisitor for F {
    #[inline(always)]
    fn visit_slot(&mut self, slot: ValueSlot) {
        self(slot);
    }
}

/// Scans all pointer-holding fields of a heap-allocated object, invoking `visitor` for each `ObjectPtr`.
#[inline]
pub(crate) fn scan_object<V: ObjectVisitor>(header_ptr: NonNull<ObjectHeader>, visitor: &mut V) {
    // SAFETY: the caller passes a live object whose slots match its descriptor.
    unsafe {
        let header = header_ptr.as_ref();
        let type_desc = header.type_ptr.as_ref();

        match type_desc.layout {
            SlotLayout::RawBytes => return,
            SlotLayout::ValueRun { values_per_unit } => {
                let count = header.get_field(0).as_len();
                for i in 1..=(count * values_per_unit) {
                    let slot = ValueSlot::new(header.field_ptr(i));
                    if slot.is_boxed_object() {
                        visitor.visit_slot(slot);
                    }
                }
                return;
            }
            SlotLayout::Fixed => {}
        }

        let field_count = type_desc.slots as usize;

        let bitmap = type_desc.pointer_bitmap;
        if bitmap != 0 {
            let limit = field_count.min(64);
            let mut mask = if limit == 64 {
                bitmap
            } else {
                bitmap & ((1u64 << limit).wrapping_sub(1))
            };

            while mask != 0 {
                let idx = mask.trailing_zeros() as usize;
                let slot = ValueSlot::new(header.field_ptr(idx));
                if slot.is_boxed_object() {
                    visitor.visit_slot(slot);
                }
                mask &= mask - 1;
            }

            for i in 64..field_count {
                if type_desc.is_field_pointer(i) {
                    let slot = ValueSlot::new(header.field_ptr(i));
                    if slot.is_boxed_object() {
                        visitor.visit_slot(slot);
                    }
                }
            }
        } else {
            for i in 0..field_count {
                let slot = ValueSlot::new(header.field_ptr(i));
                if slot.is_boxed_object() {
                    visitor.visit_slot(slot);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;
    use isa::value::{TypeDescriptor, Value};

    #[test]
    fn test_scan_object_with_primitives_only() {
        let mut dummy_type = TypeDescriptor::new(
            1,
            vec![Some("a".into()), Some("b".into()), Some("c".into())],
        );

        let total_size = size_of::<ObjectHeader>() + 3 * size_of::<Value>();
        let mut buffer = vec![0u8; total_size];
        let header_ptr = buffer.as_mut_ptr() as *mut ObjectHeader;

        unsafe {
            (*header_ptr).type_ptr = NonNull::new(&mut dummy_type).unwrap();
            (*header_ptr).gc_state = 0.into();
            (*header_ptr).set_field(0, Value::small_int(100));
            (*header_ptr).set_field(1, Value::float(2.5));
            (*header_ptr).set_field(2, Value::true_());
        }

        let non_null = NonNull::new(header_ptr).unwrap();
        let mut pointer_count = 0;
        scan_object(non_null, &mut |_slot: ValueSlot| {
            pointer_count += 1;
        });

        assert_eq!(pointer_count, 0, "Primitive fields should never trigger pointer scan callbacks");
    }

    #[test]
    fn test_scan_object_with_mixed_fields_and_static_bitmap() {
        let mut dummy_type = TypeDescriptor::with_pointer_mask(
            2,
            vec![
                Some("id".into()),
                Some("child1".into()),
                Some("weight".into()),
                Some("child2".into()),
                Some("next".into()),
            ],
            0b01010,
        );

        let mut child1_type = TypeDescriptor::new(10, vec![]);
        let mut child1_obj = ObjectHeader {
            type_ptr: NonNull::new(&mut child1_type).unwrap(),
            gc_state: 0.into(),
        };
        let child1_ptr = NonNull::new(&mut child1_obj).unwrap();

        let mut child2_type = TypeDescriptor::new(20, vec![]);
        let mut child2_obj = ObjectHeader {
            type_ptr: NonNull::new(&mut child2_type).unwrap(),
            gc_state: 0.into(),
        };
        let child2_ptr = NonNull::new(&mut child2_obj).unwrap();

        let total_size = size_of::<ObjectHeader>() + 5 * size_of::<Value>();
        let mut buffer = vec![0u8; total_size];
        let header_ptr = buffer.as_mut_ptr() as *mut ObjectHeader;

        unsafe {
            (*header_ptr).type_ptr = NonNull::new(&mut dummy_type).unwrap();
            (*header_ptr).gc_state = 0.into();
            (*header_ptr).set_field(0, Value::small_int(42));
            (*header_ptr).set_field(1, Value::boxed(child1_ptr));
            (*header_ptr).set_field(2, Value::float(99.9));
            (*header_ptr).set_field(3, Value::boxed(child2_ptr));
            (*header_ptr).set_field(4, Value::null());
        }

        let non_null = NonNull::new(header_ptr).unwrap();
        let mut visited_slots = Vec::new();
        scan_object(non_null, &mut |slot: ValueSlot| {
            visited_slots.push(slot);
        });

        assert_eq!(visited_slots.len(), 2);
        assert_eq!(visited_slots[0].load_object_ptr(), Some(child1_ptr));
        assert_eq!(visited_slots[1].load_object_ptr(), Some(child2_ptr));

        let mut child_relocated_obj = ObjectHeader {
            type_ptr: NonNull::new(&mut child1_type).unwrap(),
            gc_state: 0.into(),
        };
        let child_relocated_ptr = NonNull::new(&mut child_relocated_obj).unwrap();

        visited_slots[0].store_object_ptr(child_relocated_ptr);
        unsafe {
            let field1 = (*header_ptr).get_field(1);
            assert!(matches!(field1, Value::ObjectPtr(_)));
            assert_eq!(field1.as_object_ptr(), Some(child_relocated_ptr));
        }
    }
}