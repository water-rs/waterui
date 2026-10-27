//! Structural view transitions in the Apple backend (docs/layout-spec.md §4.5).
//!
//! A `.transition(...)` declaration is stored on the leaf its handler
//! renders as a [`TransitionClaim`]. The structural site that swaps the
//! mount — a `Dynamic` host, a container's id-keyed reconcile, a list
//! row, a navigation page, an anchored overlay — claims it at the root of
//! the mounted subtree: a declaration beneath another modifier is never
//! claimed, matching the rule every backend shares. A declaration placed
//! directly over a conditional is adopted by the `Dynamic` host and
//! applies to each content instance it swaps.
//!
//! Every claimed instance is mounted inside a [`host::TransitionHost`],
//! an ordinary `HostView` the site places. At removal the site marks the
//! host retired before it drops the content's semantics: a `Mounted`
//! whose view lies inside a retired host releases its semantics but keeps
//! its view, so the retired host owns the outgoing native tree as pure
//! visuals. The [`ghost::Ghost`] then drives the exit animation through
//! per-frame sampling on a [`clock::TransitionDriver`] — Core Animation
//! endpoint animations never see a non-linear sample.
//!
//! `LiveChild` is the value every site stores for a child that can carry
//! a claim; `remove` performs the retirement sequence of §1.5 and hands
//! back the ghost the site holds until `on_complete` fires.

/// The panic policy for transition contract violations — states user
/// code can trigger, like an unowned `.transition(...)` declaration or a
/// failed pixel capture. A debug build panics with the message; a
/// release build logs it and the caller continues to the violation's
/// defined end state (a dropped claim, a completed instance), since a
/// GUI framework must not crash a shipped app. Internal invariants —
/// states that cannot occur — keep `unreachable!`/`expect`.
macro_rules! transition_violation {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            panic!($($arg)*);
        } else {
            tracing::error!($($arg)*);
        }
    };
}

pub(crate) use transition_violation;

pub mod clock;
pub mod ghost;
pub mod host;
pub mod live;
pub mod property;
pub mod retire;
pub mod slots;

pub use ghost::Ghost;
pub use live::{LiveChild, Removal};

use waterui::reactive::Signal;
use waterui_backend_core::Environment;
use waterui_core::layout::{LayoutDirection, layout_direction};
use waterui_core::transition::{ReduceMotion, TransitionSpec};

/// The `.transition(...)` declaration a leaf carries until a structural
/// site claims it: the spec plus the environment answers resolved where
/// the leaf rendered — each clock resolves Reduce Motion at `start`, and
/// the slot direction is frozen the same way.
#[derive(Clone)]
pub struct TransitionClaim {
    /// The declared effect and timing.
    pub spec: TransitionSpec,
    /// The environment's Reduce Motion answer at render.
    pub reduce_motion: bool,
    /// The layout direction at render, resolving leading and trailing.
    pub direction: LayoutDirection,
}

impl TransitionClaim {
    /// Resolves the declaration against the environment it renders under.
    pub fn new(spec: TransitionSpec, env: &Environment) -> Self {
        Self {
            spec,
            reduce_motion: env.get::<ReduceMotion>().is_some_and(|flag| flag.0),
            direction: layout_direction(env).snapshot(),
        }
    }
}

impl core::fmt::Debug for TransitionClaim {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransitionClaim").finish_non_exhaustive()
    }
}

/// Environment marker the `.transition(...)` handler inserts for its
/// content: GPU and filtered surfaces read it at render to keep frozen
/// frames for the retirement hand-off (§2.1). Arming is unconditional —
/// an asymmetric transition can still resolve to a blur or pixel phase.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotDemand;
