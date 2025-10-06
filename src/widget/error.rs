//! Error handling for the `WaterUI` framework.
//!
//! This module provides comprehensive error handling utilities that seamlessly integrate with the
//! framework's view system. It enables developers to convert standard Rust errors into renderable
//! views, customize error presentation, and handle errors gracefully within the declarative UI paradigm.
//!
//! # Overview
//!
//! The error handling system is built around several key components:
//!
//! - [`Error`]: A type-erased error wrapper that can render as a view
//! - [`DefaultErrorView`]: Environment-based configuration for default error rendering
//! - [`UseDefaultErrorView`]: A view that delegates to the environment's error renderer
//! - [`ResultExt`]: Extension trait for `Result` types to simplify error-to-view conversion
//!
//! # Basic Usage
//!
//! ## Converting Errors to Views
//!
//! Any standard error can be converted to a renderable view:
//!
//! ```rust
//! use waterui::widget::error::Error;
//! use std::io;
//!
//! fn handle_io_error() -> Error {
//!     let io_error = io::Error::new(io::ErrorKind::NotFound, "File not found");
//!     Error::new(io_error)
//! }
//! ```
//!
//! ## Configuring Default Error Views
//!
//! Set up a custom error renderer in your environment:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::widget::error::{BoxedStdError, DefaultErrorView};
//! use waterui::ViewExt;
//! use waterui_layout::stack::vstack;
//!
//! let env = Environment::new().with(DefaultErrorView::new(|error: BoxedStdError| {
//!     let message = error.to_string();
//!     vstack((
//!         text!("❌ Error: {}", message.clone()),
//!         text!("Please contact support if this persists.")
//!             .foreground(Color::srgb(128, 128, 128)),
//!     ))
//! }));
//! ```
//!
//! ## Using Result Extensions
//!
//! Convert `Result` errors to custom views inline:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::widget::error::ResultExt;
//! use waterui::ViewExt;
//!
//! fn load_data() -> Result<String, std::io::Error> {
//!     // ... some operation that might fail
//!     Ok("data".to_string())
//! }
//!
//! fn my_view() -> impl View {
//!     match load_data().error_view(|err| {
//!         let message = err.to_string();
//!         text!("Failed to load: {}", message)
//!     }) {
//!         Ok(data) => text!("{data}").anyview(),
//!         Err(error_view) => error_view.anyview(),
//!     }
//! }
//! ```
//!
//! # Advanced Features
//!
//! ## Type Downcasting
//!
//! Errors can be downcast to specific types for specialized handling:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::widget::error::Error;
//! use std::io;
//!
//! let error = Error::new(io::Error::new(io::ErrorKind::NotFound, "File not found"));
//! match error.downcast::<io::Error>() {
//!     Ok(io_error) => {
//!         // Handle specific IO error
//!         format!("IO Error: {:?}", io_error);
//!     }
//!     Err(original_error) => {
//!         // Handle as generic error
//!         format!("Other error: {:?}", original_error);
//!     }
//! }
//! ```
//!
//! ## Creating Errors from Views
//!
//! Create errors directly from custom views:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::widget::error::Error;
//! use waterui_layout::stack::vstack;
//!
//! let custom_error = Error::from_view(vstack((
//!     text!("Something went wrong!"),
//!     text!("Please try again later."),
//! )));
//! ```
//!
//! # Architecture
//!
//! The error system uses type erasure and trait objects to handle different error types
//! uniformly while preserving the ability to downcast to specific types when needed.
//! The [`DefaultErrorView`] mechanism allows for environment-based error styling
//! configuration that can be inherited throughout the view hierarchy.

use crate::{AnyView, Environment, View};
use alloc::boxed::Box;
use core::{
    any::TypeId,
    fmt::{Debug, Display},
};

/// Re-export of the standard error trait for convenience.
pub use core::error::Error as StdError;

/// Custom error type to use with framework views.
///
/// This type encapsulates any error that can be rendered as a view.
pub struct Error {
    inner: Box<dyn ErrorImpl>,
}

impl_debug!(Error);

/// A boxed standard error trait object.
pub type BoxedStdError = Box<dyn StdError>;

/// A function type that builds a view from a boxed error.
pub type ErrorViewBuilder = Box<dyn Fn(BoxedStdError) -> AnyView>;

trait ErrorImpl: Debug + Display + 'static {
    fn body(self: Box<Self>, _env: Environment) -> AnyView;

    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

