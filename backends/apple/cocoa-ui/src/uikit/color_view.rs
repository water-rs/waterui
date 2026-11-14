//! The `UIKit` color view: a `UIView` painting one fill color.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass and calls `objc2` bindings
//! marked unsafe because `UIKit` view APIs are main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee.

use std::fmt;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{UIColor, UIView};

use crate::callback::guarded;
use crate::response::{ReportsResponse, ResponseGeometry};

/// A [`ColorView`] carries its leaf geometry beyond its background color.
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
    // SAFETY: `UIView`'s designated initializer is `initWithFrame:`, which
    // `ColorView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UIView))]
    #[name = "CocoaUiColorView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ColorViewIvars]
    #[derive(Debug)]
    /// A `UIView` whose whole content is a background color.
    pub struct ColorView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for ColorView {}

    impl ColorView {
        // SAFETY: see the module safety note. A color view declares itself
        // a fill to the sibling backend's safe-area extension — its whole
        // painted surface is the fill.
        #[unsafe(method(cocoaUiIsFill))]
        fn is_fill_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("ColorView layoutSubviews", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
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
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] }
    }

    /// Fills the view with `color`; `None` clears the fill.
    pub fn set_color(&self, color: Option<&UIColor>) {
        self.setBackgroundColor(color);
    }
}
