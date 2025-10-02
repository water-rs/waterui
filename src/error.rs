//! Utilities for displaying runtime errors inside WaterUI views.
//!
//! The types in this module let applications convert any `std::error::Error`
//! into a `View` that can be rendered by the UI, optionally leveraging a
//! user-supplied builder stored in the [`Environment`].

use core::fmt::{Debug};

use waterui_core::{AnyView, Environment, View};

use crate::ViewExt;

/// A `View` wrapper that renders a boxed `std::error::Error`.
#[derive(Debug)]
pub struct ErrorView {
    inner: BoxedError,
}

impl<E> From<E> for ErrorView
where
    E: std::error::Error + 'static,
{
    fn from(error: E) -> Self {
        Self {
            inner: Box::new(error),
        }
    }
}

/// Convenient alias for boxed dynamic errors used by the error view helpers.
type BoxedError = Box<dyn core::error::Error>;

#[allow(clippy::type_complexity)]
/// Holds a custom renderer used to turn errors into views.
pub struct ErrorViewBuilder(Box<dyn Fn(BoxedError, &Environment) -> AnyView>);

impl Debug for ErrorViewBuilder{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ErrorViewBuilder")
            .field("func", &"Box<dyn Fn(BoxedError, &Environment) -> AnyView>")
            .finish()
    }
}

impl View for ErrorView {
    /// Uses a custom builder from the environment when present, otherwise
    /// falls back to rendering the error as plain text.
    fn body(self, env: &waterui_core::Environment) -> impl View {
        if let Some(builder) = env.get::<ErrorViewBuilder>() {
            (builder.0)(self.inner, env)
        } else {
            format!("{}", self.inner).anyview()
        }
    }
}

impl ErrorViewBuilder {
    /// Creates a new `ErrorViewBuilder` with the given function.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(BoxedError, &Environment) -> AnyView + 'static,
    {
        Self(Box::new(f))
    }
}
