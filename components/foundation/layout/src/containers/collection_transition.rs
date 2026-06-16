//! Opt-in animation for reactive-collection membership changes.

use waterui_core::animation::Animation;
use waterui_core::{Environment, Metadata, View};

/// Environment request to animate reactive-collection membership changes.
///
/// While present in scope, reactive collections (`ForEach` / `List`) animate
/// membership changes: an item fades and grows in when it appears, and fades and
/// collapses out when it disappears, over [`animation`], instead of popping
/// instantly.
///
/// A backend that supports collection transitions reads this from the
/// environment when it renders a collection; backends without support ignore it
/// (the collection still renders correctly, just without the transition).
///
/// [`animation`]: Self::animation
#[derive(Debug, Clone)]
pub struct CollectionTransition {
    /// Timing curve for an item's enter and exit.
    pub animation: Animation,
}

impl CollectionTransition {
    /// Creates a transition with the given animation timing.
    #[must_use]
    pub const fn new(animation: Animation) -> Self {
        Self { animation }
    }
}

/// Wraps `content` so every reactive collection within it animates membership
/// changes with `animation` (see [`CollectionTransition`]).
///
/// Free-function constructor, mirroring [`background`](crate::background) and the
/// other layout composers: it scopes the transition to `content`'s subtree
/// through the environment.
pub fn collection_transition(content: impl View, animation: Animation) -> impl View {
    CollectionTransitionScope { content, animation }
}

struct CollectionTransitionScope<V> {
    content: V,
    animation: Animation,
}

impl<V: View> View for CollectionTransitionScope<V> {
    fn body(self, env: &Environment) -> impl View {
        Metadata::new(
            self.content,
            env.extending(CollectionTransition::new(self.animation)),
        )
    }
}
