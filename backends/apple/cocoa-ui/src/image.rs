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
