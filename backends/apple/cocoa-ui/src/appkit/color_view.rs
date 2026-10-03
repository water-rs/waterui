//! The `AppKit` color view: a layer-backed `NSView` painting one fill color.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSView` subclass, forwards to `NSView`'s own
//! implementations of the methods it overrides, and calls `objc2` bindings
//! marked unsafe because `AppKit` view APIs are main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee.

use std::fmt;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSColor, NSView};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;

/// A [`ColorView`] carries no state beyond its layer's background color.
pub struct ColorViewIvars {}

impl fmt::Debug for ColorViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorViewIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `ColorView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiColorView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ColorViewIvars]
    #[derive(Debug)]
    /// A layer-backed `NSView` whose whole content is a background color.
    ///
    /// Its coordinates are flipped: the origin is the top-left corner and `y`
    /// grows downward, matching the rest of the kit.
    pub struct ColorView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for ColorView {}

    impl ColorView {
        // SAFETY: see the module safety note.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }
    }
);

impl ColorView {
    /// A color fill view with no color set yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ColorViewIvars {});
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        view.setWantsLayer(true);
        view
    }

    /// Fills the view with `color`; `None` clears the fill.
    ///
    /// The color is pushed to the backing layer's `backgroundColor` — a plain
    /// `NSView` redraw needs no `drawRect:` pass.
    pub fn set_color(&self, color: Option<&NSColor>) {
        let cg = color.map(NSColor::CGColor);
        if let Some(layer) = self.layer() {
            layer.setBackgroundColor(cg.as_deref());
        }
    }
}
