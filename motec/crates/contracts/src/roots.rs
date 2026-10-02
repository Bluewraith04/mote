use crate::slot::ValueSlot;

/// Reports every live root slot; a slot need only stay valid for its one visitor call.
pub trait RootSource {
    fn visit_roots(&mut self, visitor: &mut dyn FnMut(ValueSlot));
}
