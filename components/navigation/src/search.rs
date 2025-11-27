/// A search bar component for filtering and finding content.
///
/// This is currently under development and not yet fully implemented.

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchBar<C, F, V> {
    data: C,
    filter: F,
    content: V,
}

/// Trait for search query operations.
///
/// This is currently under development and not yet fully implemented.

trait Query {
    fn query();
}
