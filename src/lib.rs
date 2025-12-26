#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::future_not_send)]
#![allow(clippy::doc_markdown)]
extern crate alloc;
#[macro_use]
mod macros;
pub mod background;
pub mod border;
pub mod component;
pub mod cursor;
pub mod drag_drop;
/// Error handling utilities for converting standard errors into renderable views.
pub mod error;
pub mod filter;
pub mod gesture;
pub mod gradient;
/// Task management utilities and async support.
pub mod view;
/// Widget components for building complex UI elements.
pub mod widget;
#[doc(inline)]
pub use view::View;
pub mod accessibility;

pub mod theme;
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
    // Re-export core modules from super, excluding `background` to avoid conflict with layout::background
    pub use super::env::Environment;
    pub use super::{
        AnyView, Binding, Color, Computed, Signal, SignalExt, Str, View, ViewExt, accessibility,
        animation, app, color, component, cursor, drag_drop, entry, env, error, filter, form,
        fullscreen, gesture, gradient, id, layout, locale, media, metadata, navigation, reactive,
        shape, signal, style, task, text, webview, widget, window,
    };

    // Filter extension trait for GPU filters
    pub use super::graphics::FilterViewExt;

    pub use super::color::*;
    pub use super::fullscreen::*;

    pub use super::border::Border;
    pub use super::component::*;
    pub use super::form::*;
    pub use super::layout::padding::*;
    pub use super::layout::*;
    pub use super::navigation::*;
    pub use super::style::*;
    pub use waterui_core::dynamic::{DynamicHandler, watch};

    pub use super::theme::{
        self, ColorScheme, ColorSettings, FontSettings, Theme, color as theme_color,
    };

    pub use super::text::{TextConfig, font, highlight, styled};

    pub use super::component::link::{Link, link};
    pub use super::component::menu::{Menu, MenuItem};

    pub use super::widget::{Card, Divider, card, suspense};

    // Gradient types
    pub use super::gradient::{
        AngularGradient, ColorStop, Gradient, LinearGradient, MeshGradient, MeshVertex,
        RadialGradient, UnitPoint,
    };

    // Background types (explicit to avoid module name conflict with layout::background)
    pub use super::background::{Background, Material, Shader};

    // Re-export macros
    pub use waterui_macros::hot_reload;
}
pub use color::Color;
pub use form::FormBuilder;
#[doc(inline)]
pub use view::ViewExt;
pub use waterui_form as form;
pub use waterui_graphics::color;

pub use waterui_layout as layout;
pub use waterui_locale as locale;
#[doc(inline)]
pub use waterui_macros::*;
pub use waterui_media as media;
pub use waterui_navigation as navigation;
pub use waterui_svg as svg;
pub use waterui_text as text;
pub use waterui_webview as webview;
pub mod metadata;
pub mod shape;
pub mod style;

#[doc(inline)]
pub use waterui_core::{
    AnyView, Str, animation,
    env::{self, Environment},
    id::{self, Identifiable},
    impl_extractor, raw_view, views,
};

mod reactive_ext;
pub(crate) mod view_ext;
pub use nami as reactive;
#[doc(inline)]
pub use reactive::{Binding, Computed, Signal, signal};
pub use reactive_ext::SignalExt;

/// Task management utilities and async support.
pub mod task {
    pub use executor_core::{spawn, spawn_local};
    pub use native_executor::sleep;
}

/// Graphics primitives including GPU rendering surface.
pub use waterui_graphics as graphics;

#[cfg(debug_assertions)]
#[macro_use]
pub mod debug;

mod entry;
pub use entry::entry;

pub mod app;
pub mod fullscreen;
pub mod window;

pub use tracing as log;
