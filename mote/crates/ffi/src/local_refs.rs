use isa::value::Value;

/// Scoped local reference storage for values passed to or created within a single native call frame.
#[derive(Debug, Default, Clone)]
pub struct LocalReferenceScope {
    values: Vec<Value>,
}

impl LocalReferenceScope {
    pub fn new() -> Self {
        Self {
            values: Vec::with_capacity(32),
        }
    }

    #[inline(always)]
    pub fn push(&mut self, val: Value) -> usize {
        let idx = self.values.len();
        self.values.push(val);
        idx
    }

    #[inline(always)]
    pub fn get(&self, idx: usize) -> Option<&Value> {
        self.values.get(idx)
    }

    #[inline(always)]
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut Value> {
        self.values.get_mut(idx)
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.values.clear();
    }
}
