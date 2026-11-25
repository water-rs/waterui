#![doc = include_str!("README.md")]
extern crate alloc;
extern crate self as waterui;
mod appearance;
pub use appearance::{background, border, filter, floating, shape, style};
pub mod component;
mod interaction_support;
pub use interaction_support::{cursor, drag_drop, gesture, interaction};
#[cfg(feature = "haptics")]
pub mod haptic;
mod runtime;
#[cfg(feature = "dialog")]
pub use runtime::dialog;
#[cfg(feature = "snackbar")]
pub use runtime::snackbar;
pub use runtime::{app, metadata, sheet, task, window};
pub use widget::error;
// Inspection is a TCP endpoint that also launches the inspector application on
// the developer's machine; a browser page can do neither, so on wasm the
// endpoint is absent rather than present and permanently unable to work.
#[cfg(all(feature = "inspector", not(target_arch = "wasm32")))]
pub use waterui_inspector_runtime as inspector;
/// Task management utilities and async support.
pub mod view;
/// Widget components for building complex UI elements.
pub mod widget;
#[doc(inline)]
pub use view::View;
pub use waterui_accessibility as accessibility;

/// The shared `waterui_meta_*` record emitters — `dir_static!` lands its
/// section and retention cfg at the invoking crate's profile — re-exported
/// for the asset macros that emit into user code. Gated on `assets`:
/// generated expansions only reference `__meta` when that feature's code
/// exists.
#[cfg(feature = "assets")]
#[doc(hidden)]
pub use waterui_meta as __meta;

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
    #[cfg(feature = "effects")]
    pub use super::FilterViewExt;
    pub use super::app::{App, AppMenuBar, Windows};
    pub use super::env::Environment;
    #[cfg(feature = "form")]
    pub use super::form;
    #[cfg(feature = "haptics")]
    pub use super::haptic::{Feedback, Haptic, Intensity};
    #[cfg(feature = "locale-datetime")]
    pub use super::locale::LocalizedDateTimeExt;
    #[cfg(feature = "image")]
    pub use super::media;
    #[cfg(feature = "video")]
    pub use super::video;
    #[cfg(feature = "webview")]
    pub use super::webview;
    pub use super::window::{Window, WindowState};
    pub use super::{
        AnimationExt, AnyView, Binding, Color, Computed, Identifiable, OneOf2, OneOf3, OneOf4,
        OneOf5, OneOf6, OneOf7, OneOf8, Project, Signal, SignalExt, State, Str, View, ViewExt,
        accessibility, animation, app, binding, color, component, cursor, drag_drop, env, error,
        filter, gesture, gradient, id, layout, locale, metadata, reactive, regional, shape, sheet,
        signal, style, task, text, widget, window,
    };

    #[cfg(feature = "flow-markdown")]
    pub use waterui_markdown::include_markdown;

    pub use super::color::*;
    #[cfg(feature = "dialog")]
    pub use super::dialog::{Dialog, DialogAction};
    pub use super::sheet::{Sheet, SheetDetent, SheetDismiss};
    #[cfg(feature = "snackbar")]
    pub use super::snackbar::{
        Snackbar, SnackbarDismissReason, SnackbarHost, SnackbarManager, SnackbarPosition,
        SnackbarTheme,
    };

    pub use super::border::Border;
    pub use super::component::badge::BadgeValue;
    pub use super::component::*;
    #[cfg(feature = "form")]
    pub use super::form::*;
    pub use super::layout::padding::*;
    pub use super::layout::*;
    #[cfg(feature = "navigation")]
    pub use super::navigation::{self, *};
    pub use super::reactive::collection::{SignalCollection, StateList};
    pub use super::style::*;
    pub use super::views::ForEach;
    pub use waterui_core::dynamic::{DynamicHandler, watch};

    pub use super::theme::{self, ColorScheme, ColorSettings, FontSettings, Theme, Tint};

    pub use super::text::{TextConfig, font, highlight, styled};

    #[cfg(feature = "link")]
    pub use super::component::link::{Link, LinkTarget, link};
    #[cfg(feature = "list")]
    pub use super::component::list::{
        ForEachExt, List, ListContent, ListMinRowHeight, Row, Section, detail_row, row,
    };
    #[cfg(feature = "menu")]
    pub use super::component::menu::{
        Command, CommandExt, Menu, MenuItem, NamedKey, Shortcut, ShortcutKey, ShortcutModifiers,
        ShortcutNamedKey,
    };
    #[cfg(feature = "table")]
    pub use super::component::table::{
        SortDirection, SortableTableColumn, Table, TableColumn, TableSort, col, table,
    };
    #[cfg(feature = "menu")]
    pub use super::metadata::context_menu::{ContextMenu, DismissContextMenu};
    pub use super::metadata::secure::DynamicRange;

    // Drag and drop extension traits
    pub use super::drag_drop::DropDestinationExt;

    pub use super::widget::{
        Accordion, Card, CardStyle, CardStyleTokens, CardTheme, DialogTheme, Divider, ErrorView,
        ResultExt, SuspendedView, Suspense, accordion, card, condition::when, error_view, suspense,
    };
    #[cfg(feature = "avatar")]
    pub use super::widget::{Avatar, avatar};
    #[cfg(feature = "flow-markdown")]
    pub use super::widget::{
        FlowAnimationPolicy, FlowAnimationPreset, FlowElementKind, FlowMarkdown, FlowStreamMode,
        FlowTablePolicy, flow_markdown,
    };

    // Gradient views
    #[cfg(any(feature = "gpu", feature = "cpu"))]
    pub use super::gradient::FlowingGradient;
    pub use super::gradient::{AnimatedMeshGradient, AnimatedMeshGradientConfig};
    pub use super::gradient::{Gradient, GradientStops, MeshGradient};

    // Background types (explicit to avoid module name conflict with layout::background)
    pub use super::background::{Glass, GlassStyle, Material};

    // Asset types
    #[cfg(all(feature = "assets", feature = "image", not(target_arch = "wasm32")))]
    pub use super::ImageAsset;
    #[cfg(all(feature = "assets", feature = "video", not(target_arch = "wasm32")))]
    pub use super::VideoAsset;
    #[cfg(feature = "assets")]
    pub use super::{AssetError, AssetKind, Data, asset, assets, include_bundle};
    #[cfg(all(feature = "assets", not(target_arch = "wasm32")))]
    pub use super::{
        AudioAsset, Bundle, DataAsset, FontAsset, LargeFile, LargeFileAsset, ResourceContext,
    };

    // Re-export macros. The UI-test attribute is `ui_test` here rather than
    // `test`: glob-importing a macro named `test` shadows the built-in `#[test]`
    // in every file that writes `use waterui::prelude::*`, and the resulting
    // error names neither the prelude nor the collision.
    #[cfg(feature = "form")]
    pub use waterui_macros::FormBuilder;
    #[cfg(any(feature = "ts", feature = "webview"))]
    pub use waterui_macros::TsType;
    #[cfg(feature = "ts")]
    pub use waterui_macros::{TsProps, tsx};
    pub use waterui_macros::{bench, catalog, preview, state, ui_test, view, view_builder};
    #[cfg(feature = "webview")]
    pub use waterui_macros::{eval, exec, exec_file, js_api, js_file};
}
pub use color::Color;
pub use component::badge::BadgeValue;
#[cfg(feature = "form")]
pub use form::FormBuilder;
#[cfg(feature = "effects")]
#[doc(inline)]
pub use view::FilterViewExt;
#[doc(inline)]
pub use view::ViewExt;
#[cfg(feature = "barcode")]
pub use waterui_barcode as barcode;
#[cfg(feature = "chart")]
pub use waterui_chart as chart;
#[cfg(feature = "form")]
pub use waterui_form as form;
#[cfg(feature = "canvas")]
pub use waterui_graphics::canvas;
pub use waterui_graphics::color;
pub use waterui_graphics::gradient;
#[cfg(feature = "gpu")]
pub use waterui_graphics::{GpuContent, GpuView, RedrawHandle};
#[cfg(any(feature = "gpu", feature = "cpu"))]
pub use waterui_graphics::{SHADER_MAX_PARAMS, ShaderError, ShaderPaintView};
pub use waterui_icon as icon;
#[cfg(feature = "particle")]
pub use waterui_particle as particle;

