#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_crate_versions)]

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
pub mod gesture;
#[doc(inline)]
pub use view::View;
pub mod accessibility;
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
    pub use waterui_color::Color;
    pub use waterui_core::{
        AnyView, animation,
        env::{self, Environment},
        impl_extractor, raw_view,
    };
    pub use waterui_text::text;
}
#[doc(inline)]
pub use view::ViewExt;
pub use waterui_color as color;
pub use waterui_color::Color;
#[doc(inline)]
pub use waterui_core::{
    AnyView, animation,
    env::{self, Environment},
    impl_extractor, raw_view,views
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
//pub mod hot_reload; Hot reload is tough :cry
