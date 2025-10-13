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
use core::{fmt::Debug, marker::PhantomData};

use crate::Environment;

/// Handler trait that processes an environment and produces a result of type T.
///
/// This trait is implemented by handlers that don't modify themselves during execution.
pub trait Handler<T>: 'static {
    /// Processes the environment and returns a value of type T.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing request data and context
    fn handle(&self, env: &Environment) -> T;
}

impl Debug for dyn Handler<()> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Handler<()>")
    }
}

impl Handler<()> for () {
    fn handle(&self, _env: &Environment) {}
}

impl HandlerMut<()> for () {
    fn handle(&mut self, _env: &Environment) {}
}

impl HandlerOnce<()> for () {
    fn handle(self, _env: &Environment) {}
}

/// Handler trait for mutable handlers that may change their state during execution.
///
/// This trait is implemented by handlers that need to modify their internal state
/// while processing an environment.
pub trait HandlerMut<T>: 'static {
    /// Processes the environment and returns a value of type T.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing request data and context
    fn handle(&mut self, env: &Environment) -> T;
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

/// A boxed immutable handler with dynamic dispatch.
pub type BoxHandler<T> = Box<dyn Handler<T>>;

/// A boxed mutable handler with dynamic dispatch that does not return a value.
pub type ActionObject = BoxHandler<()>;

impl<T: 'static> Handler<T> for BoxHandler<T> {
    fn handle(&self, env: &Environment) -> T {
        self.as_ref().handle(env)
    }
}

impl<T: 'static> HandlerMut<T> for BoxHandlerMut<T> {
    fn handle(&mut self, env: &Environment) -> T {
        self.as_mut().handle(env)
    }
}

/// A boxed mutable handler with dynamic dispatch.
pub type BoxHandlerMut<T> = Box<dyn HandlerMut<T>>;

/// A boxed one-time handler with dynamic dispatch.
pub type BoxHandlerOnce<T> = Box<dyn HandlerOnce<T>>;

/// Function-like trait for immutable handlers that extract parameters from the environment.
///
/// P represents the parameter types to extract, T represents the return type.
pub trait HandlerFn<P, T>: 'static {
    /// Internal implementation that extracts parameters from the environment and calls the handler.
    fn handle_inner(&self, env: &Environment) -> T;
}

/// Function-like trait for immutable handlers that extract parameters from the environment with additional state.
///
/// P represents the parameter types to extract, T represents the return type, S represents the state type.
pub trait HandlerFnWithState<P, T, S>: 'static {
    /// Internal implementation that extracts parameters from the environment and calls the handler.
    fn handle_inner(&self, state: S, env: &Environment) -> T;
}

/// Function-like trait for mutable handlers that extract parameters from the environment.
///
/// P represents the parameter types to extract, T represents the return type.
pub trait HandlerFnMut<P, T>: 'static {
    /// Internal implementation that extracts parameters from the environment and calls the handler.
    fn handle_inner(&mut self, env: &Environment) -> T;
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
            F: Fn($($ty,)*) -> R+ 'static,
        {
            fn handle_inner(&self, env: &Environment) -> R {

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
            F: Fn(S,$($ty,)*) -> R+ 'static,
        {
            fn handle_inner(&self, state:S, env: &Environment) -> R {

                $(
                    let $ty:$ty=Extractor::extract(env).expect("failed to extract value from environment");
                )*

                self(state,$($ty,)*)
            }
        }
    };
}

tuples!(impl_handle_fn_with_state);

