//! The offer a `.transition(...)` makes to the conditional it sits directly
//! over (docs/layout-spec.md §5.1).
//!
//! The declaration's content is realized with an armed [`AdoptionOffer`] in
//! its environment. The first payload the content reaches takes the offer
//! when it opens its [`RenderContext`](crate::contract::RenderContext): a
//! `Dynamic` adopts the claim for every child it swaps, the `Retain` guard a
//! `watch` puts around one re-arms it for its own content, and any other
//! payload drops it, so the declaration claims the content's leaf instead.
//! Composite views open no context, so the conditional is found through
//! them, as Hydrolysis finds it among its realized nodes.

use alloc::rc::Rc;
use core::cell::Cell;

use super::TransitionClaim;

/// A claim offered to the first payload of a transition's content.
#[derive(Clone)]
pub struct AdoptionOffer(Rc<OfferState>);

struct OfferState {
    claim: TransitionClaim,
    /// Whether the next payload to open a context takes the offer.
    armed: Cell<bool>,
    /// Whether a `Dynamic` adopted the claim.
    adopted: Cell<bool>,
}

impl core::fmt::Debug for AdoptionOffer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AdoptionOffer").finish_non_exhaustive()
    }
}

impl AdoptionOffer {
    /// An armed offer of `claim`.
    #[must_use]
    pub fn new(claim: TransitionClaim) -> Self {
        Self(Rc::new(OfferState {
            claim,
            armed: Cell::new(true),
            adopted: Cell::new(false),
        }))
    }

    /// The offer, when it is still armed; taking it disarms it.
    #[must_use]
    pub fn take(&self) -> Option<Self> {
        self.0.armed.replace(false).then(|| self.clone())
    }

    /// Passes the offer on to the next payload: a `Retain` guard is
    /// transparent to adoption.
    pub fn pass_on(&self) {
        self.0.armed.set(true);
    }

    /// Adopts the claim: the conditional applies it to each child.
    #[must_use]
    pub fn adopt(&self) -> TransitionClaim {
        self.0.adopted.set(true);
        self.0.claim.clone()
    }

    /// Whether a conditional adopted the claim.
    #[must_use]
    pub fn adopted(&self) -> bool {
        self.0.adopted.get()
    }
}
