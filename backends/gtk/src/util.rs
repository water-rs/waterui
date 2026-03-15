//! Shared utilities for the GTK backend.

use glib::object::ObjectExt;
use nami::watcher::BoxWatcherGuard;
use waterui::accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityLabel, AccessibilityRole,
    AccessibilityState, AccessibilityStateSignal,
};
use waterui::background::{Background, MaterialBackground};
use waterui::border::Border;
use waterui::component::focus::Focused;
use waterui::cursor::Cursor;
use waterui::drag_drop::{Draggable, DropDestination};
use waterui::filter::Opacity;
use waterui::gesture::GestureObserver;
use waterui::interaction::Hittable;
use waterui::metadata::context_menu::ContextMenu;
use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
use waterui::style::{Offset, Rotation, Scale, Shadow};
use waterui_core::event::{LifeCycleHook, OnEvent};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, IgnorableMetadata, Metadata, Retain};
use waterui_graphics::AppliedFilter;
use waterui_graphics::color::ResolvedColor;
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_shape::ClipShape;

/// Stores a watcher guard on a widget to prevent it from being dropped.
///
/// The guard is stored as widget data with a unique key, ensuring the reactive
/// subscription stays alive as long as the widget exists.
pub fn store_watcher_guard(widget: &impl ObjectExt, guard: BoxWatcherGuard) {
    // `set_data` takes ownership and will drop the value when the widget is destroyed
    // (or when overwritten by another `set_data` call using the same key).
    unsafe { widget.set_data("waterui_watcher_guard", guard) }
}

/// Stores multiple watcher guards on a widget.
///
/// Use this when a component has multiple reactive subscriptions that need
/// to be kept alive with the widget.
pub fn store_watcher_guards(widget: &impl ObjectExt, guards: Vec<BoxWatcherGuard>) {
    unsafe { widget.set_data("waterui_watcher_guards", guards) }
}

/// Converts a resolved color to clamped sRGBA byte channels.
#[must_use]
pub fn resolved_color_to_rgba8(color: ResolvedColor) -> (u8, u8, u8, f32) {
    let srgb = color.to_srgb_with_headroom();
    let red = (srgb.red.clamp(0.0, 1.0) * 255.0) as u8;
    let green = (srgb.green.clamp(0.0, 1.0) * 255.0) as u8;
    let blue = (srgb.blue.clamp(0.0, 1.0) * 255.0) as u8;
    let alpha = color.opacity.clamp(0.0, 1.0);
    (red, green, blue, alpha)
}

/// Converts a resolved color to clamped SDR sRGBA float channels in `[0.0, 1.0]`.
#[must_use]
pub fn resolved_color_to_srgba_f64(color: ResolvedColor) -> (f64, f64, f64, f64) {
    let srgb = color.to_srgb_with_headroom();
    assert!(
        srgb.red.is_finite(),
        "resolved color red channel must be finite"
    );
    assert!(
        srgb.green.is_finite(),
        "resolved color green channel must be finite"
    );
    assert!(
        srgb.blue.is_finite(),
        "resolved color blue channel must be finite"
    );
    assert!(
        color.opacity.is_finite(),
        "resolved color opacity channel must be finite"
    );
    (
        f64::from(srgb.red.clamp(0.0, 1.0)),
        f64::from(srgb.green.clamp(0.0, 1.0)),
        f64::from(srgb.blue.clamp(0.0, 1.0)),
        f64::from(color.opacity.clamp(0.0, 1.0)),
    )
}

/// Converts a resolved color to `#RRGGBB` format.
#[must_use]
pub fn resolved_color_to_hex(color: ResolvedColor) -> String {
    let (red, green, blue, _) = resolved_color_to_rgba8(color);
    format!("#{red:02X}{green:02X}{blue:02X}")
}

/// Converts a resolved color to CSS `rgba(r, g, b, a)` format.
#[must_use]
pub fn resolved_color_to_css_rgba(color: ResolvedColor) -> String {
    let (red, green, blue, alpha) = resolved_color_to_rgba8(color);
    format!("rgba({red}, {green}, {blue}, {alpha})")
}

fn passthrough_content(view: &AnyView) -> Option<&AnyView> {
    macro_rules! passthrough_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<Metadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    macro_rules! passthrough_ignorable_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<IgnorableMetadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    passthrough_metadata_content!(
        Environment,
        Retain,
        Opacity,
        AppliedFilter,
        Scale,
        Rotation,
        Offset,
        ClipShape,
        Border,
        Shadow,
        Focused,
        Hittable,
        GestureObserver,
        LifeCycleHook,
        OnEvent,
        Secure,
        StandardDynamicRange,
        HighDynamicRange,
        Cursor,
        IgnoreSafeArea,
        ContextMenu,
        Draggable,
        DropDestination,
        Background
    );
    passthrough_ignorable_metadata_content!(
        MaterialBackground,
        AccessibilityLabel,
        AccessibilityRole,
        AccessibilityHidden,
        AccessibilityChildren,
        AccessibilityState,
        AccessibilityStateSignal
    );

    None
}

/// Returns the stretch axis for layout, recursively unwrapping metadata wrappers.
#[must_use]
pub fn effective_stretch_axis(view: &AnyView) -> StretchAxis {
    if let Some(content) = passthrough_content(view) {
        return effective_stretch_axis(content);
    }
    view.stretch_axis()
}
