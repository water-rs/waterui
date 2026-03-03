#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::future_not_send)]
#![allow(clippy::doc_markdown)]
extern crate alloc;
extern crate self as waterui;
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
pub mod interaction;
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
        AnimationExt, AnyView, Binding, Color, Computed, Signal, SignalExt, Str, View, ViewExt,
        accessibility, animation, app, color, component, cursor, drag_drop, entry, env, error,
        filter, form, fullscreen, gesture, gradient, id, layout, locale, media, metadata,
        navigation, reactive, regional, shape, signal, style, task, text, video, webview, widget,
        window,
    };

    pub use crate::include_markdown;

    pub use super::color::*;
    pub use super::fullscreen::*;
    pub use super::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};

    pub use super::gesture::GestureObserver;

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

    // Drag and drop extension traits
    pub use super::drag_drop::DropDestinationExt;

    pub use super::widget::{Card, Divider, card, suspense};
    pub use super::widget::{
        FlowAnimationPolicy, FlowAnimationPreset, FlowElementKind, FlowMarkdown, FlowStreamMode,
        FlowTablePolicy, flow_markdown,
    };

    // Gradient types
    pub use super::gradient::{
        AngularGradient, ColorStop, Gradient, LinearGradient, MeshGradient, MeshVertex,
        RadialGradient, UnitPoint,
    };

    // Background types (explicit to avoid module name conflict with layout::background)
    pub use super::background::{Background, Material, Shader};

    // Asset types
    pub use super::{AssetError, AssetKind, Data, LargeFile, asset};

    // Re-export macros
    pub use waterui_macros::*;
}
pub use color::Color;
pub use form::FormBuilder;
#[doc(inline)]
pub use view::ViewExt;
pub use waterui_form as form;
pub use waterui_graphics::color;

pub use waterui_assets as assets;
pub use waterui_layout as layout;
pub use waterui_locale as locale;
pub use waterui_locale::regional;
#[doc(inline)]
pub use waterui_macros::*;
pub use waterui_media as media;
pub use waterui_navigation as navigation;
pub use waterui_svg as svg;
pub use waterui_text as text;
pub use waterui_video as video;
pub use waterui_webview as webview;

// Asset types re-exported for convenience
#[doc(inline)]
pub use waterui_assets::{AssetError, AssetKind, Data, LargeFile};
#[doc(inline)]
pub use waterui_assets_macros::asset;
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
pub use nami::SignalExt;
#[doc(inline)]
pub use reactive::{Binding, Computed, Signal, signal};
pub use reactive_ext::AnimationExt;

/// Task management utilities and async support.
pub mod task;

/// Inspector runtime endpoint and diagnostics streaming.
pub mod inspector;

/// Graphics primitives including GPU rendering surface.
pub use waterui_graphics as graphics;

mod entry;
pub use entry::entry;

pub mod app;
pub mod fullscreen;
pub mod snackbar;
pub mod window;

pub use tracing as log;

/// Internal helper macro for generating preview export symbols.
///
/// Symbol format: `waterui_preview_{crate_name}_{fn_name}`
#[doc(hidden)]
#[macro_export]
macro_rules! __export_preview {
    ($fn_name:expr, $body:block) => {
        $crate::pastey::paste! {
            #[doc(hidden)]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_preview_ env!("CARGO_PKG_NAME") _ $fn_name>]() -> *mut () {
                $body
            }
        }
    };
}

#[doc(hidden)]
pub use pastey;