#[cfg(feature = "assets")]
pub use waterui_assets as assets;
pub mod layout {
    //! Layout primitives re-exported from [`waterui_layout`], together with [`Divider`].
    pub use crate::widget::Divider;
    pub use waterui_layout::*;
}
pub use waterui_locale as locale;
pub use waterui_locale::regional;
#[cfg(any(feature = "ts", feature = "webview"))]
#[doc(inline)]
pub use waterui_macros::TsType;
/// The UI-test attribute, written qualified as `#[waterui::test(...)]`.
///
/// It is `ui_test` inside [`prelude`] so that a glob import of the prelude does
/// not shadow the built-in `#[test]`; this alias keeps the short spelling at the
/// path where it is actually written.
pub use waterui_macros::ui_test as test;
#[cfg(feature = "form")]
#[doc(inline)]
pub use waterui_macros::{FormBuilder, form};
#[doc(inline)]
pub use waterui_macros::{Identifiable, bench, catalog, preview, state, text, view, view_builder};
#[cfg(feature = "ts")]
#[doc(inline)]
pub use waterui_macros::{TsProps, tsx};
#[cfg(feature = "webview")]
#[doc(inline)]
pub use waterui_macros::{eval, exec, exec_file, js_api, js_file};
#[cfg(feature = "widgets")]
#[doc(inline)]
pub use waterui_macros::{widget, widget_intent};
#[cfg(feature = "image")]
pub use waterui_media as media;
#[cfg(feature = "navigation")]
pub use waterui_navigation as navigation;
#[cfg(feature = "gpu")]
pub use waterui_svg as svg;
pub use waterui_text as text;
#[cfg(feature = "ts")]
pub mod ts;
#[cfg(feature = "map")]
pub use waterui_map as map;
#[cfg(feature = "flow-markdown")]
pub use waterui_markdown as markdown;
#[cfg(feature = "flow-markdown")]
#[doc(inline)]
pub use waterui_markdown::include_markdown;
#[cfg(feature = "ts")]
#[doc(hidden)]
pub use waterui_ts::schema as __ts_schema;
#[cfg(feature = "video")]
pub use waterui_video as video;
#[cfg(feature = "webview")]
pub use waterui_webview as webview;
#[cfg(all(not(feature = "ts"), feature = "webview"))]
#[doc(hidden)]
pub use waterui_webview::ts_schema as __ts_schema;
/// The `WidgetKit` widget authoring surface: the closed snapshot schema a
/// `#[waterui::widget]` function produces, dated per timeline entry, and the
/// restricted constructors that build it.
#[cfg(feature = "widgets")]
pub use waterui_widgets as widgets;

