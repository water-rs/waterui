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

/// A handler parameter could not be extracted from the environment, so the
/// handler did not run.
///
/// The dispatcher of the action decides the outcome: an ordinary action does
/// not run and [`report`](Self::report)s the failure, key handling treats the
/// key as not handled, and a web view rejects the page's promise.
#[derive(Debug, thiserror::Error)]
#[error("failed to extract `{type_name}` from environment for action: {cause}")]
pub struct ExtractionError {
    type_name: &'static str,
    cause: crate::Error,
}

impl ExtractionError {
    /// The name of the parameter type that could not be extracted.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// Reports the failure as a contract violation: a debug build panics and
    /// a release build logs it.
    pub fn report(&self) {
        crate::contract_violation!("{self}");
    }
}

/// Reporting of a failed handler invocation by a dispatcher that has no
/// better outcome than dropping the action.
pub trait ReportExtraction<T> {
    /// Returns the handler's result, or [`report`](ExtractionError::report)s
    /// the extraction failure and returns `None`.
    fn report_extraction(self) -> Option<T>;
}

impl<T> ReportExtraction<T> for Result<T, ExtractionError> {
    fn report_extraction(self) -> Option<T> {
        self.map_err(|error| error.report()).ok()
    }
}

/// A boxed action handler that can be called multiple times.
///
/// This is essentially `Box<dyn FnMut(&Environment) -> Result<T, ExtractionError>>`.
pub type BoxedAction<T = ()> = Box<dyn FnMut(&Environment) -> Result<T, ExtractionError>>;

/// A boxed action handler that can only be called once.
///
/// This is essentially `Box<dyn FnOnce(&Environment) -> Result<T, ExtractionError>>`.
pub type BoxedActionOnce<T = ()> = Box<dyn FnOnce(&Environment) -> Result<T, ExtractionError>>;

/// Type alias for a boxed action handler (backwards compatibility).
pub type ActionObject = BoxedAction<()>;

fn extract<T: Extractor>(
    env: &Environment,
    state: &mut ExtractionState,
) -> Result<T, ExtractionError> {
    T::extract_from_action(env, state).map_err(|cause| ExtractionError {
        type_name: type_name::<T>(),
        cause,
    })
}

/// A repeatable handler that can extract arguments from the environment.
#[diagnostic::on_unimplemented(
    message = "this handler's parameters cannot be extracted from the environment",
    label = "expected a handler whose parameters implement `Extractor`",
    note = "A WaterUI handler takes no parameters or parameters implementing `Extractor`: `#[state]` makes an owned `Clone` type extractable from `.state(&value)` injections, `State<T>` wraps an injected value of a foreign type, `Use<T>` reads an environment value, `Option<E>` tolerates a missing one, and `impl_extractor!` marks a `Clone` type installed as an environment value. Repeated parameters of the same state type bind positionally — the first `.state()` call feeds the first parameter of that type."
)]
pub trait Handler<Args, T = ()>: 'static {
    /// Invokes the handler using values extracted from `env`.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtractionError`] without running the handler when one of
    /// its parameters cannot be extracted from `env`.
    fn call(&mut self, env: &Environment) -> Result<T, ExtractionError>;
}

/// A one-shot handler that can extract arguments from the environment.
pub trait HandlerOnce<Args, T = ()>: 'static {
    /// Invokes the handler once using values extracted from `env`.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtractionError`] without running the handler when one of
    /// its parameters cannot be extracted from `env`.
    fn call_once(self, env: &Environment) -> Result<T, ExtractionError>;
}

