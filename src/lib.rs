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
pub mod gesture;
/// Task management utilities and async support.
pub mod task;
pub mod view;
/// Widget components for building complex UI elements.
pub mod widget;
#[doc(inline)]
pub use view::View;
pub mod accessibility;
#[doc(inline)]
pub use waterui_derive::*;
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
    pub use super::*;
    pub use color::*;

    pub use component::*;
    pub use form::*;
    pub use layout::*;
    pub use media::*;
    pub use navigation::*;
    pub use padding::*;
    pub use style::*;

    pub use text::{
        TextConfig, font, highlight,
        link::{Link, link},
        locale, styled,
    };

    pub use widget::{Card, Divider, card, suspense};
}
pub use color::Color;
#[doc(inline)]
pub use view::ViewExt;
pub use waterui_color as color;
pub use waterui_form as form;
pub use waterui_layout as layout;
pub use waterui_media as media;
pub use waterui_navigation as navigation;
pub use waterui_text as text;
pub mod style;

#[doc(inline)]
pub use waterui_core::{
    AnyView, Str, animation,
    env::{self, Environment},
    impl_extractor, raw_view, views,
};

/// Creates a reactive text component with formatted content.
///
/// This macro provides a convenient way to create text components with
/// formatted content that automatically updates when reactive values change.
///
/// # Usage
///
/// ```ignore
/// let name = binding("World");
/// let greeting = text!("Hello, {}!", name);
/// ```
#[macro_export]
macro_rules! text {
    ($($arg:tt)*) => {
        {
            #[allow(unused_parens)]
            $crate::text::Text::new($crate::s!($($arg)*))
        }
    };
}

mod ext;
pub use ext::SignalExt;
pub use nami as reactive;
#[doc(inline)]
pub use reactive::{Binding, Computed, Signal, signal};
pub use task::task;

pub mod hot_reload;

mod entry;
pub use entry::entry;
