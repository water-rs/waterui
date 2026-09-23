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
