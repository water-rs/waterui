//! The platform image object and how a view scales it.
//!
//! `Image` names `NSImage` on macOS and `UIImage` on iOS, the same way
//! [`crate::Font`] names the platform font. [`ScaleMode`] is the one spelling
//! for the two frameworks' content-scaling enums: `UIView.ContentMode` and
//! `NSImageScaling`.

#[cfg(target_os = "macos")]
use objc2_app_kit::NSImage;
#[cfg(target_os = "ios")]
use objc2_ui_kit::UIImage;

use waterui_contract::contract_violation;

use crate::geometry::Rect;

/// The platform image object: `NSImage` on macOS, `UIImage` on iOS.
#[cfg(target_os = "macos")]
pub type Image = NSImage;
/// The platform image object: `NSImage` on macOS, `UIImage` on iOS.
#[cfg(target_os = "ios")]
pub type Image = UIImage;

/// How an image fits the box its view lays out.
///
/// The platform spellings differ — `UIView.ContentMode.ScaleAspectFit` is
/// `NSImageScaling.ScaleProportionallyUpOrDown` — so the kit names the
/// intent once. `Fill` has no `NSImageScaling` equivalent: `AppKit` image
/// scaling never crops, so `Fill` draws like `Fit` on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ScaleMode {
    /// Scale each axis independently to the box (the image may distort).
    Stretch,
    /// Keep the aspect ratio, fitting the whole image inside the box.
    #[default]
    Fit,
    /// Keep the aspect ratio, covering the box and cropping the overflow.
    /// `AppKit` has no crop mode; this behaves like [`ScaleMode::Fit`] there.
    Fill,
}

/// Where an unscaled image sits along one axis of its view's bounds — the
/// positions the platforms' image gravities name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gravity {
    /// At the top or left edge.
    Start,
    /// Centred.
    Centre,
    /// At the bottom or right edge.
    End,
}

/// The gravities that put an image of `rect`'s size exactly at `rect`
/// inside `bounds`, both in layout space (y running down), horizontal
/// first.
///
/// # Panics
///
/// When `rect` sits at no start, centre or end position of `bounds` on an
/// axis: the platform image views draw an unscaled image only there.
pub(crate) fn gravities(bounds: Rect, rect: Rect) -> (Gravity, Gravity) {
    (
        gravity(rect.origin.x, rect.size.width, bounds.size.width),
        gravity(rect.origin.y, rect.size.height, bounds.size.height),
    )
}

/// The gravity that puts a span of `extent` at `start` inside a box of
/// `length` starting at zero.
fn gravity(start: f64, extent: f64, length: f64) -> Gravity {
    /// How far, in points, a placement may sit from a gravity position:
    /// layout geometry passes through `f32`.
    const TOLERANCE: f64 = 0.5;
    if (start + extent / 2.0 - length / 2.0).abs() < TOLERANCE {
        Gravity::Centre
    } else if start.abs() < TOLERANCE {
        Gravity::Start
    } else if (start + extent - length).abs() < TOLERANCE {
        Gravity::End
    } else {
        contract_violation!(
            "an image view draws its unscaled image at the start, centre or end of its bounds; \
             {extent} at {start} in {length} is none of them; the image is drawn at the start"
        );
        Gravity::Start
    }
}
