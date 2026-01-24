//! Handler type aliases for action callbacks.
//!
//! This module provides type aliases for boxed closures that take an environment
//! reference. These are used for event handlers and callbacks throughout the framework.

use crate::{AnyView, View};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;

use crate::Environment;

/// A boxed action handler that can be called multiple times.
///
/// This is essentially `Box<dyn FnMut(&Environment) -> T>`.
pub type BoxedAction<T = ()> = Box<dyn FnMut(&Environment) -> T>;

/// A boxed action handler that can only be called once.
///
/// This is essentially `Box<dyn FnOnce(&Environment) -> T>`.
pub type BoxedActionOnce<T = ()> = Box<dyn FnOnce(&Environment) -> T>;

/// Type alias for a boxed action handler (backwards compatibility).
pub type ActionObject = BoxedAction<()>;

/// Creates a boxed action from a closure that ignores the environment.
#[inline]
pub fn boxed_action<F: FnMut() + 'static>(mut f: F) -> BoxedAction<()> {
    Box::new(move |_env: &Environment| f())
}

/// Creates a boxed action from a closure that takes the environment.
#[inline]
pub fn boxed_action_with_env<T: 'static, F: FnMut(&Environment) -> T + 'static>(
    f: F,
) -> BoxedAction<T> {
    Box::new(f)
}

/// Creates a boxed one-shot action from a closure that ignores the environment.
#[inline]
pub fn boxed_action_once<F: FnOnce() + 'static>(f: F) -> BoxedActionOnce<()> {
    Box::new(move |_env: &Environment| f())
}

/// Creates a boxed one-shot action from a closure that takes the environment.
#[inline]
pub fn boxed_action_once_with_env<T: 'static, F: FnOnce(&Environment) -> T + 'static>(
    f: F,
) -> BoxedActionOnce<T> {
    Box::new(f)
}

// ============================================================================
// Shared (Clone-able) Actions
// ============================================================================

/// A shared action that can be cloned and called multiple times.
///
/// This uses `Rc<RefCell<...>>` to allow the action to be shared across
/// multiple owners while still supporting mutation.
#[derive(Clone)]
pub struct SharedAction<T = ()>(Rc<RefCell<Box<dyn FnMut(&Environment) -> T>>>);

impl<T> SharedAction<T> {
    /// Creates a new shared action from a closure.
    pub fn new<F: FnMut(&Environment) -> T + 'static>(f: F) -> Self {
        Self(Rc::new(RefCell::new(Box::new(f))))
    }

    /// Calls the action with the given environment.
    pub fn call(&self, env: &Environment) -> T {
        (self.0.borrow_mut())(env)
    }
}

impl<T> core::fmt::Debug for SharedAction<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SharedAction")
    }
}

/// Creates a shared action from a closure that ignores the environment.
#[inline]
pub fn shared_action<F: FnMut() + 'static>(mut f: F) -> SharedAction<()> {
    SharedAction::new(move |_env: &Environment| f())
}

/// Creates a shared action from a closure that takes the environment.
#[inline]
pub fn shared_action_with_env<T: 'static, F: FnMut(&Environment) -> T + 'static>(
    f: F,
) -> SharedAction<T> {
    SharedAction::new(f)
}

// ============================================================================
// ViewBuilder
// ============================================================================

/// A trait for types that can repeatedly construct views.
///
/// This is a convenience trait that provides similar functionality to `Fn() -> impl View`,
/// allowing types to be used as view factories.
pub trait ViewBuilder: 'static {
    /// The type of view produced by this builder.
    type Output: View;
    /// Builds a view
    fn build(&self) -> Self::Output;
}

impl<V: View, F> ViewBuilder for F
where
    F: 'static + Fn() -> V,
{
    type Output = V;
    fn build(&self) -> Self::Output {
        (self)()
    }
}

/// A builder for creating views from handler functions.
pub struct AnyViewBuilder<V = AnyView>(Box<dyn ViewBuilder<Output = V>>);

impl<V: View> AnyViewBuilder<V> {
    /// Creates a new `ViewBuilder` from a handler function.
    #[must_use]
    pub fn new(handler: impl ViewBuilder<Output = V>) -> Self {
        Self(Box::new(handler))
    }

    /// Builds a view by invoking the underlying handler.
    #[must_use]
    pub fn build(&self) -> V {
        ViewBuilder::build(&*self.0)
    }

    /// Erases the specific view type, returning a builder that produces `AnyView`.
    #[must_use]
    pub fn erase(self) -> AnyViewBuilder<AnyView> {
        AnyViewBuilder::new(move || {
            let v = self.build();
            AnyView::new(v)
        })
    }
}

impl<V> core::fmt::Debug for AnyViewBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AnyViewBuilder")
    }
}
