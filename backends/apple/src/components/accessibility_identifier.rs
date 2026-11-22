//! The `accessibility_identifier` metadata:
//! `Metadata<AccessibilityIdentifier>` wrapped around a child.
//!
//! Mirrors `WuiAccessibilityIdentifier`: the stable automation identifier
//! lands on the content's platform view — `accessibilityIdentifier` on
//! `UIKit`, `setAccessibilityIdentifier` on `AppKit`. Invisible to users,
//! never spoken; it exists for XCUITest-style lookups only. The leaf is
//! the child's own, so nested metadata wrappers reach the same view the
//! Swift `accessibilityTarget` chain did.

use crate::contract::{NativeLeaf, RenderContext};
use cocoa_ui::PlatformView;
use cocoa_ui::accessibility as a11y;
use cocoa_ui::view;
use objc2_foundation::NSString;
use waterui::accessibility::AccessibilityIdentifier;
use waterui_core::Metadata;

#[cfg(target_os = "macos")]
fn set_identifier(target: &PlatformView, identifier: &NSString) {
    let element: &objc2::runtime::ProtocolObject<dyn cocoa_ui::objc2_app_kit::NSAccessibility> =
        objc2::runtime::ProtocolObject::from_ref(target);
    a11y::set_identifier(element, identifier);
}

#[cfg(target_os = "ios")]
fn set_identifier(target: &PlatformView, identifier: &NSString) {
    a11y::set_identifier(target, identifier);
}

/// The `accessibility_identifier` realization.
pub fn realize_accessibility_identifier_metadata(
    metadata: Metadata<AccessibilityIdentifier>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mut leaf = ctx.render(metadata.content);
    let target = view::retain_base(leaf.view());
    set_identifier(&target, &NSString::from_str(metadata.value.as_str()));
    leaf.keep(target);
    leaf
}
