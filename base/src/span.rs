/// A byte range in the source map's single offset space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        assert!(start <= end, "span starts after it ends");
        Self { start, end }
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    /// The smallest span covering both.
    pub fn merge(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_emptiness() {
        assert_eq!(Span::new(3, 8).len(), 5);
        assert!(Span::new(4, 4).is_empty());
        assert!(!Span::new(4, 5).is_empty());
    }

    #[test]
    fn contains_excludes_the_end() {
        let span = Span::new(2, 5);
        assert!(span.contains(2));
        assert!(span.contains(4));
        assert!(!span.contains(5));
        assert!(!span.contains(1));
    }

    #[test]
    fn merge_covers_both_in_either_order() {
        let a = Span::new(2, 4);
        let b = Span::new(7, 9);
        assert_eq!(a.merge(b), Span::new(2, 9));
        assert_eq!(b.merge(a), Span::new(2, 9));
    }

    #[test]
    #[should_panic(expected = "span starts after it ends")]
    fn new_rejects_a_reversed_range() {
        Span::new(5, 4);
    }
}
