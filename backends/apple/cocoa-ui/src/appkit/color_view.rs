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
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSColor, NSView};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;

use crate::callback::guarded;
use crate::response::{ReportsResponse, ResponseGeometry};

/// A [`ColorView`] carries its leaf geometry beyond its layer's background
/// color.
pub struct ColorViewIvars {
    /// How the view was placed as a leaf, and its response rectangle.
    geometry: ResponseGeometry,
}

impl fmt::Debug for ColorViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorViewIvars")
            .field("geometry", &self.geometry)
            .finish()
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

        // SAFETY: see the module safety note. A color view declares itself
        // a fill to the sibling backend's safe-area extension — its whole
        // painted surface is the fill.
        #[unsafe(method(cocoaUiIsFill))]
        fn is_fill_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            guarded("ColorView layout", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layout] };
                self.ivars().geometry.commit(self);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame_override(&self) -> CGRect {
            guarded("ColorView accessibilityFrame", || {
                self.ivars()
                    .geometry
                    .accessibility_frame(self)
                    // SAFETY: see the module safety note.
                    .unwrap_or_else(|| unsafe { msg_send![super(self), accessibilityFrame] })
            })
        }
    }
);

impl ReportsResponse for ColorView {
    fn response_geometry(&self) -> &ResponseGeometry {
        &self.ivars().geometry
    }
}

impl ColorView {
    /// A color fill view with no color set yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ColorViewIvars {
            geometry: ResponseGeometry::default(),
        });
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
