//! Visitor abstraction over a filter's reactive parameters.
//!
//! Each filter implementation calls `visitor.visit(idx, &self.field)` for
//! every `FilterParam` field it owns; the runtime supplies a visitor that
//! installs animation watchers indexed by `idx`.

use crate::FilterParam;

/// Sink for [`crate::Filter::visit_signals`] traversal.
pub trait SignalVisitor {
    /// Visit a single parameter at `param_index` in the flattened parameter
    /// layout. Filters with `N` parameters call this `N` times in declared
    /// order.
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P);
}

/// Internal helper that wraps a [`SignalVisitor`] and shifts every visited
/// parameter index by `base`. Used by [`crate::Chain`] to emit child indices
/// relative to the chain's flattened layout.
pub(crate) struct OffsetVisitor<'a, V: SignalVisitor + ?Sized> {
    inner: &'a mut V,
    base: usize,
}

impl<'a, V: SignalVisitor + ?Sized> OffsetVisitor<'a, V> {
    pub(crate) fn new(inner: &'a mut V, base: usize) -> Self {
        Self { inner, base }
    }
}

impl<V: SignalVisitor + ?Sized> SignalVisitor for OffsetVisitor<'_, V> {
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P) {
        self.inner.visit(self.base + param_index, param);
    }
}