// Asset types re-exported for convenience
#[cfg(all(feature = "assets", feature = "image", not(target_arch = "wasm32")))]
pub use waterui_assets::ImageAsset;
#[cfg(all(feature = "assets", feature = "video", not(target_arch = "wasm32")))]
pub use waterui_assets::VideoAsset;
#[doc(inline)]
#[cfg(all(feature = "assets", not(target_arch = "wasm32")))]
pub use waterui_assets::{
    AssetError, AssetKind, AudioAsset, Bundle, Data, DataAsset, FontAsset, LargeFile,
    LargeFileAsset,
};
#[cfg(all(feature = "assets", target_arch = "wasm32"))]
pub use waterui_assets::{AssetError, AssetKind, Data};
/// `include_web!("web")` — the one-macro web frontend. Its expansion speaks
/// [`webview`](crate::webview)'s asset-origin API and serves a staged
/// [`Bundle`] resolved from the rendering environment's [`ResourceContext`], so
/// it is exported only when both features are on.
#[doc(inline)]
#[cfg(all(feature = "webview", feature = "assets"))]
pub use waterui_assets_macros::include_web;
#[doc(inline)]
#[cfg(feature = "assets")]
pub use waterui_assets_macros::{asset, assets, include_bundle};
pub use waterui_resources::ResourceContext;
pub use waterui_url::Url;

#[doc(inline)]
#[cfg(feature = "capture")]
pub use waterui_capture::{CaptureHandle, CaptureScope, UnwiredCaptureError};
#[doc(inline)]
pub use waterui_capture::{
    ContentLedger, RenderError, RenderRequest, RenderResult, ViewRenderer, content_ledger,
    view_renderer,
};
#[doc(inline)]
pub use waterui_core::{
    AnyView, Error, OneOf2, OneOf3, OneOf4, OneOf5, OneOf6, OneOf7, OneOf8, Str, animation,
    contract_violation,
    easing::{self, EasingCurve, Interpolatable},
    env::{self, Environment},
    extract::{self, Extractor, State},
    handler::{self, Handler, HandlerOnce},
    id::{self, Identifiable},
    raw_view,
    resolve::{self, AnyResolvable, Resolvable},
    views,
};
#[doc(inline)]
pub use waterui_interaction::{event, key};

// Selective re-exports only: `waterui::transition` stays free for the
// `waterui-transition` pixel-effects crate re-export (#1389).
pub use waterui_structural_transition::{
    AnyTransition, Edge, PropertyTransition, ReduceMotion, Transition, TransitionContext,
    TransitionFrame, TransitionGeometry, TransitionPhase, TransitionProperties, Transitioned,
};

/// Declarative and imperative haptic feedback primitives.
#[cfg(feature = "haptics")]
#[doc(inline)]
pub use haptic::{Feedback, Haptic, Intensity};

