use core::{any::Any, cmp::Ordering, fmt};
use std::rc::{Rc, Weak};

use rustc_hash::{FxHashMap, FxHashSet};

use super::HydrolysisRenderer;
use crate::renderer::mount::RetainedScopes;

/// Strong identity lease for one retained semantic object.
///
/// The erased owner keeps its allocation alive while the identity is present in
/// a renderer map. That makes pointer identity collision-free even when a
/// collection removes one retained node and allocates its replacement during
/// the same refresh.
#[derive(Clone)]
pub struct RetainedIdentity {
    owner: Rc<dyn Any>,
    address: usize,
}

impl RetainedIdentity {
    pub(crate) fn for_rc<T: 'static>(owner: &Rc<T>) -> Self {
        let address = Rc::as_ptr(owner) as usize;
        let owner: Rc<dyn Any> = owner.clone();
        Self { owner, address }
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
        Rc::strong_count(&self.owner) > 1
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

/// Address reservations for the owners that key renderer-local animation
/// slots.
///
/// A per-view slot is keyed on its owner's `Rc` address, and the animation
/// controller retires a slot only at the end of the rebuild frame that stops
/// binding it. A structural patch can drop an owner and allocate its
/// replacement earlier in that same frame; were the freed address reused, the
/// replacement would bind the dead owner's slot and inherit its track. Each
/// pin is a `Weak`, which keeps the owner's allocation reserved without
/// keeping the owner alive, and the pins retire in lockstep with the slots,
/// so no address is reissued while a slot keyed on it can still be bound.
#[derive(Debug, Default)]
pub struct AnimationOwnerPins {
    /// One reservation per owner whose slots the controller may still hold.
    pinned: FxHashMap<usize, Weak<dyn Any>>,
    /// Owners bound since the controller's last `begin_rebuild_frame` —
    /// exactly the owners whose slots survive its next retirement.
    bound: FxHashSet<usize>,
}

impl AnimationOwnerPins {
    /// Reserves `owner`'s allocation and returns the address its slots key on.
    pub(crate) fn pin<T: 'static>(&mut self, owner: &Rc<T>) -> usize {
        let address = Rc::as_ptr(owner) as usize;
        self.bound.insert(address);
        self.pinned.entry(address).or_insert_with(|| {
            let pin: Weak<dyn Any> = Rc::<T>::downgrade(owner);
            pin
        });
        address
    }

    /// Mirrors the controller's `begin_rebuild_frame`.
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.bound.clear();
    }

    /// Mirrors the controller's slot retirement: releases every owner not
    /// bound since [`Self::begin_rebuild_frame`].
    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.pinned
            .retain(|address, _| self.bound.contains(address));
    }
}

impl HydrolysisRenderer {
    /// Pushes the retained node whose subtree is about to flush onto the
    /// render owner chain — the ancestry input registration reads. The
    /// accessibility builder gets the same push for its semantic-key owner.
    ///
    /// Unlike [`Self::push_input_owner`], every `emit_accessibility` owner
    /// push pairs with this on the flush path, so both stacks stay balanced.
    pub(crate) fn push_render_owner(&mut self, owner: &Rc<()>) {
        self.owner_stack.push(RetainedIdentity::for_rc(owner));
        self.core
            .record_scope(|scopes| scopes.push_render_owner(owner));
        #[cfg(feature = "accessibility")]
        self.accessibility.push_owner(owner);
    }

    /// Pops the owner [`Self::push_render_owner`] pushed.
    pub(crate) fn pop_render_owner(&mut self) {
        self.owner_stack
            .pop()
            .expect("hydrolysis render owner stack underflow");
        self.core.record_scope(RetainedScopes::pop_render_owner);
        #[cfg(feature = "accessibility")]
        self.accessibility.pop_owner();
    }

    /// Pushes an owner only input ancestry sees: marks where a view begins —
    /// a retained sub-view's root, the view a row's press belongs to —
    /// without joining the accessibility owner chain.
    pub(crate) fn push_input_owner(&mut self, owner: &Rc<()>) {
        self.owner_stack.push(RetainedIdentity::for_rc(owner));
        self.core
            .record_scope(|scopes| scopes.push_input_owner(owner));
    }

    /// Pops the owner [`Self::push_input_owner`] pushed.
    pub(crate) fn pop_input_owner(&mut self) {
        self.owner_stack
            .pop()
            .expect("hydrolysis input owner stack underflow");
        self.core.record_scope(RetainedScopes::pop_input_owner);
    }
}

#[cfg(test)]
mod tests {
    use super::AnimationOwnerPins;
    use std::rc::Rc;

    #[test]
    fn owner_pins_release_exactly_the_owners_a_rebuild_left_unbound() {
        let (kept, dropped) = (Rc::new(0_u8), Rc::new(0_u8));
        let mut pins = AnimationOwnerPins::default();
        pins.pin(&kept);
        pins.pin(&dropped);
        assert_eq!((Rc::weak_count(&kept), Rc::weak_count(&dropped)), (1, 1));

        pins.begin_rebuild_frame();
        pins.pin(&kept);
        pins.finish_rebuild_frame();
        assert_eq!((Rc::weak_count(&kept), Rc::weak_count(&dropped)), (1, 0));
    }
}
