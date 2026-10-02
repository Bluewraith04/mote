use std::collections::HashMap;

/// Register file for one call frame. Operands are `u8`, so a frame has at most 256 registers; registers are reused as soon as they are freed.
pub struct RegisterAllocator {
    var_map: HashMap<String, Vec<(u8, bool)>>,
    scopes: Vec<Vec<(String, bool)>>,
    in_use: [bool; 256],
    high_water: u16,
    overflowed: bool,
    windows: HashMap<String, Window>,
    scope_blocks: Vec<Vec<(u8, usize)>>,
}

/// A struct held in `slots` consecutive registers from `base`, one per field slot.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub base: u8,
    /// The struct's type index.
    pub ty: u16,
    pub slots: usize,
}

impl RegisterAllocator {
    pub fn new() -> Self {
        Self {
            var_map: HashMap::new(),
            scopes: vec![Vec::new()],
            windows: HashMap::new(),
            scope_blocks: vec![Vec::new()],
            in_use: [false; 256],
            high_water: 0,
            overflowed: false,
        }
    }

    /// A fresh allocator with `params` bound to r0, r1, … in order.
    pub(crate) fn with_params(params: &[String]) -> Self {
        let mut alloc = Self::new();
        for p in params {
            let reg = alloc.alloc_temp();
            alloc.record_binding(p, reg, true);
        }
        alloc
    }

    /// Opens a nested lexical scope; locals bound after it are released by the matching `exit_scope`.
    pub fn scope_depth(&self) -> usize {
        self.scopes.len()
    }

    pub(crate) fn enter_scope(&mut self) {
        self.scopes.push(Vec::new());
        self.scope_blocks.push(Vec::new());
    }

    /// Binds `name` to a struct held in the `slots` registers from `base`; the current scope owns them.
    pub(crate) fn bind_window(&mut self, name: &str, base: u8, slots: usize, ty: u16) {
        self.windows.insert(name.to_string(), Window { base, ty, slots });
        self.scope_blocks.last_mut().unwrap().push((base, slots));
    }

    pub fn window(&self, name: &str) -> Option<Window> {
        self.windows.get(name).copied()
    }

    /// Closes the innermost scope: drops its bindings and frees the registers it owned. The root scope is never closed.
    pub(crate) fn exit_scope(&mut self) {
        if self.scopes.len() <= 1 {
            return;
        }
        let scope = self.scopes.pop().unwrap();
        for (base, slots) in self.scope_blocks.pop().unwrap() {
            for r in base as usize..base as usize + slots {
                self.in_use[r] = false;
            }
        }
        for (name, owns) in scope.into_iter().rev() {
            let freed_reg = match self.var_map.get_mut(&name) {
                Some(stack) => {
                    let reg = stack.pop().map(|(r, _)| r);
                    if stack.is_empty() {
                        self.var_map.remove(&name);
                    }
                    reg
                }
                None => None,
            };
            if let (true, Some(reg)) = (owns, freed_reg)
                && !self.reg_is_bound(reg) {
                    self.in_use[reg as usize] = false;
                }
        }
    }

    fn record_binding(&mut self, name: &str, reg: u8, owns: bool) {
        self.var_map.entry(name.to_string()).or_default().push((reg, false));
        self.scopes.last_mut().unwrap().push((name.to_string(), owns));
    }

    fn reg_is_bound(&self, reg: u8) -> bool {
        self.var_map.values().any(|stack| stack.iter().any(|&(r, _)| r == reg))
    }

    fn claim(&mut self, reg: u8) {
        self.in_use[reg as usize] = true;
        self.high_water = self.high_water.max(reg as u16 + 1);
    }

    /// The lowest free register; on exhaustion sets `overflowed` and returns 255.
    pub(crate) fn alloc_temp(&mut self) -> u8 {
        match self.in_use.iter().position(|&used| !used) {
            Some(r) => {
                let r = r as u8;
                self.claim(r);
                r
            }
            None => {
                self.overflowed = true;
                255
            }
        }
    }

    /// Reserves `n` consecutive free registers, the window a `CALL` copies its arguments from.
    pub(crate) fn alloc_block(&mut self, n: usize) -> u8 {
        if n == 0 {
            return self.alloc_temp();
        }
        if n > 256 {
            self.overflowed = true;
            return 0;
        }
        for base in 0..=(256 - n) {
            if self.in_use[base..base + n].iter().all(|&used| !used) {
                for r in base..base + n {
                    self.claim(r as u8);
                }
                return base as u8;
            }
        }
        self.overflowed = true;
        0
    }

    pub fn free_block(&mut self, base: u8, n: usize) {
        for k in 0..n {
            if let Some(r) = base.checked_add(k as u8) {
                self.free_temp(r);
            }
        }
    }

    /// Release a temp. No-op for a register bound to a named variable.
    pub(crate) fn free_temp(&mut self, reg: u8) {
        if self.reg_is_bound(reg) {
            return;
        }
        self.in_use[reg as usize] = false;
    }

