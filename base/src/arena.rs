use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

/// A typed index into an [`Arena`].
pub struct Id<T> {
    index: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    pub(crate) fn from_index(index: u32) -> Self {
        Self { index, marker: PhantomData }
    }

    pub fn index(self) -> usize {
        self.index as usize
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for Id<T> {}

impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.index.cmp(&other.index)
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.index)
    }
}

/// Owns values of one type and hands out their ids.
#[derive(Clone)]
pub struct Arena<T> {
    items: Vec<T>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self, item: T) -> Id<T> {
        let index = u32::try_from(self.items.len()).expect("arena holds more than u32::MAX items");
        self.items.push(item);
        Id::from_index(index)
    }

    pub fn get(&self, id: Id<T>) -> Option<&T> {
        self.items.get(id.index())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    pub fn iter(&self) -> impl Iterator<Item = (Id<T>, &T)> {
        self.items.iter().enumerate().map(|(index, item)| (Id::from_index(index as u32), item))
    }
}

impl<T> Index<Id<T>> for Arena<T> {
    type Output = T;

    fn index(&self, id: Id<T>) -> &T {
        &self.items[id.index()]
    }
}

impl<T> IndexMut<Id<T>> for Arena<T> {
    fn index_mut(&mut self, id: Id<T>) -> &mut T {
        &mut self.items[id.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_returns_ids_in_order() {
        let mut arena = Arena::new();
        let a = arena.alloc("a");
        let b = arena.alloc("b");
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
        assert_eq!(arena[a], "a");
        assert_eq!(arena[b], "b");
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn ids_are_copy_for_any_item_type() {
        let mut arena = Arena::new();
        let id = arena.alloc(String::from("owned"));
        let copy = id;
        assert_eq!(id, copy);
        assert!(id <= copy);
    }

    #[test]
    fn index_mut_changes_the_item() {
        let mut arena = Arena::new();
        let id = arena.alloc(1);
        arena[id] += 4;
        assert_eq!(arena[id], 5);
    }

    #[test]
    fn get_is_none_for_an_id_from_a_longer_arena() {
        let mut long = Arena::new();
        long.alloc(0);
        let far = long.alloc(1);
        let short: Arena<i32> = Arena::new();
        assert_eq!(short.get(far), None);
    }

    #[test]
    fn iter_yields_ids_with_items() {
        let mut arena = Arena::new();
        arena.alloc('x');
        arena.alloc('y');
        let seen: Vec<_> = arena.iter().map(|(id, c)| (id.index(), *c)).collect();
        assert_eq!(seen, [(0, 'x'), (1, 'y')]);
    }
}
