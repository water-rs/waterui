//! Handler traits and implementations for processing environments.
//!
//! This module provides a handler system that allows functions to process environments
//! and extract parameters from them in various ways:
//!
//! - `Handler` - For immutable handlers that don't change during execution
//! - `HandlerMut` - For mutable handlers that may modify their state
//! - `HandlerOnce` - For single-use handlers that are consumed during processing
//!
//! The module also provides utility functions to convert regular functions into handlers
//! with automatic parameter extraction from environments.

use crate::{AnyView, View, extract::Extractor};
use alloc::boxed::Box;
use core::{any::type_name, fmt::Debug, marker::PhantomData};

use crate::Environment;

/// Type alias for an action handler that produces no result.
pub type ActionObject = BoxHandler<()>;

/// Handler trait that processes an environment and produces a result of type T.
///
/// This trait is implemented by handlers that don't modify themselves during execution.
pub trait Handler<T>: 'static {
    /// Processes the environment and returns a value of type T.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing request data and context
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

impl HandlerOnce<()> for () {
    fn handle(self, _env: &Environment) {}
}

/// Handler trait for single-use handlers that are consumed during execution.
///
/// This trait is implemented by handlers that can only be called once because
/// they consume themselves during processing.
pub trait HandlerOnce<T>: 'static {
    /// Processes the environment and returns a value of type T.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing request data and context
    fn handle(self, env: &Environment) -> T;
}

/// A boxed handler with dynamic dispatch.
pub type BoxHandler<T> = Box<dyn Handler<T>>;

impl<T: 'static> Handler<T> for BoxHandler<T> {
    fn handle(&mut self, env: &Environment) -> T {
        self.as_mut().handle(env)
    }
}

/// A boxed one-time handler with dynamic dispatch.
pub type BoxHandlerOnce<T> = Box<dyn HandlerOnce<T>>;

/// Function-like trait for immutable handlers that extract parameters from the environment.
///
/// P represents the parameter types to extract, T represents the return type.
pub trait HandlerFn<P, T>: 'static {
    /// Internal implementation that extracts parameters from the environment and calls the handler.
    fn handle_inner(&mut self, env: &Environment) -> T;
}

/// Function-like trait for immutable handlers that extract parameters from the environment with additional state.
///
/// P represents the parameter types to extract, T represents the return type, S represents the state type.
pub trait HandlerFnWithState<P, T, S>: 'static {
    /// Internal implementation that extracts parameters from the environment and calls the handler.
    fn handle_inner(&mut self, state: S, env: &Environment) -> T;
}

/// Function-like trait for single-use handlers that extract parameters from the environment.
///
/// P represents the parameter types to extract, T represents the return type.
pub trait HandlerFnOnce<P, T>: 'static {
    /// Internal implementation that extracts parameters from the environment and calls the handler.
    fn handle_inner(self, env: &Environment) -> T;
}

macro_rules! impl_handle_fn {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<F, R, $($ty:Extractor,)*> HandlerFn<($($ty,)*),R> for F
        where
            F: FnMut($($ty,)*) -> R+ 'static,
        {
            fn handle_inner(&mut self, env: &Environment) -> R {

                $(
                    let $ty:$ty=Extractor::extract(env).expect("failed to extract value from environment");
                )*

                self($($ty,)*)
            }
        }
    };
}

macro_rules! impl_handle_fn_with_state {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<F, S, R, $($ty:Extractor,)*> HandlerFnWithState<($($ty,)*),R,S> for F
        where
            F: FnMut(S,$($ty,)*) -> R+ 'static,
        {
            fn handle_inner(&mut self, state:S, env: &Environment) -> R {

                $(
                    let $ty:$ty=Extractor::extract(env).expect("failed to extract value from environment");
                )*

                self(state,$($ty,)*)
            }
        }
    };
}

tuples!(impl_handle_fn_with_state);

tuples!(impl_handle_fn);

macro_rules! impl_handle_fn_once {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<F, R, $($ty:Extractor,)*> HandlerFnOnce<($($ty,)*),R> for F
        where
            F: FnOnce($($ty,)*) -> R+ 'static,
        {
            fn handle_inner(self, env: &Environment) -> R {

                $(
                    let $ty:$ty=Extractor::extract(env).expect("failed to extract value from environment");
                )*

                self($($ty,)*)
            }
        }
    };
}

tuples!(impl_handle_fn_once);