impl<E: StdError + 'static> ErrorImpl for E {
    fn body(self: Box<Self>, _env: Environment) -> AnyView {
        AnyView::new(UseDefaultErrorView::new(self))
    }
}

impl Error {
    /// Creates a new `Error` from any type that implements the standard error trait.
    ///
    /// # Arguments
    ///
    /// * `error` - Any error type that implements `StdError` and has static lifetime.
    pub fn new(error: impl StdError + 'static) -> Self {
        Self {
            inner: Box::new(error),
        }
    }

    /// Attempts to downcast the error to a concrete type.
    ///
    /// # Arguments
    ///
    /// * `T` - The type to downcast to.
    ///
    /// # Returns
    ///
    /// A `Result` containing either the boxed downcast type or the original error.
    ///
    /// # Errors
    ///
    /// Returns `Err(self)` if the error cannot be downcast to the specified type `T`.
    pub fn downcast<T: 'static>(self) -> Result<Box<T>, Self> {
        if ErrorImpl::type_id(&*self.inner) == TypeId::of::<T>() {
            unsafe { Ok(Box::from_raw(Box::into_raw(self.inner).cast::<T>())) }
        } else {
            Err(self)
        }
    }

    /// Creates an error directly from a view.
    ///
    /// # Arguments
    ///
    /// * `view` - Any type that implements `View`.
    pub fn from_view(view: impl View) -> Self {
        Self {
            inner: Box::new(ErrorView::new(view)),
        }
    }
}

/// A wrapper that turns a view into an error.
pub struct ErrorView(AnyView);

impl ErrorView {
    fn new(view: impl View) -> Self {
        Self(AnyView::new(view))
    }
}

impl Display for ErrorView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("ErrorView<{}>", self.0.name()))
    }
}

impl Debug for ErrorView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Debug::fmt(&self, f)
    }
}

impl ErrorImpl for ErrorView {
    fn body(self: Box<Self>, _env: Environment) -> AnyView {
        self.0
    }
}

impl View for Error {
    fn body(self, env: &Environment) -> impl View {
        self.inner.body(env.clone())
    }
}

/// Extension trait for `Result` types to easily convert errors to views.
#[allow(clippy::missing_errors_doc)]
pub trait ResultExt<T, E> {
    /// Converts an error to a custom view.
    ///
    /// # Arguments
    ///
    /// * `view` - A function that converts the error to a view.
    ///
    /// # Returns
    ///
    /// A `Result` with the original value or an `Error` containing the view.
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, Error>
    where
        Self: Sized;
}

impl<T, E: Debug + Display + 'static> ResultExt<T, E> for Result<T, E> {
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, Error>
    where
        Self: Sized,
    {
        self.map_err(|error| Error::from_view(view(error)))
    }
}

/// A view that renders an error using the default error view from the environment.
#[derive(Debug)]
pub struct UseDefaultErrorView(BoxedStdError);

impl From<BoxedStdError> for UseDefaultErrorView {
    fn from(value: BoxedStdError) -> Self {
        Self(value)
    }
}

impl UseDefaultErrorView {
    /// Creates a new view that will use the default error rendering.
    ///
    /// # Arguments
    ///
    /// * `error` - Any error type that implements `StdError`.
    pub fn new(error: impl StdError + 'static) -> Self {
        let boxed: BoxedStdError = Box::new(error);
        Self::from(boxed)
    }
}

impl View for UseDefaultErrorView {
    fn body(self, env: &Environment) -> impl View {
        if let Some(builder) = env.get::<DefaultErrorView>() {
            builder.build(self.0)
        } else {
            AnyView::new(())
        }
    }
}

/// A configurator for the default error view.
///
/// This can be placed in the environment to define how errors should be displayed.
pub struct DefaultErrorView(ErrorViewBuilder);
impl_debug!(DefaultErrorView);

impl DefaultErrorView {
    /// Creates a new default error view builder.
    ///
    /// # Arguments
    ///
    /// * `builder` - A function that creates a view from a boxed error.
    pub fn new<V: View>(builder: impl 'static + Fn(BoxedStdError) -> V) -> Self {
        Self(Box::new(move |error| AnyView::new(builder(error))))
    }

    /// Builds a view from a boxed error using the configured builder.
    ///
    /// # Arguments
    ///
    /// * `error` - A boxed error to render.
    ///
    /// # Returns
    ///
    /// An `AnyView` containing the rendered error.
    pub fn build(&self, error: BoxedStdError) -> AnyView {
        (self.0)(error)
    }
}
