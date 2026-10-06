//! A fixed set of locks keyed by id, so per-object locks never accumulate.

use std::sync::Mutex;

const STRIPES: usize = 64;

pub(crate) struct LockStripes([Mutex<()>; STRIPES]);

impl LockStripes {
    pub(crate) fn new() -> Self {
        Self(std::array::from_fn(|_| Mutex::new(())))
    }

    pub(crate) fn get(&self, id: u64) -> &Mutex<()> {
        &self.0[(id % STRIPES as u64) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_share_a_fixed_set_of_locks() {
        let locks = LockStripes::new();
        assert!(std::ptr::eq(locks.get(1), locks.get(1 + STRIPES as u64)));
        assert!(!std::ptr::eq(locks.get(1), locks.get(2)));
    }
}
