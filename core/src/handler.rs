//! Handler type aliases for action callbacks.
//!
//! This module provides type aliases for boxed closures that take an environment
//! reference. These are used for event handlers and callbacks throughout the framework.

use crate::extract::{ExtractionState, Extractor};
use crate::{AnyView, View};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::type_name;
use core::cell::RefCell;
use core::fmt;

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

fn extract_or_panic<T: Extractor>(env: &Environment, state: &mut ExtractionState) -> T {
    T::extract_from_action(env, state).unwrap_or_else(|error| {
        panic!(
            "failed to extract `{}` from environment for action: {error}",
            type_name::<T>()
        )
    })
}

/// A repeatable handler that can extract arguments from the environment.
pub trait Handler<Args, T = ()>: 'static {
    /// Invokes the handler using values extracted from `env`.
    fn call(&mut self, env: &Environment) -> T;
}

/// A one-shot handler that can extract arguments from the environment.
pub trait HandlerOnce<Args, T = ()>: 'static {
    /// Invokes the handler once using values extracted from `env`.
    fn call_once(self, env: &Environment) -> T;
}

macro_rules! impl_handler {
    () => {
        impl<F, Output> Handler<(), Output> for F
        where
            F: FnMut() -> Output + 'static,
        {
            fn call(&mut self, _env: &Environment) -> Output {
                self()
            }
        }

        impl<F, Output> HandlerOnce<(), Output> for F
        where
            F: FnOnce() -> Output + 'static,
        {
            fn call_once(self, _env: &Environment) -> Output {
                self()
            }
        }
    };
    ($($T:ident),+) => {
        impl<Func, Output, $($T),+> Handler<($($T,)+), Output> for Func
        where
            Func: FnMut($($T),+) -> Output + 'static,
            $($T: Extractor),+
        {
            #[allow(non_snake_case)]
            fn call(&mut self, env: &Environment) -> Output {
                let mut state = ExtractionState::default();
                $(let $T = extract_or_panic::<$T>(env, &mut state);)+
                self($($T),+)
            }
        }

        impl<Func, Output, $($T),+> HandlerOnce<($($T,)+), Output> for Func
        where
            Func: FnOnce($($T),+) -> Output + 'static,
            $($T: Extractor),+
        {
            #[allow(non_snake_case)]
            fn call_once(self, env: &Environment) -> Output {
                let mut state = ExtractionState::default();
                $(let $T = extract_or_panic::<$T>(env, &mut state);)+
                self($($T),+)
            }
        }
    };
}

impl_handler!();
impl_handler!(A);
impl_handler!(A, B);
impl_handler!(A, B, C);
impl_handler!(A, B, C, D);
impl_handler!(A, B, C, D, E);
impl_handler!(A, B, C, D, E, F);
impl_handler!(A, B, C, D, E, F, G);
impl_handler!(A, B, C, D, E, F, G, H);

/// Creates a boxed action from a handler.
#[inline]
pub fn boxed_action<Args, T: 'static>(mut f: impl Handler<Args, T>) -> BoxedAction<T> {
    Box::new(move |env: &Environment| f.call(env))
}

/// Creates a boxed one-shot action from a handler.
#[inline]
pub fn boxed_action_once<Args, T: 'static>(f: impl HandlerOnce<Args, T>) -> BoxedActionOnce<T> {
    Box::new(move |env: &Environment| f.call_once(env))
}

// ============================================================================
// Shared (Clone-able) Actions
// ============================================================================

/// A shared action that can be cloned and called multiple times.
///
/// This uses `Rc<RefCell<...>>` to allow the action to be shared across
/// multiple owners while still supporting mutation.
type SharedActionFn<T> = Rc<RefCell<Box<dyn FnMut(&Environment) -> T>>>;

/// Cloneable action handle backed by shared mutable state.
#[derive(Clone)]
pub struct SharedAction<T = ()>(SharedActionFn<T>);

impl<T: 'static> SharedAction<T> {
    /// Creates a new shared action from a closure.
    pub fn new<Args>(f: impl Handler<Args, T>) -> Self {
        Self(Rc::new(RefCell::new(boxed_action(f))))
    }

    /// Calls the action with the given environment.
    #[expect(
        clippy::must_use_candidate,
        reason = "actions are side-effectful and may intentionally return unit"
    )]
    pub fn call(&self, env: &Environment) -> T {
        (self.0.borrow_mut())(env)
    }
}

impl<T> fmt::Debug for SharedAction<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedAction")
    }
}

/// Creates a shared action from a closure that ignores the environment.
#[inline]
pub fn shared_action<Args, T: 'static>(f: impl Handler<Args, T>) -> SharedAction<T> {
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
pub struct AnyViewBuilder<V = AnyView>(Rc<dyn ViewBuilder<Output = V>>);

impl<V> Clone for AnyViewBuilder<V> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<V: View> AnyViewBuilder<V> {
    /// Creates a new `ViewBuilder` from a handler function.
    #[must_use]
    pub fn new(handler: impl ViewBuilder<Output = V>) -> Self {
        Self(Rc::new(handler))
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

impl<V> fmt::Debug for AnyViewBuilder<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AnyViewBuilder")
    }
}
