use core::{any::Any, cmp::Ordering, fmt};
use std::rc::Rc;

/// Strong identity lease for one retained semantic object.
///
/// The erased owner keeps its allocation alive while the identity is present in
/// a renderer map. That makes pointer identity collision-free even when a
/// collection removes one retained node and allocates its replacement during
/// the same refresh.
#[derive(Clone)]
pub(crate) struct RetainedIdentity {
    _owner: Rc<dyn Any>,
    address: usize,
}

impl RetainedIdentity {
    pub(crate) fn for_rc<T: 'static>(owner: &Rc<T>) -> Self {
        let address = Rc::as_ptr(owner) as usize;
        let owner: Rc<dyn Any> = owner.clone();
        Self {
            _owner: owner,
            address,
        }
    }

    pub(crate) const fn address(&self) -> usize {
        self.address
    }

    /// Whether the retained object is still owned by something other than this
    /// lease.
    ///
    /// A renderer map that keys its entries by identity can use this to keep an
    /// entry alive for exactly as long as the retained tree holds the object,
    /// rather than for as long as it happens to be rendered. A container that
    /// renders one child at a time — tabs, a collapsed split column — retains
    /// every child but flushes only the visible one, so "not flushed this
    /// frame" does not mean "gone".
    ///
    /// This requires that the renderer hold no strong reference to the object
    /// beyond the single lease stored in the map.
    pub(crate) fn is_retained_elsewhere(&self) -> bool {
        Rc::strong_count(&self._owner) > 1
    }
}

impl fmt::Debug for RetainedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RetainedIdentity")
            .field(&self.address)
            .finish()
    }
}

impl PartialEq for RetainedIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl Eq for RetainedIdentity {}

impl PartialOrd for RetainedIdentity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RetainedIdentity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.address.cmp(&other.address)
    }
}
