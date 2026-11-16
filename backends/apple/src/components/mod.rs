//! The component ports this backend owns.
//!
//! One module per claimed view type: each registers its handler on the
//! dispatcher through [`crate::registry`]. Everything a module needs beyond
//! the [`crate::contract`] is a `cocoa-ui` addition, never a reactivity type
//! crossing into the kit.

pub mod accessibility_identifier;
pub mod accessibility_metadata;
pub mod anchored_overlay;
pub mod badge;
pub mod border;
pub mod button;
mod button_chrome;
#[cfg(any(feature = "calendar", feature = "multi_date_picker"))]
pub mod calendar;
#[cfg(feature = "canvas")]
pub mod canvas;
pub mod clip_shape;
#[cfg(feature = "color_picker")]
pub mod color_picker;
pub mod container;
#[cfg(feature = "context_menu")]
pub mod context_menu;
#[cfg(any(test, feature = "slider", target_os = "macos"))]
pub mod control_size;
pub mod cursor;
#[cfg(feature = "date_picker")]
pub mod date_picker;
#[cfg(feature = "dialog")]
pub mod dialog;
pub mod draggable;
pub mod drop_destination;
pub mod dynamic;
pub mod dynamic_range;
pub mod empty;
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
pub mod filtered;
pub mod fixed_container;
pub mod focused;
mod font;
pub mod gesture;
pub mod glass_background;
#[cfg(feature = "gpu_surface")]
pub mod gpu_surface;
pub mod hittable;
pub mod ignore_safe_area;
pub mod image;
#[cfg(target_os = "macos")]
pub mod layer_transform;
pub mod layout_priority;
pub mod lifecycle_hook;
#[cfg(feature = "list")]
pub mod list;
#[cfg(feature = "map")]
pub mod map;
pub mod material_background;
#[cfg(feature = "menu")]
pub mod menu;
#[cfg(any(feature = "menu", feature = "context_menu", feature = "text_field"))]
mod menu_items;
#[cfg(feature = "multi_date_picker")]
pub mod multi_date_picker;
/// The AppKit-native `CIFilter` realization of portable chains — the
/// filtered leaf's macOS fast path (#1748).
#[cfg(all(target_os = "macos", feature = "applied_filter"))]
pub mod native_filter;
#[cfg(feature = "navigation")]
pub mod navigation;
pub mod offset;
pub mod on_event;
pub mod on_key_press;
pub mod opacity;
pub mod passthrough;
#[cfg(feature = "picker")]
pub mod picker;
#[cfg(feature = "picture")]
pub mod picture;
#[cfg(feature = "plain")]
pub mod plain;
pub mod progress;
pub mod resolved_color;
pub mod resolved_gradient;
pub mod resolved_shape;
pub mod retain;
pub mod rotation;
pub mod scale;
pub mod scroll;
pub mod secure;
#[cfg(feature = "secure_field")]
pub mod secure_field;
pub mod shadow;
#[cfg(feature = "slider")]
pub mod slider;
pub mod spacer;
#[cfg(feature = "stepper")]
pub mod stepper;
#[cfg(feature = "table")]
pub mod table;
pub mod text;
#[cfg(feature = "text_field")]
pub mod text_field;
#[cfg(feature = "toggle")]
pub mod toggle;
// Unconditional — like `empty`: `Metadata<TransitionSpec>` can be produced by
// any view tree (`.transition(...)` lives on `View`), so no feature gates it.
pub mod transition;
#[cfg(any(feature = "video", feature = "video_player"))]
pub mod video;
pub mod view_renderer;
#[cfg(feature = "webview")]
pub mod webview;
pub mod with_env;
