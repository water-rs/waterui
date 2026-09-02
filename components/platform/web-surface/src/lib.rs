//! The accessibility contract every `WaterUI` web-surface realization publishes.
//!
//! A web page is opaque to the host accessibility tree — an engine draws it into
//! a texture or a subview nothing can see into — so the realization itself has to
//! publish a node or the whole region is missing from a screen reader. Both the
//! standard `WebView` and the full Chromium component owe the tree the same node,
//! and neither depends on the other, so the contract lives here instead of in
//! either component.

use waterui_core::accessibility::AccessibilityRole;
use waterui_core::{AnyView, Environment, IgnorableMetadata, View};

/// Wraps a realization of web page content in the accessibility node it owes
/// the tree.
///
/// The page is opaque to the host accessibility tree — an engine draws it into
/// a texture or a subview nothing can see into — so the surface itself has to
/// appear or the whole region is missing from a screen reader. Every engine
/// realization publishes the same node, and whatever the application already
/// said with `.a11y_role(...)` wins over this default.
///
/// The label is deliberately not defaulted: only the application knows what the
/// page is, and a made-up one reads worse than none.
pub fn web_surface_semantics(env: &Environment, view: impl View) -> AnyView {
    if env.get::<AccessibilityRole>().is_some() {
        AnyView::new(view)
    } else {
        AnyView::new(IgnorableMetadata::new(view, AccessibilityRole::Group))
    }
}
