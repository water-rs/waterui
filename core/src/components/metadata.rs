//! Metadata components for attaching arbitrary data to views.
//!
//! Every metadata key is strict: each backend realizes it or ignores it
//! explicitly in its own code, so a missing realization is a contract
//! violation rather than silently absent behaviour.
//!
//! Metadata can be used to attach arbitrary data to views that will be processed
//! by renderers, such as accessibility attributes, transition effects, or custom
//! rendering instructions.

use alloc::boxed::Box;
use core::any::Any;
use core::any::type_name;
use core::fmt;

use crate::backend::Backend;
use crate::{AnyView, Environment, View};

/// Represents a view that carries additional metadata of type `T`.
///
/// This struct allows attaching arbitrary data to a view component. Every
/// backend handles every key, so a `Metadata` never expands through `body`.
///
/// Metadata is transparent for layout system, it is not a native view.
#[derive(Debug)]
#[must_use]
pub struct Metadata<T: MetadataKey> {
    /// The view content wrapped by this metadata.
    pub content: AnyView,
    /// The metadata value associated with the content.
    pub value: T,
}

/// A marker trait for metadata keys.
pub trait MetadataKey: 'static {}

impl<T: MetadataKey> Metadata<T> {
    /// Creates a new `Metadata` instance with the specified content and value.
    ///
    /// # Arguments
    ///
    /// * `content` - The view to be wrapped with metadata.
    /// * `value` - The metadata value to associate with the content.
    pub fn new(content: impl View, value: T) -> Self {
        Self {
            content: AnyView::new(content),
            value,
        }
    }
}

impl<T: MetadataKey> View for Metadata<T> {
    fn body(self, _env: &Environment) -> impl View {
        never_expands::<T>();
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.metadata(self, env, cx)
    }
}

fn never_expands<T>() {
    unreachable!(
        "metadata {} reached `body`: every backend handles every metadata key",
        type_name::<T>()
    );
}

/// A metadata key that retains a value for its lifetime.
///
/// This is useful for keeping watcher guards, subscriptions, or other values
/// alive as long as the view exists. The retained value is dropped when the
/// view is dropped. It is the one lifetime primitive: a modifier that needs
/// something alive exactly as long as its view is a composite over
/// `Metadata<Retain>`, never a key of its own.
///
/// A value built with [`Retain::resolved`] is constructed from the
/// environment the retained content resolves in — the environment a handler
/// attached to that content sees, so an install on either side of the
/// modifier in the same chain reaches it. Backends call [`Retain::resolve`]
/// with that environment once the content is built, then keep the `Retain`.
pub struct Retain {
    value: Retained,
}

/// Builds a [`Retain::resolved`] value from its content's environment.
type BuildRetained = Box<dyn FnOnce(&Environment) -> Box<dyn Any>>;

/// What a [`Retain`] holds: its value, or the builder that makes it.
enum Retained {
    /// Kept alive, never read.
    Value {
        _kept: Box<dyn Any>,
    },
    Pending(BuildRetained),
}

impl fmt::Debug for Retain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Retain")
            .field("resolved", &matches!(self.value, Retained::Value { .. }))
            .finish()
    }
}

impl MetadataKey for Retain {}

impl Retain {
    /// Creates a new `Retain` from a value.
    pub fn new<T: 'static>(value: T) -> Self {
        Self {
            value: Retained::Value {
                _kept: Box::new(value),
            },
        }
    }

    /// Creates a `Retain` whose value `build` makes from the environment the
    /// retained content resolves in.
    pub fn resolved<T: 'static>(build: impl FnOnce(&Environment) -> T + 'static) -> Self {
        Self {
            value: Retained::Pending(Box::new(move |env| Box::new(build(env)))),
        }
    }

    /// Builds a [`Retain::resolved`] value from `env`, the environment the
    /// retained content resolves in. A value that is already built is kept
    /// as it is.
    pub fn resolve(&mut self, env: &Environment) {
        let pending = core::mem::replace(
            &mut self.value,
            Retained::Value {
                _kept: Box::new(()),
            },
        );
        self.value = match pending {
            Retained::Pending(build) => Retained::Value { _kept: build(env) },
            value @ Retained::Value { .. } => value,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe;
    impl MetadataKey for Probe {}

    #[test]
    #[should_panic(expected = "reached `body`")]
    fn metadata_never_expands() {
        let _ = AnyView::new(Metadata::new((), Probe).body(&Environment::new()));
    }
}
