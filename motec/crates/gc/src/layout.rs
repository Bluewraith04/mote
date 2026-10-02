use crate::object_model::{get_object_size, HEADER_SIZE};
use crate::scanning::scan_object;
use contracts::{TypeLayout, ValueSlot};
use isa::value::{ObjectHeader, Value};
use std::mem::size_of;
use std::ptr::NonNull;

/// The object layouts (declared fields, strings, raw buffers, backings) as a `TypeLayout`.
pub struct StandardLayout;

impl TypeLayout for StandardLayout {
    fn slot_count(&self, object: NonNull<ObjectHeader>) -> usize {
        (get_object_size(object) - HEADER_SIZE) / size_of::<Value>()
    }

    fn visit_refs(&self, object: NonNull<ObjectHeader>, visitor: &mut dyn FnMut(ValueSlot)) {
        scan_object(object, &mut |slot: ValueSlot| visitor(slot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use isa::value::TypeDescriptor;

    fn with_object(fields: usize, ptr_fields: &[usize], check: impl FnOnce(NonNull<ObjectHeader>)) {
        let mask = ptr_fields.iter().fold(0u64, |m, &i| m | (1 << i));
        let mut ty = TypeDescriptor::with_pointer_mask(
            1,
            (0..fields).map(|i| Some(format!("f{i}"))).collect(),
            mask,
        );
        let mut buf = vec![0u64; (HEADER_SIZE + fields * size_of::<Value>()) / 8 + 1];
        let header = buf.as_mut_ptr() as *mut ObjectHeader;
        unsafe {
            (*header).type_ptr = NonNull::new(&mut ty).unwrap();
            let obj = NonNull::new(header).unwrap();
            for i in 0..fields {
                *(*header).field_ptr(i) = if ptr_fields.contains(&i) {
                    Value::boxed(obj)
                } else {
                    Value::small_int(i as i64)
                };
            }
            check(obj);
        }
    }

    fn with_counted(ty: &mut TypeDescriptor, slots: usize, count: u64, check: impl FnOnce(NonNull<ObjectHeader>)) {
        let mut buf = vec![0u64; (HEADER_SIZE + slots * size_of::<Value>()) / 8 + 1];
        let header = buf.as_mut_ptr() as *mut ObjectHeader;
        unsafe {
            (*header).type_ptr = NonNull::new(ty).unwrap();
            let obj = NonNull::new(header).unwrap();
            *(*header).field_ptr(0) = Value::uint(count);
            for i in 1..slots {
                *(*header).field_ptr(i) = Value::boxed(obj);
            }
            check(obj);
        }
    }

    #[test]
    fn test_pointer_backing_traces_count_slots() {
        let mut ty = TypeDescriptor::intrinsic_backing(isa::value::BACKING_TYPE_ID);
        with_counted(&mut ty, 6, 3, |obj| {
            assert_eq!(StandardLayout.slot_count(obj), 4);
            let mut n = 0;
            StandardLayout.visit_refs(obj, &mut |_| n += 1);
            assert_eq!(n, 3);
        });
    }

    #[test]
    fn test_map_backing_traces_every_slot_of_every_entry() {
        let mut ty = TypeDescriptor::intrinsic_backing(isa::value::MAP_BACKING_TYPE_ID);
        with_counted(&mut ty, 9, 4, |obj| {
            assert_eq!(StandardLayout.slot_count(obj), 9);
            let mut n = 0;
            StandardLayout.visit_refs(obj, &mut |_| n += 1);
            assert_eq!(n, 8);
        });
    }

    #[test]
    fn test_raw_byte_objects_are_sized_from_length_and_never_traced() {
        for mut ty in [
            TypeDescriptor::string_type(),
            TypeDescriptor::intrinsic_backing(isa::value::BYTES_BACKING_TYPE_ID),
        ] {
            with_counted(&mut ty, 4, 17, |obj| {
                assert_eq!(StandardLayout.slot_count(obj), 3);
                let mut n = 0;
                StandardLayout.visit_refs(obj, &mut |_| n += 1);
                assert_eq!(n, 0);
            });
        }
    }

    #[test]
    fn test_layout_sizes_and_reports_only_reference_slots() {
        with_object(5, &[1, 3], |obj| {
            let layout: &dyn TypeLayout = &StandardLayout;
            assert_eq!(layout.slot_count(obj), 5);
            let mut seen = Vec::new();
            layout.visit_refs(obj, &mut |s| seen.push(s.as_ptr()));
            assert_eq!(seen.len(), 2);
        });
    }

    #[test]
    fn test_layout_of_a_primitive_only_object_reports_no_references() {
        with_object(3, &[], |obj| {
            let mut n = 0;
            StandardLayout.visit_refs(obj, &mut |_| n += 1);
            assert_eq!(n, 0);
            assert_eq!(StandardLayout.slot_count(obj), 3);
        });
    }
}
