//! The resolver pattern powers WaterUI's dynamic configuration story.
//!
//! Instead of hard-coding primitive values, UI-facing types expose lightweight
//! wrappers that implement [`Resolvable`]. When a widget needs an actual value,
//! it calls [`Resolvable::resolve`] with an [`Environment`], receiving a
//! [`Signal`](nami::Signal) that stays up-to-date as the environment changes.
//!
//! Two prominent examples are:
//! - Fonts (`components/text/src/font.rs`), where `Font` resolves to a
//!   `ResolvedFont` so typography can react to user themes or platform scales.
//! - Colors (`utils/color/src/lib.rs`), where `Color` resolves into
//!   `ResolvedColor`, letting themes swap palettes, blend values, or add opacity
//!   without rebuilding widgets.
//!
//! This module defines the abstractions—[`Resolvable`], [`AnyResolvable`],
//! and [`Map`]—that make it easy for domain crates to plug into the same flow.

use alloc::boxed::Box;
use core::fmt::Debug;

use nami::{Computed, Signal, SignalExt};

use crate::Environment;

/// A trait for types that can be resolved to a value in a given environment.
///
/// This trait enables reactive values that depend on environmental context.
///
/// In convention, Resolvable type should have a same output for a same environment.
pub trait Resolvable: Debug + Clone {
    /// The type of the resolved value.
    type Resolved;
    /// Resolves this value in the given environment, returning a signal.
    ///
    /// # Arguments
    /// * `env` - The environment to resolve in
    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved>;
}

trait ResolvableImpl<T>: Debug {
    fn resolve(&self, env: &Environment) -> Computed<T>;
    fn clone_box(&self) -> Box<dyn ResolvableImpl<T>>;
}

impl<R: Resolvable + 'static> ResolvableImpl<R::Resolved> for R {
    fn resolve(&self, env: &Environment) -> Computed<R::Resolved> {
        self.resolve(env).computed()
    }

    fn clone_box(&self) -> Box<dyn ResolvableImpl<R::Resolved>> {
        Box::new(self.clone())
    }
}

/// A type-erased wrapper for any resolvable value.
///
/// This allows storing resolvable values of different types in a uniform way.
#[derive(Debug)]
pub struct AnyResolvable<T> {
    inner: Box<dyn ResolvableImpl<T>>,
}

impl<T> Resolvable for AnyResolvable<T>
where
    T: 'static + Debug,
{
    type Resolved = T;
    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        self.inner.resolve(env)
    }
}

impl<T> Clone for AnyResolvable<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_box(),
        }
    }
}
impl<T> AnyResolvable<T> {
    /// Creates a new type-erased resolvable value.
    ///
    /// # Arguments
    /// * `value` - The resolvable value to wrap
    pub fn new(value: impl Resolvable<Resolved = T> + 'static) -> Self {
        Self {
            inner: Box::new(value),
        }
    }

    /// Resolves this value in the given environment.
    ///
    /// # Arguments
    /// * `env` - The environment to resolve in
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<T> {
        self.inner.resolve(env)
    }
}

/// A mapping type that transforms a resolvable value using a function.
#[derive(Clone)]
pub struct Map<R, F> {
    resolvable: R,
    func: F,
}

impl<R: Debug, F> Debug for Map<R, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("With")
            .field("resolvable", &self.resolvable)
            .field("func", &"Fn(...)")
            .finish()
    }
}

impl<R, F> Map<R, F> {
    /// Creates a new mapping that transforms the resolved value using the given function.
    #[must_use]
    pub const fn new<T, U>(resolvable: R, func: F) -> Self
    where
        R: Resolvable<Resolved = T>,
        F: Fn(T) -> U + Clone + 'static,
        T: 'static,
        U: 'static,
    {
        Self { resolvable, func }
    }
}

impl<R, F, T, U> Resolvable for Map<R, F>
where
    R: Resolvable<Resolved = T>,
    F: Fn(T) -> U + Clone + 'static,
    T: 'static,
    U: 'static,
{
    type Resolved = U;
    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let func = self.func.clone();
        self.resolvable.resolve(env).map(func)
    }
}
