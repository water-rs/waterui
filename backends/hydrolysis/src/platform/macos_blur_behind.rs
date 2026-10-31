//! The `AppKit` blur behind a window whose background is a behind-window
//! material: an `NSVisualEffectView` blending behind the window, inside the
//! view the engine presents into and beneath the engine's own views.
//!
//! The effect view lives in the window's content view — the view winit
//! hands out — never in the content view's superview. That superview is the
//! window's frame view, which lays out the title bar from subviews it owns;
//! a view added there that it does not know breaks the title bar's layout.
//!
//! The engine's views are subviews of the same content view, and their
//! stack begins above every subview present when the engine attaches, so
//! the effect view goes in at the very bottom: beneath the engine whether
//! it attaches before or after the engine does.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly as _};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSResponder, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
};
use objc2_foundation::{NSObject, NSPoint};

// An effect view that never answers a hit: every click over it reaches the
// content view, as it does over the engine's own views. An effect view that
// took the hit would be asked about the event in the content view's place —
// `acceptsFirstMouse:` among them, which a plain view declines.
objc2::define_class!(
    // SAFETY: `NSVisualEffectView` has no subclassing requirements for a
    // view that only declines hits; the class adds no ivars and implements
    // no Drop.
    #[unsafe(super(NSVisualEffectView, NSView, NSResponder, NSObject))]
    #[name = "HydrolysisBlurBehindView"]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ()]
    struct BlurBehindView;

    impl BlurBehindView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

/// Installs the blur behind `content_view`'s window at the bottom of
/// `content_view`, filling it and following its size. Removing the
/// returned view from its superview removes the blur.
/// `underWindowBackground` is the material for both behind-window levels —
/// Hydrolysis's own tint, cleared to under the content, supplies the
/// level's colour, so the effect view only needs to blur.
pub(super) fn install(
    content_view: &NSView,
    mtm: MainThreadMarker,
) -> Retained<NSVisualEffectView> {
    let this = BlurBehindView::alloc(mtm).set_ivars(());
    // SAFETY: `init` is `NSView`'s designated initializer, inherited by a
    // `define_class!` type that declares none of its own.
    let view: Retained<BlurBehindView> = unsafe { objc2::msg_send![super(this), init] };
    let effect_view = Retained::into_super(view);
    effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect_view.setState(NSVisualEffectState::Active);
    effect_view.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
    effect_view.setFrame(content_view.bounds());
    effect_view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    content_view.addSubview_positioned_relativeTo(&effect_view, NSWindowOrderingMode::Below, None);
    effect_view
}