pub use nami as reactive;
pub use nami::SignalExt;
#[doc(inline)]
pub use reactive::{Binding, Computed, Project, Signal, binding, signal};
#[doc(inline)]
pub use waterui_core::AnimationExt;

pub use waterui_core::plugin::Plugin;
/// Graphics primitives including GPU rendering surface.
pub use waterui_graphics as graphics;

pub use tracing as log;

/// Internal helper macro for generating preview export symbols.
///
/// Symbol format: `waterui_preview_{crate_name}_{fn_name}`
///
/// The export exists in debug builds of the emitting crate — the same gate
/// the compiler-emitted `waterui_meta_*` records use — so a release build
/// never carries the entry point or the view it forces into the artifact.
/// The preview host loads it from the debug artifact, whether that is the
/// dynamic-linking desktop dylib or the static Android cdylib.
#[doc(hidden)]
#[macro_export]
macro_rules! __export_preview {
    ($fn_name:expr, $body:block) => {
        $crate::pastey::paste! {
            #[doc(hidden)]
            #[cfg(debug_assertions)]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_preview_ env!("CARGO_PKG_NAME") _ $fn_name>]() -> *mut () {
                $body
            }
        }
    };
}

#[doc(hidden)]
pub use pastey;

/// Expands `waterui-ts` runtime conversions (`IntoJs` / `FromJs`) only when the
/// `ts` feature is enabled on `waterui`, so `#[derive(TsType)]` works cleanly
/// in `webview`-only crates without requiring the JS view runtime.
#[cfg(feature = "ts")]
#[doc(hidden)]
#[macro_export]
macro_rules! __with_ts_runtime {
    ($($item:tt)*) => {
        $($item)*
    };
}

/// No-op expansion when the `ts` feature is disabled.
#[cfg(not(feature = "ts"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __with_ts_runtime {
    ($($item:tt)*) => {};
}

/// Capture standalone resource roots once when creating an application environment.
///
/// # Contract
/// When the native executable's location cannot be determined a debug build
/// panics, and a release build logs the error and installs a context rooted
/// at `waterui_assets` and `fonts` in the working directory.
#[doc(hidden)]
#[cfg_attr(target_arch = "wasm32", allow(clippy::missing_const_for_fn))]
pub fn install_application_resources(env: &mut Environment) {
    #[cfg(not(target_arch = "wasm32"))]
    if env.get::<ResourceContext>().is_none() {
        let context = ResourceContext::application().unwrap_or_else(|error| {
            contract_violation!(
                "application resources could not be located ({error}); resolving them against the working directory"
            );
            ResourceContext::new("waterui_assets", "fonts")
        });
        env.insert(context);
    }
    #[cfg(feature = "image")]
    if env.get::<waterui_media::PhotoCache>().is_none() {
        env.insert(waterui_media::PhotoCache::default());
    }
    #[cfg(all(target_arch = "wasm32", not(feature = "image")))]
    let _ = env;
}

/// Plugin that installs standalone application resources (`ResourceContext` and
/// default media caches) into an environment without overwriting existing entries.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ApplicationResources;

impl Plugin for ApplicationResources {
    fn install(self, env: &mut Environment) {
        install_application_resources(env);
    }
}

/// Configures a freshly-created environment with compile-time discovered app plugins.
///
/// This currently installs the runtime translation catalog generated from the caller's
/// `i18n/*.toml` files — or the directory `WATERUI_I18N_DIR` names when the caller is a
/// CLI-generated backend crate without an `i18n/` of its own. It is intended to be used
/// at environment creation boundaries such as backend entry points.
#[macro_export]
macro_rules! configure_environment {
    ($env:expr) => {{
        let mut __waterui_env = $env;
        $crate::Plugin::install($crate::ApplicationResources, &mut __waterui_env);
        $crate::Plugin::install($crate::catalog!(), &mut __waterui_env);
        __waterui_env
    }};
}

#[cfg(all(test, feature = "widgets"))]
mod widgets_glob_tests {
    use crate::prelude::*;
    use crate::widgets::*;

    #[test]
    fn widgets_root_globs_cleanly_alongside_prelude() {
        let _app_view = vstack((text("App"), spacer())).padding();
        let _now = Date::now();
        let _timeline = {
            use crate::widgets::prelude::*;

            Timeline::new([TimelineEntry::new(
                Date::unix_timestamp(1_700_000_000.0),
                vstack((
                    text("Steps").headline(),
                    spacer(),
                    text(Date::now()).relative(),
                    image_system("figure.walk").a11y_label("Walking"),
                ))
                .padding()
                .background(Color::ACCENT),
            )])
        };
        assert_eq!(_timeline.entries().len(), 1);
    }
}
