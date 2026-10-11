//! The `AppKit` color well: an `NSColorWell` wrapped for the kit's action
//! model.
//!
//! # Safety
//!
//! The `unsafe` here allocates the well through `objc2` — `AppKit` APIs are
//! main-thread only, which the `MainThreadOnly` thread kinds and
//! [`MainThreadMarker`] constructor guarantee.

use std::fmt;

use objc2::rc::{Retained, Weak};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{NSColorPanel, NSColorWell, NSView};
use objc2_core_foundation::CGRect;

use objc2_app_kit::NSColor;

use crate::ActionTarget;
use crate::geometry::Size;

/// An `NSColorWell`: a swatch that opens the shared color panel and reports
/// the chosen color.
pub struct ColorWell {
    well: Retained<NSColorWell>,
}

impl fmt::Debug for ColorWell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorWell").finish_non_exhaustive()
    }
}

impl ColorWell {
    /// A color well showing the standard control color.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let well = NSColorWell::alloc(mtm);
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let well: Retained<NSColorWell> = unsafe { msg_send![well, initWithFrame: CGRect::ZERO] };
        Self { well }
    }

    /// The view a container adds and frames.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.well
    }

    /// Whether the color panel offers the opacity slider.
    ///
    /// Also turns on `showsAlpha` on the shared `NSColorPanel`, as the
    /// system panel reads it at activation time.
    pub fn set_supports_alpha(&self, supports_alpha: bool, mtm: MainThreadMarker) {
        self.well.setSupportsAlpha(supports_alpha);
        if supports_alpha {
            NSColorPanel::sharedColorPanel(mtm).setShowsAlpha(true);
        }
    }

    /// The color the well shows.
    #[must_use]
    pub fn color(&self) -> Retained<NSColor> {
        self.well.color()
    }

    /// Shows `color` on the well.
    pub fn set_color(&self, color: &NSColor) {
        self.well.setColor(color);
    }

    /// Whether the well responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.well.setEnabled(enabled);
    }

    /// Calls `handler` each time the user picks a new color. The returned
    /// target must be kept for as long as the well should respond; dropping
    /// it detaches the target.
    pub fn install_action(&self, handler: impl Fn() + 'static) -> ActionTarget {
        let this: Weak<NSColorWell> = Weak::new(&self.well);
        ActionTarget::new(&self.well, move |_mtm| {
            if this.load().is_some() {
                handler();
            }
        })
    }

    /// The size a measure pass reports.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.well.intrinsicContentSize();
        Size::new(size.width, size.height)
    }
}
