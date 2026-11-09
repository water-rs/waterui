//! Interaction state shared by controls and rendering backends.
//!
//! [`Disabled`] is the environment-driven disabled state for interactive
//! controls. The `.disabled(...)` view modifier installs it, so a container can
//! disable its entire subtree; controls read the state in force at their own
//! position out of the environment they are handed, the same way they read a
//! theme color, and never carry it on their own configuration.
//!
//! [`InteractionState`] is the set of states an interactive control is in at a
//! moment — hovered, focused, pressed and so on. Theme styles describe
//! per-state values with [`StateValue`], and a view that draws its own chrome
//! learns the state through [`InteractionReport`].

use alloc::vec::Vec;

use nami::{Computed, SignalExt, signal::IntoComputed, zip::zip};

use crate::{Environment, metadata::MetadataKey};

bitflags::bitflags! {
    /// The states an interactive control is in at one moment.
    ///
    /// The flags combine: a selected control can be hovered and pressed at
    /// once. The empty set is the resting state.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct InteractionState: u8 {
        /// A pointer is over the control.
        const HOVERED = 1 << 0;
        /// The control holds keyboard focus that should be shown, the way
        /// `:focus-visible` is: focus reached by keyboard, not by a click.
        const FOCUSED = 1 << 1;
        /// A pointer or key is pressing the control.
        const PRESSED = 1 << 2;
        /// The control is being dragged.
        const DRAGGED = 1 << 3;
        /// The control is selected, through [`Selected`].
        const SELECTED = 1 << 4;
        /// The control is disabled, through [`Disabled`].
        const DISABLED = 1 << 5;
    }
}

/// A styling value that differs by [`InteractionState`].
///
/// It holds a resting value and an ordered list of overrides, each keyed by
/// the states it requires. [`Self::resolve`] returns the first override whose
/// states are all present, so the order the overrides are added in is their
/// precedence — the author states it rather than the framework guessing it:
///
/// ```rust
/// use waterui_core::interaction::{InteractionState, StateValue};
///
/// let radius = StateValue::new(8.0)
///     .when(InteractionState::DRAGGED, 16.0)
///     .when(InteractionState::PRESSED, 16.0)
///     .when(InteractionState::FOCUSED, 12.0)
///     .when(InteractionState::HOVERED, 12.0);
/// assert_eq!(*radius.resolve(InteractionState::HOVERED | InteractionState::PRESSED), 16.0);
/// assert_eq!(*radius.resolve(InteractionState::empty()), 8.0);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateValue<T> {
    resting: T,
    overrides: Vec<(InteractionState, T)>,
}

impl<T> StateValue<T> {
    /// A value that is `resting` in every state until overrides are added.
    #[must_use]
    pub const fn new(resting: T) -> Self {
        Self {
            resting,
            overrides: Vec::new(),
        }
    }

    /// Uses `value` whenever every state in `states` is present, unless an
    /// override added earlier already matched.
    #[must_use]
    pub fn when(mut self, states: InteractionState, value: T) -> Self {
        self.overrides.push((states, value));
        self
    }

    /// The value for `state`.
    #[must_use]
    pub fn resolve(&self, state: InteractionState) -> &T {
        self.overrides
            .iter()
            .find(|(required, _)| state.contains(*required))
            .map_or(&self.resting, |(_, value)| value)
    }

    /// The resting value.
    #[must_use]
    pub const fn resting(&self) -> &T {
        &self.resting
    }
}

impl<T> From<T> for StateValue<T> {
    fn from(resting: T) -> Self {
        Self::new(resting)
    }
}

/// Asks the backend to report an interactive control's state into a binding,
/// as `.interaction_state(...)` installs it.
///
/// The backend writes the state of the outermost interactive control at or
/// inside the view this is installed on, every time it changes. Chrome the
/// control does not draw itself follows the control through the binding: a
/// chip's outline, a split button's half, a floating surface's elevation. The
/// binding stays at the resting state while no interactive control is there.
#[derive(Debug, Clone)]
pub struct InteractionReport(pub nami::Binding<InteractionState>);

impl crate::plugin::Plugin for InteractionReport {}

