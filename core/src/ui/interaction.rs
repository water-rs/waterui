//! Interaction state shared by controls and rendering backends.
//!
//! The central type is [`Disabled`], the environment-driven disabled state
//! for interactive controls. The `.disabled(...)` view modifier installs it,
//! so a container can disable its entire subtree; controls read the state in
//! force at their own position out of the environment they are handed, the
//! same way they read a theme color, and never carry it on their own
//! configuration.

use nami::{Computed, SignalExt, signal::IntoComputed, zip::zip};

use crate::Environment;

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
    use super::Disabled;
    use crate::Environment;
    use nami::{Signal, binding};

    #[test]
    fn resolve_without_scope_returns_local_signal() {
        let env = Environment::new();
        let local = binding(false);
        let resolved = Disabled::resolve(&env, local.clone());
        assert!(!resolved.get());
        local.set(true);
        assert!(resolved.get());
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
        assert!(!resolved.get());
        outer.set(true);
        assert!(resolved.get(), "outer scope must disable the subtree");
        outer.set(false);
        inner.set(true);
        assert!(resolved.get(), "inner scope must disable the subtree");
        inner.set(false);
        assert!(!resolved.get());
    }
}
