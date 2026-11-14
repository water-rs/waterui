//! The `UIKit` color well: a `UIColorWell` wrapped for the kit's action
//! model.
//!
//! # Safety
//!
//! The `unsafe` here reads the well's `UIColor` HDR accessors, which the
//! bindings mark unsafe — `UIKit` APIs are main-thread only, which the
//! `MainThreadOnly` thread kinds and [`MainThreadMarker`] constructor
//! guarantee.

use std::fmt;

use objc2::MainThreadMarker;
use objc2::rc::{Retained, Weak};
use objc2_ui_kit::{UIColor, UIColorWell, UIView};

use crate::action::ControlEvents;
use crate::geometry::Size;
use crate::{ActionTarget, color::Rgba};

/// A `UIColorWell`: a swatch that presents the color picker and reports the
/// chosen color.
pub struct ColorWell {
    well: Retained<UIColorWell>,
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
        Self {
            well: UIColorWell::new(mtm),
        }
    }

    /// The view a container adds and frames.
    #[must_use]
    pub fn view(&self) -> &UIView {
        &self.well
    }

    /// Whether the picker offers the opacity slider.
    pub fn set_supports_alpha(&self, supports_alpha: bool) {
        self.well.setSupportsAlpha(supports_alpha);
    }

    /// The color the well shows; `None` until one has been set.
    #[must_use]
    pub fn color(&self) -> Option<Retained<UIColor>> {
        self.well.selectedColor()
    }

    /// Shows `color` on the well.
    pub fn set_color(&self, color: &UIColor) {
        self.well.setSelectedColor(Some(color));
    }

    /// Whether the well responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.well.setEnabled(enabled);
    }

    /// Calls `handler` each time the user picks a new color. The returned
    /// target must be kept for as long as the well should respond; dropping
    /// it detaches the target.
    pub fn install_action(&self, handler: impl Fn() + 'static) -> ActionTarget {
        let this: Weak<UIColorWell> = Weak::new(&self.well);
        ActionTarget::new(&self.well, ControlEvents::VALUE_CHANGED, move |_mtm| {
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

/// `color` split into the SDR base it was exposed from and the HDR headroom
/// it carries — `linearExposure − 1`, `0` for ordinary colors.
#[must_use]
pub fn sdr_base_and_headroom(color: &UIColor) -> (Retained<UIColor>, f64) {
    // SAFETY: `linearExposure` is a documented `UIColor` accessor.
    let exposure = unsafe { color.linearExposure() };
    if exposure > 1.0 {
        // SAFETY: `standardDynamicRangeColor` is a documented `UIColor`
        // accessor.
        (unsafe { color.standardDynamicRangeColor() }, exposure - 1.0)
    } else {
        (color.into(), 0.0)
    }
}

/// `color` read as sRGB components through its `getRed:green:blue:alpha:` —
/// the fallback for colors the extended-linear conversion declines.
#[must_use]
pub fn srgb_components(color: &UIColor) -> Option<Rgba> {
    // SAFETY: the out pointers point at locals that outlive the call.
    unsafe {
        let mut red = 0.0;
        let mut green = 0.0;
        let mut blue = 0.0;
        let mut alpha = 0.0;
        color
            .getRed_green_blue_alpha(&raw mut red, &raw mut green, &raw mut blue, &raw mut alpha)
            .then(|| Rgba::new(red, green, blue, alpha))
    }
}