macro_rules! impl_handle_fn_mut {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<F, R, $($ty:Extractor,)*> HandlerFnMut<($($ty,)*),R> for F
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

tuples!(impl_handle_fn);

tuples!(impl_handle_fn_mut);

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
    fn handle(&self, env: &Environment) -> T {
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
    fn handle(&self, env: &Environment) -> T {
        self.h.handle_inner(env)
    }
}

impl<H, P, T> HandlerMut<T> for IntoHandlerMut<H, P, T>
where
    H: HandlerFnMut<P, T>,
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

into_handlers!(IntoHandlerMut, HandlerMut, HandlerFnMut);

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

/// Converts a mutable function into a mutable handler.
///
/// # Arguments
///
/// * `h` - The mutable function to convert into a handler
///
/// # Returns
///
/// A handler that implements the [`HandlerMut`] trait
pub const fn into_handler_mut<H, P, T>(h: H) -> IntoHandlerMut<H, P, T>
where
    H: HandlerFnMut<P, T>,
    P: 'static,
    T: 'static,
{
    IntoHandlerMut::new(h)
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

/// Trait for functions that build views from extracted parameters.
///
/// This trait is implemented for functions that extract parameters from an environment
/// and construct a view from them.
pub trait ViewBuilderFn<P>: 'static {
    /// The type of view produced by this builder.
    type Output: View;
    /// Builds a view by extracting parameters from the environment.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing request data and context
    fn build_inner(&self, env: &Environment) -> Self::Output;
}

/// A wrapper that converts a function into a view builder.
#[derive(Debug, Clone)]
pub struct IntoViewBuilder<P: 'static, F> {
    f: F,
    _phantom: core::marker::PhantomData<P>,
}

impl<P: 'static, F> IntoViewBuilder<P, F>
where
    F: ViewBuilderFn<P>,
{
    /// Creates a new view builder from the given function.
    #[must_use]
    pub const fn new(f: F) -> Self {
        Self {
            f,
            _phantom: core::marker::PhantomData,
        }
    }
}

macro_rules! impl_view_builder_fn {
    ($($param:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        impl<V, F, $($param:Extractor),*> ViewBuilderFn<($($param,)*)> for F
        where
            V: View,
            F: Fn($($param,)*) -> V + 'static,
        {
            type Output = V;
            fn build_inner(&self, env: &Environment) -> Self::Output {
                $(
                    let $param:$param=Extractor::extract(env).expect("failed to extract value from environment");
                )*
                (self)($($param,)*)
            }
        }
    };
}

tuples!(impl_view_builder_fn);

/// Trait for types that can repeatedly build views from an environment.
///
/// Implementors can be invoked multiple times to construct new view instances,
/// extracting necessary data from the environment on each invocation.
pub trait ViewBuilder: 'static {
    /// The type of view produced by this builder.
    type Output: View;
    /// Builds a view from the environment.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing request data and context
    fn build(&self, env: &Environment) -> Self::Output;
}

impl<P: 'static, F> ViewBuilder for IntoViewBuilder<P, F>
where
    F: ViewBuilderFn<P>,
{
    type Output = F::Output;
    fn build(&self, env: &Environment) -> Self::Output {
        self.f.build_inner(env)
    }
}

/// Converts a function into a view builder.
#[must_use]
pub const fn into_view_builder<P: 'static, F>(f: F) -> IntoViewBuilder<P, F>
where
    F: ViewBuilderFn<P>,
{
    IntoViewBuilder::new(f)
}

/// A builder for creating views from handler functions.
///
/// This struct wraps a boxed handler that produces `AnyView` instances.
pub struct AnyViewBuilder(BoxHandler<AnyView>);

impl Debug for AnyViewBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ViewBuilder(..)")
    }
}

impl<P, F> Handler<AnyView> for IntoViewBuilder<P, F>
where
    F: ViewBuilderFn<P>,
{
    fn handle(&self, env: &Environment) -> AnyView {
        AnyView::new(self.build(env))
    }
}

impl AnyViewBuilder {
    /// Creates a new `ViewBuilder` from a handler function.
    /// # Arguments
    /// * `handler` - The function that builds a view from extracted parameters
    #[must_use]
    pub fn new<H: 'static>(handler: impl ViewBuilderFn<H>) -> Self {
        Self(Box::new(into_view_builder(handler)))
    }

    /// Builds a view by invoking the underlying handler with the given environment.
    /// # Arguments
    /// * `env` - The environment to pass to the handler
    /// # Returns
    /// An `AnyView` produced by the handler
    pub fn build(&self, env: &Environment) -> AnyView {
        (self.0).handle(env)
    }
}