//! `NSVisualEffectView` — vibrancy/material backgrounds for chrome.

use crate::Retained;
use objc2::msg_send;
use objc2_app_kit::{NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectView};
use objc2_foundation::MainThreadMarker;

/// A material backdrop for in-content chrome — an `NSVisualEffectView`
/// configured the way a header bar is: `HeaderView` material blended
/// within the window.
#[must_use]
pub fn header_material_view(mtm: MainThreadMarker) -> Retained<NSVisualEffectView> {
    // SAFETY: `init` is `NSVisualEffectView`'s designated initializer.
    let view: Retained<NSVisualEffectView> =
        unsafe { msg_send![mtm.alloc::<NSVisualEffectView>(), init] };
    view.setMaterial(NSVisualEffectMaterial::HeaderView);
    view.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
    view
}

/// The tint underneath the material: the view's own layer takes the
/// color and the material blends over it.
pub fn set_material_background(view: &NSVisualEffectView, color: Option<&objc2_app_kit::NSColor>) {
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        // SAFETY: `layer` is a live `CALayer`; the `CGColor` outlives the
        // call — the layer retains its property value.
        unsafe {
            let _: () = msg_send![&*layer, setBackgroundColor: color.map(objc2_app_kit::NSColor::CGColor).as_deref()];
        }
    }
}
