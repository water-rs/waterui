//! The system's preferred text styles and their metrics.
//!
//! `preferred_font` answers the font the platform would use for a text style
//! at the current content-size category, as `UIFont`/`NSFont` report it.
//!
//! # Safety
//!
//! The `unsafe` here reads font descriptors and their trait dictionaries —
//! documented accessors, on the calling thread. The traits attribute is an
//! `NSDictionary` and the weight trait inside it an `NSNumber`, which are
//! the types `UIFontDescriptor`/`NSFontDescriptor` document for them.

/// One of the platform's named text styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextStyle {
    /// The default body text.
    Body,
    /// The largest title (`.title1`, a first-level heading).
    Title1,
    /// A headline.
    Headline,
    /// A subheadline.
    Subheadline,
    /// The smaller caption style (`.caption1`).
    Caption1,
    /// A footnote.
    Footnote,
}

/// The metrics a preferred font reports for a style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    /// The point size.
    pub size: f64,
    /// The weight on the `UIFont.Weight`/`NSFontWeight` scale: light `-0.4`,
    /// regular `0.0`, bold `0.4`, black `0.62`.
    pub weight: f64,
    /// The face's declared line pitch — line box plus leading.
    pub line_height: f64,
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSFont, NSFontDescriptor, NSFontTextStyle};
    use objc2_foundation::{NSDictionary, NSNumber};

    use super::{FontMetrics, TextStyle, weight_or_regular};

    fn native_style(style: TextStyle) -> &'static NSFontTextStyle {
        // SAFETY: these are the platform's own text-style constants, alive
        // for the process.
        unsafe {
            match style {
                TextStyle::Body => objc2_app_kit::NSFontTextStyleBody,
                TextStyle::Title1 => objc2_app_kit::NSFontTextStyleTitle1,
                TextStyle::Headline => objc2_app_kit::NSFontTextStyleHeadline,
                TextStyle::Subheadline => objc2_app_kit::NSFontTextStyleSubheadline,
                TextStyle::Caption1 => objc2_app_kit::NSFontTextStyleCaption1,
                TextStyle::Footnote => objc2_app_kit::NSFontTextStyleFootnote,
            }
        }
    }

    pub fn preferred_font(style: TextStyle) -> FontMetrics {
        let options = NSDictionary::new();
        // SAFETY: `options` is an empty dictionary of the declared type, and
        // the style constants are `NSFontTextStyle` values.
        let font =
            unsafe { NSFont::preferredFontForTextStyle_options(native_style(style), &options) };
        let descriptor = font.fontDescriptor();
        let weight = weight_or_regular(traits_weight(&descriptor));
        FontMetrics {
            size: font.pointSize(),
            weight,
            line_height: font.ascender() - font.descender() + font.leading(),
        }
    }

    /// The `NSFontWeightTrait` number the descriptor's traits dictionary
    /// declares, or `None` when the descriptor does not say.
    fn traits_weight(descriptor: &NSFontDescriptor) -> Option<f64> {
        // SAFETY: the traits attribute key is a platform constant.
        let traits = unsafe { descriptor.objectForKey(objc2_app_kit::NSFontTraitsAttribute) }?;
        // SAFETY: the traits attribute is an `NSDictionary` by contract; the
        // weight trait inside it an `NSNumber`.
        unsafe {
            let traits = traits.downcast_ref::<NSDictionary>().expect("font traits");
            let value: Option<Retained<AnyObject>> =
                msg_send![traits, objectForKey: objc2_app_kit::NSFontWeightTrait];
            value.map(|value| {
                value
                    .downcast_ref::<NSNumber>()
                    .expect("font weight")
                    .doubleValue()
            })
        }
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber};
    use objc2_ui_kit::{UIFont, UIFontDescriptor, UIFontTextStyle};

    use super::{FontMetrics, TextStyle, weight_or_regular};

    fn native_style(style: TextStyle) -> &'static UIFontTextStyle {
        // SAFETY: these are the platform's own text-style constants, alive
        // for the process.
        unsafe {
            match style {
                TextStyle::Body => objc2_ui_kit::UIFontTextStyleBody,
                TextStyle::Title1 => objc2_ui_kit::UIFontTextStyleTitle1,
                TextStyle::Headline => objc2_ui_kit::UIFontTextStyleHeadline,
                TextStyle::Subheadline => objc2_ui_kit::UIFontTextStyleSubheadline,
                TextStyle::Caption1 => objc2_ui_kit::UIFontTextStyleCaption1,
                TextStyle::Footnote => objc2_ui_kit::UIFontTextStyleFootnote,
            }
        }
    }

    pub fn preferred_font(style: TextStyle) -> FontMetrics {
        let font = UIFont::preferredFontForTextStyle(native_style(style));
        // SAFETY: `fontDescriptor` is a documented accessor on a live font.
        let descriptor = unsafe { font.fontDescriptor() };
        let weight = weight_or_regular(traits_weight(&descriptor));
        // SAFETY: `ascender`, `descender`, `leading` and `pointSize` are
        // documented property accessors.
        unsafe {
            FontMetrics {
                size: font.pointSize(),
                weight,
                line_height: font.ascender() - font.descender() + font.leading(),
            }
        }
    }

    /// The `UIFontWeightTrait` number the descriptor's traits dictionary
    /// declares, or `None` when the descriptor does not say.
    fn traits_weight(descriptor: &UIFontDescriptor) -> Option<f64> {
        // SAFETY: the traits attribute key is a platform constant.
        let traits =
            unsafe { descriptor.objectForKey(objc2_ui_kit::UIFontDescriptorTraitsAttribute) }?;
        // SAFETY: the traits attribute is an `NSDictionary` by contract; the
        // weight trait inside it an `NSNumber`.
        unsafe {
            let traits = traits.downcast_ref::<NSDictionary>().expect("font traits");
            let value: Option<Retained<AnyObject>> =
                msg_send![traits, objectForKey: objc2_ui_kit::UIFontWeightTrait];
            value.map(|value| {
                value
                    .downcast_ref::<NSNumber>()
                    .expect("font weight")
                    .doubleValue()
            })
        }
    }
}

/// The metrics of the platform's preferred font for `style`.
///
/// Content-size-category changes are reported through the platform's
/// appearance observation channels; call this again when they fire to read
/// the new metrics.
#[must_use]
pub fn preferred_font(style: TextStyle) -> FontMetrics {
    imp::preferred_font(style)
}

/// The weight a preferred font's descriptor resolves to: the declared weight
/// trait, or `0.0` — regular on the `UIFont.Weight`/`NSFontWeight` scale —
/// when the descriptor omits the traits dictionary or the trait itself. A
/// preferred font without a weight trait is the regular face.
fn weight_or_regular(trait_value: Option<f64>) -> f64 {
    trait_value.unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::weight_or_regular;

    /// A descriptor without a weight trait — or without a traits dictionary
    /// at all — is the regular face: `0.0`, not the bold `0.4`.
    #[test]
    fn missing_weight_trait_is_regular() {
        assert_eq!(weight_or_regular(None), 0.0);
    }

    /// A declared weight trait passes through unchanged, including the
    /// float32-rounded constants `AppKit`/`UIKit` publish.
    #[test]
    fn explicit_weight_trait_passes_through() {
        assert_eq!(weight_or_regular(Some(0.0)), 0.0);
        assert_eq!(
            weight_or_regular(Some(0.400_000_005_960_464_5)),
            0.400_000_005_960_464_5
        );
        assert_eq!(
            weight_or_regular(Some(-0.800_000_011_920_929)),
            -0.800_000_011_920_929
        );
    }
}
