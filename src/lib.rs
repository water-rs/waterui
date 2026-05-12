#![doc = include_str!("../README.md")]
extern crate alloc;
extern crate self as waterui;
#[macro_use]
mod macros;
mod appearance;
pub use appearance::{background, border, filter, gradient, shape, style};
pub mod component;
mod interaction_support;
pub use interaction_support::{cursor, drag_drop, gesture, interaction};
mod runtime;
pub use runtime::{app, entry, error, fullscreen, inspector, metadata, snackbar, task, window};
/// Task management utilities and async support.
pub mod view;
/// Widget components for building complex UI elements.
pub mod widget;
#[doc(inline)]
pub use view::View;
pub use waterui_core::accessibility;

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
    #[cfg(feature = "barcode")]
    pub use super::barcode;
    #[cfg(feature = "chart")]
    pub use super::chart;
    pub use super::env::Environment;
    #[cfg(feature = "map")]
    pub use super::map;
    #[cfg(feature = "media")]
    pub use super::media;
    #[cfg(feature = "particle")]
    pub use super::particle;
    #[cfg(feature = "video")]
    pub use super::video;
    #[cfg(feature = "webview")]
    pub use super::webview;
    pub use super::{
        AnimationExt, AnyView, Binding, Color, Computed, FilterViewExt, Signal, SignalExt, State,
        Str, View, ViewExt, accessibility, animation, app, color, component, cursor, drag_drop,
        entry, env, error, filter, form, fullscreen, gesture, gradient, id, layout, locale,
        metadata, navigation, reactive, regional, shape, signal, style, task, text, widget, window,
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
    pub use super::component::list::{
        List, ListContent, ListItem, ListSection, Row, Section, detail_row, row,
    };
    pub use super::component::menu::{
        Command, CommandExt, Menu, MenuItem, Shortcut, ShortcutModifiers,
    };

    // Drag and drop extension traits
    pub use super::drag_drop::DropDestinationExt;

    pub use super::widget::{Card, Divider, card, suspense};
    #[cfg(feature = "flow-markdown")]
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
    #[cfg(feature = "assets")]
    pub use super::{
        AssetError, AssetKind, AudioAsset, Bundle, Data, DataAsset, FontAsset, ImageAsset,
        LargeFile, LargeFileAsset, VideoAsset, asset, assets, include_bundle,
    };

    // Re-export macros
    pub use waterui_macros::*;
}
pub use color::Color;
pub use form::FormBuilder;
#[doc(inline)]
pub use view::{FilterViewExt, ViewExt};
#[cfg(feature = "barcode")]
pub use waterui_barcode as barcode;
#[cfg(feature = "chart")]
pub use waterui_chart as chart;
pub use waterui_form as form;
pub use waterui_graphics::color;
pub use waterui_graphics::image_analysis;
pub use waterui_graphics::image_generator;
pub use waterui_graphics::{
    CheckerboardGenerator, DominantColor, DotGridGenerator, GeneratedImage, Histogram,
    ImageAnalysis, ImageGenerator, LinearGradientGenerator, MinMaxLuma, NoiseGenerator,
    RadialGradientGenerator, StripeGenerator,
};
pub use waterui_icon as icon;

#[cfg(feature = "assets")]
pub use waterui_assets as assets;
pub use waterui_layout as layout;
pub use waterui_locale as locale;
pub use waterui_locale::regional;
#[doc(inline)]
pub use waterui_macros::*;
#[cfg(feature = "map")]
pub use waterui_map as map;
#[cfg(feature = "media")]
pub use waterui_media as media;
pub use waterui_navigation as navigation;
#[cfg(feature = "particle")]
pub use waterui_particle as particle;
pub use waterui_svg as svg;
pub use waterui_text as text;
#[cfg(feature = "video")]
pub use waterui_video as video;
#[cfg(feature = "webview")]
pub use waterui_webview as webview;

// Asset types re-exported for convenience
#[doc(inline)]
#[cfg(feature = "assets")]
pub use waterui_assets::{
    AssetError, AssetKind, AudioAsset, Bundle, Data, DataAsset, FontAsset, ImageAsset, LargeFile,
    LargeFileAsset, VideoAsset,
};
#[doc(inline)]
#[cfg(feature = "assets")]
pub use waterui_assets_macros::{asset, assets, include_bundle};
pub use waterui_url::Url;

#[doc(inline)]
pub use waterui_core::{
    AnyView, Str, animation,
    env::{self, Environment},
    extract::State,
    id::{self, Identifiable},
    impl_extractor, raw_view, views,
};

pub(crate) mod view_ext;
pub use nami as reactive;
pub use nami::SignalExt;
#[doc(inline)]
pub use reactive::{Binding, Computed, Signal, signal};
pub use runtime::reactive_ext::AnimationExt;

pub use waterui_core::plugin::Plugin;
/// Graphics primitives including GPU rendering surface.
pub use waterui_graphics as graphics;

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

/// Configures a freshly-created environment with compile-time discovered app plugins.
///
/// This currently installs the runtime translation catalog generated from the caller's
/// `i18n/*.toml` files. It is intended to be used at environment creation boundaries
/// such as backend entry points.
#[macro_export]
macro_rules! configure_environment {
    ($env:expr) => {{
        let mut __waterui_env = $env;
        $crate::Plugin::install($crate::catalog!(), &mut __waterui_env);
        __waterui_env
    }};
}
