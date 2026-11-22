//! Typed error views for `WaterUI`.
//!
//! This module turns errors and `Result` values into [`View`]s without type
//! erasure or environment-injected builders:
//!
//! - [`ErrorView`]: Renders an error value through an [`ErrorViewBuilder`] (by
//!   default [`DefaultErrorView`], which renders the error's [`Display`]
//!   representation as [`Text`]).
//! - [`ErrorViewBuilder`]: Trait implemented by [`DefaultErrorView`] and
//!   closures `FnOnce(E) -> V`.
//! - [`ResultExt`]: Extension trait converting `Result<T, E>` errors directly
//!   into typed views or [`ErrorView`]s.
//!
//! # Examples
//!
//! ## Default error view
//!
//! ```rust
//! use std::io;
//! use waterui::prelude::*;
//! use waterui::error::ErrorView;
//!
//! let view = ErrorView::new(io::Error::new(io::ErrorKind::NotFound, "File not found"));
//! let from_err: ErrorView<_> = io::Error::other("disk full").into();
//! # let _ = (view, from_err);
//! ```
//!
//! ## Custom error builder
//!
//! ```rust
//! use std::io;
//! use waterui::prelude::*;
//! use waterui::error::ErrorView;
//!
//! let view = ErrorView::with_builder(
//!     io::Error::other("connection reset"),
//!     |err: io::Error| text(format!("Request failed: {err}")),
//! );
//! # let _ = view;
//! ```
//!
//! ## Converting `Result` errors to views
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::error::ResultExt;
//!
//! fn load_data() -> Result<String, std::io::Error> {
//!     Ok("data".to_string())
//! }
//!
//! fn my_view() -> impl View {
//!     load_data()
//!         .map(text)
//!         .error_view(|err| text(format!("Failed to load: {err}")))
//! }
//! ```

use alloc::format;
use core::fmt::Display;

use waterui_core::{Environment, View};
use waterui_text::{Text, text};

/// Re-export of the standard error trait for convenience.
pub use core::error::Error as StdError;

/// A boxed standard error trait object for callers that opt into combining
/// heterogeneous error types.
pub type BoxedStdError = alloc::boxed::Box<dyn StdError>;

/// Builds a [`View`] from an error of type `E`.
///
/// Implemented by [`DefaultErrorView`] (which formats `E` into [`Text`]) and by
/// any `FnOnce(E) -> V + 'static` closure returning a [`View`].
pub trait ErrorViewBuilder<E>: 'static {
    /// The view produced for an error.
    type Body: View;

    /// Builds the view representing `error`.
    fn build(self, error: E) -> Self::Body;
}

/// Default error renderer that displays an error's [`Display`] text.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefaultErrorView;

impl<E: Display + 'static> ErrorViewBuilder<E> for DefaultErrorView {
    type Body = Text;

    fn build(self, error: E) -> Self::Body {
        text(format!("{error}"))
    }
}

impl<E: 'static, F, V> ErrorViewBuilder<E> for F
where
    F: FnOnce(E) -> V + 'static,
    V: View,
{
    type Body = V;

    fn build(self, error: E) -> Self::Body {
        self(error)
    }
}

/// A [`View`] that renders an error `E` with a statically typed builder `B`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorView<E, B = DefaultErrorView> {
    error: E,
    builder: B,
}

impl<E> ErrorView<E, DefaultErrorView> {
    /// Creates an [`ErrorView`] that renders `error` using [`DefaultErrorView`].
    #[must_use]
    pub const fn new(error: E) -> Self {
        Self {
            error,
            builder: DefaultErrorView,
        }
    }
}

impl<E, F, V> ErrorView<E, F>
where
    F: FnOnce(E) -> V + 'static,
    V: View,
{
    /// Creates an [`ErrorView`] with a custom closure builder.
    #[must_use]
    pub const fn with_builder(error: E, builder: F) -> Self {
        Self { error, builder }
    }
}

