use std::collections::HashMap;
use std::sync::Arc;

/// An interned name; the [`Interner`] that made it turns it back into text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Symbol(u32);

/// Stores each distinct name once.
#[derive(Default)]
pub struct Interner {
    names: Vec<Arc<str>>,
    lookup: HashMap<Arc<str>, Symbol>,
}

impl Interner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, name: &str) -> Symbol {
        if let Some(&symbol) = self.lookup.get(name) {
            return symbol;
        }
        let symbol = Symbol(u32::try_from(self.names.len()).expect("interner holds more than u32::MAX names"));
        let name: Arc<str> = Arc::from(name);
        self.names.push(Arc::clone(&name));
        self.lookup.insert(name, symbol);
        symbol
    }

    pub fn get(&self, name: &str) -> Option<Symbol> {
        self.lookup.get(name).copied()
    }

    pub fn resolve(&self, symbol: Symbol) -> &str {
        &self.names[symbol.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_names_share_a_symbol() {
        let mut interner = Interner::new();
        let a = interner.intern("name");
        let b = interner.intern("name");
        assert_eq!(a, b);
        assert_eq!(interner.len(), 1);
    }

    #[test]
    fn different_names_get_different_symbols() {
        let mut interner = Interner::new();
        assert_ne!(interner.intern("a"), interner.intern("b"));
    }

    #[test]
    fn resolve_returns_the_original_text() {
        let mut interner = Interner::new();
        let symbol = interner.intern("héllo");
        assert_eq!(interner.resolve(symbol), "héllo");
    }

    #[test]
    fn get_does_not_intern() {
        let mut interner = Interner::new();
        assert_eq!(interner.get("x"), None);
        assert!(interner.is_empty());
        let x = interner.intern("x");
        assert_eq!(interner.get("x"), Some(x));
    }

    #[test]
    fn the_empty_name_is_a_name() {
        let mut interner = Interner::new();
        let symbol = interner.intern("");
        assert_eq!(interner.resolve(symbol), "");
    }
}