    /// Allocate a fresh register for a named local and emit no code.
    pub(crate) fn alloc_var(&mut self, name: &str) -> u8 {
        let reg = self.alloc_temp();
        self.record_binding(name, reg, true);
        reg
    }

    /// Promotes a live temp into a named local without a `MOVE`; the current scope owns it.
    pub(crate) fn bind_var(&mut self, name: &str, reg: u8) {
        self.claim(reg);
        self.record_binding(name, reg, true);
    }

    /// Binds `name` to a register the current scope does not own, as `match` pattern bindings alias the subject.
    pub(crate) fn alias_var(&mut self, name: &str, reg: u8) {
        self.claim(reg);
        self.record_binding(name, reg, false);
    }

    pub fn get_var(&self, name: &str) -> Option<u8> {
        self.var_map.get(name).and_then(|stack| stack.last()).map(|&(r, _)| r)
    }

    /// Marks `name`'s innermost binding as holding a shared cell, not the value.
    pub(crate) fn mark_cell(&mut self, name: &str) {
        if let Some(top) = self.var_map.get_mut(name).and_then(|stack| stack.last_mut()) {
            top.1 = true;
        }
    }

    /// `name`'s innermost binding holds a cell.
    pub(crate) fn is_cell(&self, name: &str) -> bool {
        self.var_map.get(name).and_then(|stack| stack.last()).is_some_and(|&(_, cell)| cell)
    }

    pub(crate) fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Registers the frame needs (`CodeObject::register_count`), at least 4.
    pub(crate) fn total_registers(&self) -> u16 {
        self.high_water.max(4)
    }
}

impl Default for RegisterAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_freed_registers_regardless_of_order() {
        let mut a = RegisterAllocator::new();
        let r0 = a.alloc_temp();
        let r1 = a.alloc_temp();
        let r2 = a.alloc_temp();
        assert_eq!((r0, r1, r2), (0, 1, 2));
        a.free_temp(r1);
        a.free_temp(r0);
        assert_eq!(a.alloc_temp(), 0);
        assert_eq!(a.alloc_temp(), 1);
        assert_eq!(a.alloc_temp(), 3);
    }

    #[test]
    fn never_hands_out_a_live_register() {
        let mut a = RegisterAllocator::new();
        let live = a.alloc_temp();
        for _ in 0..50 {
            assert_ne!(a.alloc_temp(), live);
        }
    }

    #[test]
    fn alloc_block_is_contiguous_and_above_live_temps() {
        let mut a = RegisterAllocator::new();
        let _ = a.alloc_temp();
        let _ = a.alloc_temp();
        let base = a.alloc_block(3);
        assert_eq!(base, 2);
        for _ in 0..5 {
            let t = a.alloc_temp();
            assert!(!(base..base + 3).contains(&t));
        }
        a.free_block(base, 3);
        assert_eq!(a.alloc_block(2), 2);
    }

    #[test]
    fn params_take_r0_upward() {
        let a = RegisterAllocator::with_params(&["a".into(), "b".into(), "c".into()]);
        assert_eq!(a.get_var("a"), Some(0));
        assert_eq!(a.get_var("b"), Some(1));
        assert_eq!(a.get_var("c"), Some(2));
    }

    #[test]
    fn bound_variables_are_not_freed_by_free_temp() {
        let mut a = RegisterAllocator::new();
        let t = a.alloc_temp();
        a.bind_var("x", t);
        a.free_temp(t);
        assert_eq!(a.alloc_temp(), 1);
        assert_eq!(a.get_var("x"), Some(t));
    }

    #[test]
    fn exit_scope_frees_owned_locals_and_restores_shadowed_bindings() {
        let mut a = RegisterAllocator::new();
        let outer = a.alloc_temp();
        a.bind_var("x", outer);
        a.enter_scope();
        let t = a.alloc_temp();
        a.bind_var("x", t);
        assert_eq!(a.get_var("x"), Some(t));
        a.enter_scope();
        let y = a.alloc_temp();
        a.bind_var("y", y);
        a.exit_scope();
        assert_eq!(a.alloc_temp(), y);
        a.free_temp(y);
        a.exit_scope();
        assert_eq!(a.get_var("x"), Some(outer));
        assert_eq!(a.alloc_temp(), t);
    }

    #[test]
    fn exit_scope_keeps_registers_held_by_an_outer_binding() {
        let mut a = RegisterAllocator::new();
        let subj = a.alloc_temp();
        a.enter_scope();
        a.alias_var("bound", subj);
        a.exit_scope();
        assert_ne!(a.alloc_temp(), subj);
    }

    #[test]
    fn root_scope_cannot_be_exited() {
        let mut a = RegisterAllocator::new();
        a.exit_scope();
        a.exit_scope();
        let r = a.alloc_temp();
        assert_eq!(r, 0);
    }

    #[test]
    fn flags_overflow_instead_of_wrapping() {
        let mut a = RegisterAllocator::new();
        for _ in 0..256 {
            a.alloc_temp();
        }
        assert!(!a.overflowed());
        let _ = a.alloc_temp();
        assert!(a.overflowed());
    }
}
