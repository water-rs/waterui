//! The platform text face and the ways to construct one.
//!
//! `Font` names `NSFont` on macOS and `UIFont` on iOS; weight values are the
//! framework's own scale, where [`weight::REGULAR`] is the system default.
//!
//! # Safety
//!
//! Every `unsafe` here is an `objc2` signature that marks a call unsafe only
//! because the framework considers it non-`Send`/`Sync` or because it returns
//! a nullable pointer the contract says is never null. All calls run on the
//! main thread through the caller's [`MainThreadMarker`].

use objc2::{MainThreadMarker, rc::Retained};
#[cfg(target_os = "macos")]
use objc2_app_kit::NSFont;
use objc2_foundation::NSString;
#[cfg(target_os = "ios")]
use objc2_ui_kit::UIFont;

/// The platform font object: `NSFont` on macOS, `UIFont` on iOS.
#[cfg(target_os = "ios")]
pub type Font = UIFont;
/// The platform font object: `NSFont` on macOS, `UIFont` on iOS.
#[cfg(target_os = "macos")]
pub type Font = NSFont;

/// The platform weight scale, shared by both font classes.
pub mod weight {
    /// 100.
    pub const THIN: f64 = -0.6;
    /// 200.
    pub const ULTRA_LIGHT: f64 = -0.8;
    /// 300.
    pub const LIGHT: f64 = -0.4;
    /// 400, the default.
    pub const REGULAR: f64 = 0.0;
    /// 500.
    pub const MEDIUM: f64 = 0.23;
    /// 600.
    pub const SEMI_BOLD: f64 = 0.3;
    /// 700.
    pub const BOLD: f64 = 0.4;
    /// 800.
    pub const HEAVY: f64 = 0.56;
    /// 900.
    pub const BLACK: f64 = 0.62;
}

/// The font size the platform uses for label text when the caller does not
/// set one.
#[must_use]
pub fn system_size() -> f64 {
    Font::systemFontSize()
}

/// The system face at `size` and `weight`, both on the platform's own scale.
///
/// A non-positive `size` resolves to [`system_size`].
#[must_use]
pub fn system(_mtm: MainThreadMarker, size: f64, weight: f64) -> Retained<Font> {
    let size = if size <= 0.0 { system_size() } else { size };
    Font::systemFontOfSize_weight(size, weight)
}

/// The monospaced system face at `size` and `weight`.
///
/// A non-positive `size` resolves to [`system_size`].
#[must_use]
pub fn monospaced(_mtm: MainThreadMarker, size: f64, weight: f64) -> Retained<Font> {
    let size = if size <= 0.0 { system_size() } else { size };
    Font::monospacedSystemFontOfSize_weight(size, weight)
}

/// The monospaced-digit system face at `size` and `weight` — tabular
/// numerals for readouts whose digits must not drift.
///
/// A non-positive `size` resolves to [`system_size`].
#[must_use]
pub fn monospaced_digit(_mtm: MainThreadMarker, size: f64, weight: f64) -> Retained<Font> {
    let size = if size <= 0.0 { system_size() } else { size };
    #[cfg(target_os = "ios")]
    {
        UIFont::monospacedDigitSystemFontOfSize_weight(size, weight)
    }
    #[cfg(target_os = "macos")]
    {
        NSFont::monospacedDigitSystemFontOfSize_weight(size, weight)
    }
}

/// A face installed under PostScript or family `name`, at `size` points.
///
/// `None` when the name does not resolve to a face this process can use.
#[must_use]
pub fn named(name: &str, size: f64) -> Option<Retained<Font>> {
    let name = NSString::from_str(name);
    Font::fontWithName_size(&name, size)
}

/// A variant of `font` that adds the italic trait to its descriptor, or
/// `None` when the face has no italic variant.
///
/// The returned font keeps the original's size.
#[must_use]
pub fn italic_variant(font: &Font) -> Option<Retained<Font>> {
    #[cfg(target_os = "ios")]
    {
        use objc2_ui_kit::UIFontDescriptorSymbolicTraits;
        // SAFETY: see the module safety note.
        let descriptor = unsafe { font.fontDescriptor() };
        // SAFETY: see the module safety note.
        let descriptor = unsafe {
            descriptor.fontDescriptorWithSymbolicTraits(
                descriptor.symbolicTraits() | UIFontDescriptorSymbolicTraits::TraitItalic,
            )
        }?;
        // SAFETY: see the module safety note.
        Some(Font::fontWithDescriptor_size(&descriptor, unsafe {
            font.pointSize()
        }))
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSFontDescriptorSymbolicTraits;
        let descriptor = font.fontDescriptor();
        let descriptor = descriptor.fontDescriptorWithSymbolicTraits(
            descriptor.symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic,
        );
        Font::fontWithDescriptor_size(&descriptor, font.pointSize())
    }
}

/// The distance from baseline to baseline the platform typesetter uses for
/// `font` when a paragraph style does not override it.
#[must_use]
pub fn typesetter_line_height(_mtm: MainThreadMarker, font: &Font) -> f64 {
    #[cfg(target_os = "ios")]
    {
        // SAFETY: see the module safety note.
        unsafe { font.lineHeight() }
    }
    #[cfg(target_os = "macos")]
    {
        // `NSFont` does not expose a leading distance; the layout manager's
        // default is what AppKit text fields lay out with.
        let layout_manager = objc2_app_kit::NSLayoutManager::new();
        layout_manager.defaultLineHeightForFont(font)
    }
}
