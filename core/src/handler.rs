//! Handler traits for action callbacks.
//!
//! This module provides simple handler traits for callbacks that need to be
//! stored as trait objects. Unlike the previous version, these handlers do NOT
//! perform automatic parameter extraction from the environment.
//!
//! For state passing, use the builder pattern (e.g., `.with_state(&value)`) on
//! views and buttons, which clones state at creation time.

use crate::{AnyView, View};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::{any::type_name, fmt::Debug};

use crate::Environment;

/// Type alias for a boxed action handler.
pub type ActionObject = BoxHandler<()>;

/// Handler trait that processes an environment and produces a result of type T.
///
/// This is a simple trait for callbacks that can be called multiple times.
/// The environment is provided for context but extraction should be done
/// explicitly via the builder pattern.
pub trait Handler<T>: 'static {
    /// Processes the environment and returns a value of type T.
    fn handle(&mut self, env: &Environment) -> T;
}

impl<T> Debug for dyn Handler<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl Handler<()> for () {
    fn handle(&mut self, _env: &Environment) {}
}

/// Handler trait for single-use handlers that are consumed during execution.
pub trait HandlerOnce<T>: 'static {
    /// Processes the environment and returns a value of type T.
    fn handle(self, env: &Environment) -> T
    where
        Self: Sized;

    /// Calls the handler when boxed. Used for trait object dispatch.
    fn call_box(self: Box<Self>, env: &Environment) -> T;
}

impl<T> Debug for dyn HandlerOnce<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl HandlerOnce<()> for () {
    fn handle(self, _env: &Environment) {}

    fn call_box(self: Box<Self>, env: &Environment) {
        (*self).handle(env);
    }
}

/// A boxed handler with dynamic dispatch.
pub type BoxHandler<T> = Box<dyn Handler<T>>;

/// A reference-counted handler with dynamic dispatch.
pub type SharedHandler<T> = Rc<dyn Handler<T>>;

impl<T: 'static> Handler<T> for BoxHandler<T> {
    fn handle(&mut self, env: &Environment) -> T {
        self.as_mut().handle(env)
    }
}

/// A boxed one-time handler with dynamic dispatch.
pub type BoxHandlerOnce<T> = Box<dyn HandlerOnce<T>>;

impl<T: 'static> HandlerOnce<T> for Box<dyn HandlerOnce<T>> {
    fn handle(self, env: &Environment) -> T
    where
        Self: Sized,
    {
        HandlerOnce::call_box(self, env)
    }

    fn call_box(self: Box<Self>, env: &Environment) -> T {
        (*self).call_box(env)
    }
}

// ============================================================================
// Simple closure implementations
// ============================================================================

/// Wrapper for FnMut() closures to implement Handler.
pub struct SimpleAction<F>(pub F);

impl<F: Debug> Debug for SimpleAction<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("SimpleAction").field(&self.0).finish()
    }
}

impl<F: FnMut() + 'static> Handler<()> for SimpleAction<F> {
    fn handle(&mut self, _env: &Environment) {
        (self.0)();
    }
}

/// Wrapper for FnOnce() closures to implement HandlerOnce.
pub struct SimpleActionOnce<F>(pub Option<F>);

impl<F: Debug> Debug for SimpleActionOnce<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("SimpleActionOnce").field(&self.0).finish()
    }
}

impl<F: FnOnce() + 'static> HandlerOnce<()> for SimpleActionOnce<F> {
    fn handle(mut self, _env: &Environment) {
        if let Some(f) = self.0.take() {
            f();
        }
    }

    fn call_box(mut self: Box<Self>, _env: &Environment) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

/// Creates a boxed Handler from a FnMut() closure.
pub fn boxed_action<F: FnMut() + 'static>(f: F) -> BoxHandler<()> {
    Box::new(SimpleAction(f))
}

/// Creates a boxed HandlerOnce from a FnOnce() closure.
pub fn boxed_action_once<F: FnOnce() + 'static>(f: F) -> BoxHandlerOnce<()> {
    Box::new(SimpleActionOnce(Some(f)))
}

/// Wrapper for shared actions that need interior mutability.
pub struct SharedAction<F>(core::cell::RefCell<F>);

impl<F: Debug> Debug for SharedAction<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("SharedAction").field(&self.0).finish()
    }
}

impl<F: FnMut() + 'static> Handler<()> for SharedAction<F> {
    fn handle(&mut self, _env: &Environment) {
        (self.0.borrow_mut())();
    }
}

/// Creates a SharedHandler from a FnMut() closure.
pub fn shared_action<F: FnMut() + 'static>(f: F) -> SharedHandler<()> {
    Rc::new(SharedAction(core::cell::RefCell::new(f)))
}

// ============================================================================
// ViewBuilder (unchanged)
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

impl<V> Debug for AnyViewBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AnyViewBuilder")
    }
}