macro_rules! impl_handler {
    () => {
        impl<F, Output> Handler<(), Output> for F
        where
            F: FnMut() -> Output + 'static,
        {
            fn call(&mut self, _env: &Environment) -> Result<Output, ExtractionError> {
                Ok(self())
            }
        }

        impl<F, Output> HandlerOnce<(), Output> for F
        where
            F: FnOnce() -> Output + 'static,
        {
            fn call_once(self, _env: &Environment) -> Result<Output, ExtractionError> {
                Ok(self())
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
            fn call(&mut self, env: &Environment) -> Result<Output, ExtractionError> {
                let mut state = ExtractionState::default();
                $(let $T = extract::<$T>(env, &mut state)?;)+
                Ok(self($($T),+))
            }
        }

        impl<Func, Output, $($T),+> HandlerOnce<($($T,)+), Output> for Func
        where
            Func: FnOnce($($T),+) -> Output + 'static,
            $($T: Extractor),+
        {
            #[allow(non_snake_case)]
            fn call_once(self, env: &Environment) -> Result<Output, ExtractionError> {
                let mut state = ExtractionState::default();
                $(let $T = extract::<$T>(env, &mut state)?;)+
                Ok(self($($T),+))
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
// Event handlers
// ============================================================================

/// A boxed event handler that consumes one event payload alongside the
/// environment-driven extractors.
///
/// This is essentially `Box<dyn FnMut(E, &Environment) -> Result<T, ExtractionError>>` and is the
/// counterpart to [`BoxedAction`] for views that report events upward —
/// for example media playback events, video errors, or any photo-style
/// completion notification. The first argument is the event payload (typed
/// to the producing component); the remaining arguments are extracted from
/// `env` exactly like [`Handler`] arguments.
pub type BoxedEventAction<E, T = ()> =
    Box<dyn FnMut(E, &Environment) -> Result<T, ExtractionError>>;

/// A repeatable event handler that consumes a typed event payload plus
/// environment-extracted arguments.
///
/// Implemented for any closure of the form `FnMut(E, A1, ..., An) -> T`
/// where each `Ai: Extractor`. The shape mirrors [`Handler`] but inserts
/// an "event" position in the leading argument slot. This lets event
/// callbacks on views like `Photo::on_event`, `Video::on_event`, and
/// `WebView::on_event` reuse the same `State<T>` / `Environment` extractor
/// machinery as `Button::action`.
///
/// The `Args` tuple only counts extractor positions — the event payload is
/// not part of the tuple — so a closure like `|event: E| { ... }` matches
/// `EventHandler<E, (), ()>` and reads as "no extractors, returns unit".
pub trait EventHandler<E, Args, T = ()>: 'static {
    /// Invokes the handler with the given event payload and the extractor
    /// arguments resolved from `env`.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtractionError`] without running the handler when one of
    /// its extractor parameters cannot be extracted from `env`.
    fn call(&mut self, event: E, env: &Environment) -> Result<T, ExtractionError>;
}

macro_rules! impl_event_handler {
    () => {
        impl<F, E, Output> EventHandler<E, (), Output> for F
        where
            F: FnMut(E) -> Output + 'static,
        {
            fn call(&mut self, event: E, _env: &Environment) -> Result<Output, ExtractionError> {
                Ok(self(event))
            }
        }
    };
    ($($T:ident),+) => {
        impl<Func, E, Output, $($T),+> EventHandler<E, ($($T,)+), Output> for Func
        where
            Func: FnMut(E, $($T),+) -> Output + 'static,
            $($T: Extractor),+
        {
            #[allow(non_snake_case)]
            fn call(&mut self, event: E, env: &Environment) -> Result<Output, ExtractionError> {
                let mut state = ExtractionState::default();
                $(let $T = extract::<$T>(env, &mut state)?;)+
                Ok(self(event, $($T),+))
            }
        }
    };
}

impl_event_handler!();
impl_event_handler!(A);
impl_event_handler!(A, B);
impl_event_handler!(A, B, C);
impl_event_handler!(A, B, C, D);
impl_event_handler!(A, B, C, D, E1);
impl_event_handler!(A, B, C, D, E1, F);
impl_event_handler!(A, B, C, D, E1, F, G);
impl_event_handler!(A, B, C, D, E1, F, G, H);

/// Erases an [`EventHandler`] into a [`BoxedEventAction`] so a component
/// config can store the callback as a typed field without leaking the
/// extractor-tuple generics.
#[inline]
pub fn boxed_event_handler<E, Args, T: 'static>(
    mut f: impl EventHandler<E, Args, T>,
) -> BoxedEventAction<E, T>
where
    E: 'static,
{
    Box::new(move |event: E, env: &Environment| f.call(event, env))
}

// ============================================================================
// Shared (Clone-able) Actions
// ============================================================================

/// A shared action that can be cloned and called multiple times.
///
/// This uses `Rc<RefCell<...>>` to allow the action to be shared across
/// multiple owners while still supporting mutation.
type SharedActionFn<T> = Rc<RefCell<BoxedAction<T>>>;

/// Cloneable action handle backed by shared mutable state.
#[derive(Clone)]
pub struct SharedAction<T = ()>(SharedActionFn<T>);

impl<T: 'static> SharedAction<T> {
    /// Creates a new shared action from a closure.
    pub fn new<Args>(f: impl Handler<Args, T>) -> Self {
        Self(Rc::new(RefCell::new(boxed_action(f))))
    }

    /// Calls the action with the given environment.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtractionError`] without running the action when one of
    /// its parameters cannot be extracted from `env`.
    pub fn call(&self, env: &Environment) -> Result<T, ExtractionError> {
        (self.0.borrow_mut())(env)
    }
}

impl SharedAction<()> {
    /// Runs the action as an ordinary dispatcher does: when a parameter
    /// cannot be extracted, the action does not run and the failure is
    /// [`report`](ExtractionError::report)ed.
    pub fn run(&self, env: &Environment) {
        self.call(env).report_extraction();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::State;
    use alloc::rc::Rc;
    use core::cell::Cell;

    #[test]
    fn shared_action_invokes_unit_handler_repeatedly() {
        let count = Rc::new(Cell::new(0));
        let captured_count = Rc::clone(&count);
        let action = shared_action(move || captured_count.set(captured_count.get() + 1));

        action.run(&Environment::default());
        action.run(&Environment::default());

        assert_eq!(count.get(), 2);
    }

    #[test]
    fn shared_action_preserves_return_values() {
        let action = shared_action(|| 7);

        assert_eq!(action.call(&Environment::default()).ok(), Some(7));
    }

    #[test]
    fn failed_extraction_does_not_run_the_handler() {
        let ran = Rc::new(Cell::new(false));
        let captured = Rc::clone(&ran);
        let action = shared_action(move |_: State<u8>| captured.set(true));

        let error = action
            .call(&Environment::default())
            .expect_err("the state is not installed");

        assert!(!ran.get());
        assert!(error.type_name().contains("State"));
        assert!(error.to_string().contains("not found"));
    }

    #[test]
    #[should_panic(expected = "failed to extract")]
    #[cfg(debug_assertions)]
    fn reporting_a_failed_extraction_panics_in_debug() {
        shared_action(|_: State<u8>| {}).run(&Environment::default());
    }
}
