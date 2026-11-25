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

pub mod adoption;
pub mod clock;
pub mod ghost;
pub mod host;
pub mod live;
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
pub mod pixel;
pub mod property;
pub mod retire;
pub mod slots;

use waterui::reactive::Signal;
use waterui_backend_core::Environment;
use waterui_core::layout::{LayoutDirection, layout_direction};
use waterui_structural_transition::{ReduceMotion, TransitionSpec};

/// The `.transition(...)` declaration a leaf carries until a structural
/// site claims it: the spec plus the environment answers resolved where
/// the leaf rendered — each clock samples Reduce Motion at `start`, while
/// the slot direction is frozen at render.
#[derive(Clone)]
pub struct TransitionClaim {
    /// The declared effect and timing.
    pub spec: TransitionSpec,
    /// The environment's Reduce Motion signal at render; sampled at `start`.
    pub reduce_motion: ReduceMotion,
    /// The layout direction at render, resolving leading and trailing.
    pub direction: LayoutDirection,
}

impl TransitionClaim {
    /// Resolves the declaration against the environment it renders under.
    pub fn new(spec: TransitionSpec, env: &Environment) -> Self {
        Self {
            spec,
            reduce_motion: env.get::<ReduceMotion>().cloned().unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use waterui::reactive::{Binding, Signal as _};
    use waterui_backend_core::Environment;
    use waterui_core::animation::Animation;
    use waterui_core::layout::{LayoutDirection, Size};
    use waterui_structural_transition::{
        Edge, PropertyTransition, ReduceMotion, TransitionGeometry, TransitionPhase, TransitionSpec,
    };

    use super::TransitionClaim;

    #[test]
    fn transition_claim_reads_live_reduce_motion_at_start() {
        let setting = Binding::bool(false);
        let mut env = Environment::new();
        env.insert(ReduceMotion::new(setting.clone()));

        let spec = TransitionSpec::new(PropertyTransition::move_in(Edge::Bottom))
            .with_animation(Animation::linear(Duration::from_millis(100)));
        let claim = TransitionClaim::new(spec, &env);
        let geometry =
            TransitionGeometry::new(Size::new(200.0, 80.0), LayoutDirection::LeftToRight);

        let mut full = claim
            .spec
            .start(TransitionPhase::Insertion, claim.reduce_motion.snapshot());
        full.advance(Duration::from_millis(50));
        let full_props = full.properties(geometry);
        assert!(full_props.translation[1] > 1.0);

        setting.set(true);
        let mut reduced = claim
            .spec
            .start(TransitionPhase::Insertion, claim.reduce_motion.snapshot());
        reduced.advance(Duration::from_millis(50));
        let reduced_props = reduced.properties(geometry);
        assert!(reduced_props.translation[1].abs() < 1e-6);
        assert!(reduced_props.opacity < 1.0);
    }
}
