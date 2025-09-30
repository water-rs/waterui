#![doc = include_str!("../README.md")]

extern crate alloc;

#[macro_use]
mod macros;
pub mod background;
pub mod component;
/// Error handling utilities for converting standard errors into renderable views.
pub mod error;
pub mod filter;
/// Task management utilities and async support.
pub mod task;
pub mod view;
/// Widget components for building complex UI elements.
pub mod widget;
#[doc(inline)]
pub use view::View;
pub mod prelude {
    //! A collection of commonly used traits and types for easy importing.
    //!
    //! This module re-exports essential components from the library, allowing users to
    //! import them all at once with a single `use` statement. It includes traits for
    //! building views, handling signals, and working with colors and text.
    //!
    //! # Example
    //!
    //! ```rust
    //! use waterui::prelude::*;
    //!
    //! fn my_view() -> impl View {
    //!     // Your view implementation here
    //! }
    //! ```
    pub use crate::{Str, background, component::*, filter, task, view::View, widget};
    pub use nami::{Binding, Computed, Signal, signal};
    pub use waterui_core::{
        AnyView, Color, animation,
        env::{self, Environment},
        impl_extractor, raw_view, shape,
    };
    pub use waterui_text::text;
}
#[doc(inline)]
pub use view::ViewExt;
#[doc(inline)]
pub use waterui_core::{
    AnyView, Color, animation,
    env::{self, Environment},
    impl_extractor, raw_view, shape,
};
pub use waterui_text::text;

#[doc(inline)]
pub use nami::{Binding, Computed, Signal, signal};
mod ext;
pub use ext::SignalExt;
pub use nami as reactive;
pub use task::task;
#[doc(inline)]
pub use waterui_core as core;
pub use waterui_str::Str;

#[macro_export]
macro_rules! wrapper {
    ($ident:ident,$ty:ty) => {
        #[derive(Debug, Clone)]
        pub struct $ident(pub $ty);

        impl From<$ty> for $ident {
            fn from(value: $ty) -> Self {
                Self(value)
            }
        }

        impl From<$ident> for $ty {
            fn from(value: $ident) -> Self {
                value.0
            }
        }

        impl core::ops::Deref for $ident {
            type Target = $ty;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::ops::DerefMut for $ident {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}