macro_rules! into_handlers {
    ($name:ident,$handler:ident,$handler_fn:ident) => {
        /// Wrapper that converts a function into a handler.
        #[derive(Debug, Clone)]
        pub struct $name<H, P, T> {
            h: H,
            _marker: PhantomData<(P, T)>,
        }

        impl<H, P, T> $name<H, P, T>
        where
            H: $handler_fn<P, T>,
        {
            /// Creates a new handler wrapper around the given function.
            #[must_use]
            pub const fn new(h: H) -> Self {
                Self {
                    h,
                    _marker: PhantomData,
                }
            }
        }
    };
}

/// A wrapper that allows a handler function with state to be used as a regular handler.
#[derive(Debug)]
pub struct IntoHandlerWithState<H, P, T, S> {
    h: H,
    state: S,
    _marker: PhantomData<(P, T, S)>,
}

impl<H, P, T, S> IntoHandlerWithState<H, P, T, S>
where
    H: HandlerFnWithState<P, T, S>,
    S: 'static + Clone,
{
    /// Creates a new handler wrapper around the given function and state.
    #[must_use]
    pub const fn new(h: H, state: S) -> Self {
        Self {
            h,
            state,
            _marker: PhantomData,
        }
    }
}

impl<H, P, T, S> Handler<T> for IntoHandlerWithState<H, P, T, S>
where
    H: HandlerFnWithState<P, T, S>,
    S: 'static + Clone,
    T: 'static,
    P: 'static,
{
    fn handle(&mut self, env: &Environment) -> T {
        self.h.handle_inner(self.state.clone(), env)
    }
}

/// Creates a handler with associated state from a handler function and state value.
pub const fn into_handler_with_state<H, P, T, S>(h: H, state: S) -> IntoHandlerWithState<H, P, T, S>
where
    H: HandlerFnWithState<P, T, S>,
    S: 'static + Clone,
    T: 'static,
    P: 'static,
{
    IntoHandlerWithState::new(h, state)
}

into_handlers!(IntoHandler, Handler, HandlerFn);

impl<H, P, T> Handler<T> for IntoHandler<H, P, T>
where
    H: HandlerFn<P, T>,
    P: 'static,
    T: 'static,
{
    fn handle(&mut self, env: &Environment) -> T {
        self.h.handle_inner(env)
    }
}

impl<H, P, T> HandlerOnce<T> for IntoHandlerOnce<H, P, T>
where
    H: HandlerFnOnce<P, T>,
    P: 'static,
    T: 'static,
{
    fn handle(self, env: &Environment) -> T {
        self.h.handle_inner(env)
    }
}

into_handlers!(IntoHandlerOnce, HandlerOnce, HandlerFnOnce);

/// Converts a function into an immutable handler.
///
/// # Arguments
///
/// * `h` - The function to convert into a handler
///
/// # Returns
///
/// A handler that implements the Handler trait
pub const fn into_handler<H, P, T>(h: H) -> IntoHandler<H, P, T>
where
    P: 'static,
    T: 'static,
    H: HandlerFn<P, T>,
{
    IntoHandler::new(h)
}

/// Converts a single-use function into a one-time handler.
///
/// # Arguments
///
/// * `h` - The single-use function to convert into a handler
///
/// # Returns
///
/// A handler that implements the [`HandlerOnce`] trait
pub const fn into_handler_once<H, P, T>(h: H) -> IntoHandlerOnce<H, P, T>
where
    H: HandlerFnOnce<P, T>,
    P: 'static,
    T: 'static,
{
    IntoHandlerOnce::new(h)
}

/// A trait for types that can repeatedly construct views.
///
/// This is a convenience trait that provides similar functionality to `Fn() -> impl View`,
/// allowing types to be used as view factories.
pub trait ViewBuilder: 'static {
    /// The type of view produced by this builder.
    type Output: View;
    /// Builds a view
    // Note: unlike `Handler`, a `View` can obtain its `Environment` during rendering
    // so no need to pass it here
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
///
/// This struct wraps a boxed handler that produces `AnyView` instances.
pub struct AnyViewBuilder<V = AnyView>(Box<dyn ViewBuilder<Output = V>>);

impl<V: View> AnyViewBuilder<V> {
    /// Creates a new `ViewBuilder` from a handler function.
    /// # Arguments
    /// * `handler` - The function that builds a view from extracted parameters
    #[must_use]
    pub fn new(handler: impl ViewBuilder<Output = V>) -> Self {
        Self(Box::new(handler))
    }

    /// Builds a view by invoking the underlying handler.
    /// # Returns
    /// An `AnyView` produced by the handler
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
