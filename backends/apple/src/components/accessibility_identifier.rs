//! The `accessibility_identifier` metadata:
//! `IgnorableMetadata<AccessibilityIdentifier>` wrapped around a child.
//!
//! Mirrors `WuiAccessibilityIdentifier`: the stable automation identifier
//! lands on the content's platform view — `accessibilityIdentifier` on
//! `UIKit`, `setAccessibilityIdentifier` on `AppKit`. Invisible to users,
//! never spoken; it exists for XCUITest-style lookups only. The leaf is
//! the child's own, so nested metadata wrappers reach the same view the
//! Swift `accessibilityTarget` chain did.

use cocoa_ui::PlatformView;
use cocoa_ui::accessibility as a11y;
use cocoa_ui::view;
use objc2_foundation::NSString;
use waterui::accessibility::AccessibilityIdentifier;
use waterui_core::IgnorableMetadata;

use crate::dispatch::Dispatcher;

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

/// Installs the `accessibility_identifier` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<IgnorableMetadata<AccessibilityIdentifier>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        set_identifier(&target, &NSString::from_str(metadata.value.as_str()));
        leaf.keep(target);
        leaf
    });
}