impl<E, B> ErrorView<E, B> {
    /// Replaces the error renderer with `builder`.
    #[must_use]
    pub fn builder<F, V>(self, builder: F) -> ErrorView<E, F>
    where
        F: FnOnce(E) -> V + 'static,
        V: View,
    {
        ErrorView {
            error: self.error,
            builder,
        }
    }

    /// Returns a reference to the underlying error.
    #[must_use]
    pub const fn error(&self) -> &E {
        &self.error
    }

    /// Unwraps and returns the underlying error.
    #[must_use]
    pub fn into_error(self) -> E {
        self.error
    }
}

impl<E: StdError + 'static> From<E> for ErrorView<E, DefaultErrorView> {
    fn from(error: E) -> Self {
        Self::new(error)
    }
}

impl<E, B> View for ErrorView<E, B>
where
    E: 'static,
    B: ErrorViewBuilder<E>,
{
    fn body(self, _env: &Environment) -> impl View {
        self.builder.build(self.error)
    }
}

/// Creates an [`ErrorView`] that renders `error` as plain text.
#[must_use]
pub const fn error_view<E>(error: E) -> ErrorView<E, DefaultErrorView> {
    ErrorView::new(error)
}

/// Extension trait for [`Result`] to turn errors into typed views.
pub trait ResultExt<T, E> {
    /// Converts the `Err` variant into a view `V`.
    ///
    /// # Errors
    ///
    /// Returns `Err(view(error))` when `self` is `Err(error)`.
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, V>
    where
        Self: Sized;

    /// Converts the `Err` variant into a default [`ErrorView<E>`].
    ///
    /// # Errors
    ///
    /// Returns `Err(ErrorView::new(error))` when `self` is `Err(error)`.
    fn default_error_view(self) -> Result<T, ErrorView<E>>
    where
        Self: Sized,
        E: Display + 'static;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, V> {
        self.map_err(view)
    }

    fn default_error_view(self) -> Result<T, ErrorView<E>>
    where
        E: Display + 'static,
    {
        self.map_err(ErrorView::new)
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use core::fmt;

    use nami::Signal as _;
    use waterui_core::{Environment, View};
    use waterui_text::text;

    use super::{DefaultErrorView, ErrorView, ErrorViewBuilder, ResultExt, error_view};

    #[derive(Debug)]
    struct SampleError(&'static str);

    impl fmt::Display for SampleError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl core::error::Error for SampleError {}

    #[test]
    fn default_error_view_formats_display_text() {
        let rendered = DefaultErrorView.build(SampleError("boom"));
        assert_eq!(rendered.content().snapshot().to_plain().as_str(), "boom");
    }

    #[test]
    fn error_view_builder_replaces_default_renderer() {
        let view = error_view(SampleError("boom")).builder(|err| text(format!("custom: {err}")));
        let _body = view.body(&Environment::new());
    }

    #[test]
    fn result_ext_maps_err_without_erasure() {
        let res: Result<(), SampleError> = Err(SampleError("oops"));
        let err_view = res
            .default_error_view()
            .expect_err("err variant should remain Err");
        assert_eq!(err_view.error().0, "oops");

        let custom: Result<(), SampleError> = Err(SampleError("oops"));
        let custom_view = custom
            .error_view(|e| text(format!("custom: {e}")))
            .expect_err("err variant should map to custom view");
        assert_eq!(
            custom_view.content().snapshot().to_plain().as_str(),
            "custom: oops"
        );
    }

    #[test]
    fn from_error_constructs_default_error_view() {
        let view = ErrorView::from(SampleError("from-err"));
        assert_eq!(view.into_error().0, "from-err");
    }

    #[test]
    fn closure_error_view_builder_infers_parameter_type() {
        let built = ErrorView::with_builder(SampleError("one"), |err| {
            text(format!("custom-builder: {}", err.0))
        });
        assert_eq!(
            built
                .body(&Environment::new())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "custom-builder: one"
        );

        let replaced =
            ErrorView::new(SampleError("two")).builder(|err| text(format!("replaced: {}", err.0)));
        assert_eq!(
            replaced
                .body(&Environment::new())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "replaced: two"
        );
    }
}