/// Marks the control it modifies as selected, as `.selected(...)` installs it.
///
/// A selected control adds [`InteractionState::SELECTED`] to its state, so its
/// style's selected values apply, and is announced as selected by assistive
/// technology. Unlike [`Disabled`] it is not inherited: it applies to the
/// interactive control it modifies, not to controls nested inside it.
#[derive(Debug, Clone)]
pub struct Selected(pub Computed<bool>);

impl MetadataKey for Selected {}

/// Environment-driven disabled state for interactive controls.
///
/// A control inside a disabled subtree must neither respond to input nor
/// render as interactive. Nested `.disabled(...)` scopes combine with a
/// logical OR: a subtree stays disabled while *any* enclosing scope is
/// disabled, tracked reactively without rebuilding the subtree.
#[derive(Debug, Clone)]
pub struct Disabled(Computed<bool>);

impl Disabled {
    /// Creates a disabled state from a reactive signal.
    #[must_use]
    pub fn new(disabled: impl IntoComputed<bool>) -> Self {
        Self(disabled.into_computed())
    }

    /// The reactive disabled signal carried by this scope.
    #[must_use]
    pub const fn signal(&self) -> &Computed<bool> {
        &self.0
    }

    /// OR-combines `local` with the disabled state inherited from `env`.
    ///
    /// Controls call this from their config `resolve` so an explicit
    /// per-control disabled signal and an enclosing `.disabled(...)` scope
    /// both take effect.
    #[must_use]
    pub fn resolve(env: &Environment, local: impl IntoComputed<bool>) -> Computed<bool> {
        let local = local.into_computed();
        match env.get::<Self>() {
            Some(inherited) => zip(inherited.0.clone(), local)
                .map(|(inherited, local)| inherited || local)
                .into_computed(),
            None => local,
        }
    }

    /// Installs a disabled scope into `env`, OR-combined with any scope
    /// already inherited from an enclosing `.disabled(...)`.
    pub fn install(env: &mut Environment, disabled: impl IntoComputed<bool>) {
        let merged = Self::resolve(env, disabled);
        env.insert(Self(merged));
    }
}

#[cfg(test)]
mod tests {
    use super::{Disabled, InteractionState, StateValue};
    use crate::Environment;
    use nami::{Signal, binding};

    #[test]
    fn state_value_precedence_is_insertion_order() {
        let value = StateValue::new("resting")
            .when(InteractionState::HOVERED, "hovered")
            .when(InteractionState::PRESSED, "pressed");
        // An earlier override wins over a later one that also matches.
        assert_eq!(
            *value.resolve(InteractionState::HOVERED | InteractionState::PRESSED),
            "hovered"
        );
        assert_eq!(*value.resolve(InteractionState::PRESSED), "pressed");
        assert_eq!(*value.resolve(InteractionState::empty()), "resting");
    }

    #[test]
    fn state_value_combined_flags_require_every_state() {
        let value = StateValue::new(0)
            .when(InteractionState::SELECTED | InteractionState::HOVERED, 2)
            .when(InteractionState::SELECTED, 1);
        // The combination matches only when every required flag is present.
        assert_eq!(*value.resolve(InteractionState::SELECTED), 1);
        assert_eq!(
            *value.resolve(InteractionState::SELECTED | InteractionState::HOVERED),
            2
        );
        assert_eq!(*value.resolve(InteractionState::HOVERED), 0);
    }

    #[test]
    fn resolve_without_scope_returns_local_signal() {
        let env = Environment::new();
        let local = binding(false);
        let resolved = Disabled::resolve(&env, local.clone());
        assert!(!resolved.snapshot());
        local.set(true);
        assert!(resolved.snapshot());
    }

    #[test]
    fn nested_scopes_or_combine_reactively() {
        let mut env = Environment::new();
        let outer = binding(false);
        Disabled::install(&mut env, outer.clone());
        let inner = binding(false);
        Disabled::install(&mut env, inner.clone());

        let resolved = env
            .get::<Disabled>()
            .expect("installed disabled scope must be present")
            .signal()
            .clone();
        assert!(!resolved.snapshot());
        outer.set(true);
        assert!(resolved.snapshot(), "outer scope must disable the subtree");
        outer.set(false);
        inner.set(true);
        assert!(resolved.snapshot(), "inner scope must disable the subtree");
        inner.set(false);
        assert!(!resolved.snapshot());
    }
}
