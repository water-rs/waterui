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

use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSControlSize, NSFont};
use objc2_foundation::NSString;
#[cfg(target_os = "ios")]
use objc2_ui_kit::UIFont;
use waterui_contract::contract_violation;

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

/// The font size `AppKit` pairs with a control size class: 11 pt for
/// small, 9 pt for mini, the regular system size for regular and large.
/// `UIKit` has no per-control-size font.
#[cfg(target_os = "macos")]
#[must_use]
pub fn system_size_for_control_size(size: NSControlSize) -> f64 {
    NSFont::systemFontSizeForControlSize(size)
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

/// A symbolic trait a font descriptor can add: the platform resolves the
/// face that carries it.
#[derive(Clone, Copy)]
enum SymbolicTrait {
    Italic,
    /// The platform picks the bold face per font: a text-style font gets
    /// its style's bold counterpart, Semibold for body text, and a regular
    /// fixed-size system font gets Bold.
    Bold,
}

/// A variant of `font` that adds the italic trait to its descriptor, or
/// `None` when the face has no italic variant.
///
/// The returned font keeps the original's size.
#[must_use]
pub fn italic_variant(font: &Font) -> Option<Retained<Font>> {
    trait_variant(font, SymbolicTrait::Italic)
}

/// A variant of `font` that adds the bold symbolic trait to its descriptor,
/// or `None` when the face has no bold variant.
///
/// The returned font keeps the original's size.
#[must_use]
pub fn bold_variant(font: &Font) -> Option<Retained<Font>> {
    trait_variant(font, SymbolicTrait::Bold)
}

/// A variant of `font` whose descriptor adds `symbolic`, at the original's
/// size, or `None` when the platform resolves no such face.
fn trait_variant(font: &Font, symbolic: SymbolicTrait) -> Option<Retained<Font>> {
    #[cfg(target_os = "ios")]
    {
        use objc2_ui_kit::UIFontDescriptorSymbolicTraits;
        let added = match symbolic {
            SymbolicTrait::Italic => UIFontDescriptorSymbolicTraits::TraitItalic,
            SymbolicTrait::Bold => UIFontDescriptorSymbolicTraits::TraitBold,
        };
        // SAFETY: see the module safety note.
        let descriptor = unsafe { font.fontDescriptor() };
        // SAFETY: see the module safety note.
        let descriptor = unsafe {
            descriptor.fontDescriptorWithSymbolicTraits(descriptor.symbolicTraits() | added)
        }?;
        // SAFETY: see the module safety note.
        Some(Font::fontWithDescriptor_size(&descriptor, unsafe {
            font.pointSize()
        }))
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSFontDescriptorSymbolicTraits;
        let added = match symbolic {
            SymbolicTrait::Italic => NSFontDescriptorSymbolicTraits::TraitItalic,
            SymbolicTrait::Bold => NSFontDescriptorSymbolicTraits::TraitBold,
        };
        let descriptor = font.fontDescriptor();
        let descriptor =
            descriptor.fontDescriptorWithSymbolicTraits(descriptor.symbolicTraits() | added);
        Font::fontWithDescriptor_size(&descriptor, font.pointSize())
    }
}

/// The weight `font` declares, on the platform's own scale.
#[must_use]
pub fn weight_of(font: &Font) -> f64 {
    crate::system_font::font_weight(font)
}

/// The weight the system face `font` draws at, on the platform's own scale.
///
/// Unlike [`weight_of`], this reads the face rather than its descriptor: a
/// variant built by adding the bold symbolic trait keeps the weight trait of
/// the descriptor it came from, so the bold of the macOS headline style
/// declares Bold while drawing the Heavy face.
///
/// Reports as [`weight_coordinate`] does.
#[must_use]
pub fn drawn_weight(font: &Font) -> f64 {
    // SAFETY: see the module safety note.
    let size = unsafe { point_size(font) };
    weight_at_coordinate(size, weight_coordinate(font))
}

/// The platform weight whose system face at `size` draws at the `wght`
/// design coordinate nearest `coordinate`.
///
/// # Panics
///
/// Never in practice: the named weights are a fixed non-empty table.
#[must_use]
pub fn weight_at_coordinate(size: f64, coordinate: f64) -> f64 {
    const NAMED: [f64; 9] = [
        weight::ULTRA_LIGHT,
        weight::THIN,
        weight::LIGHT,
        weight::REGULAR,
        weight::MEDIUM,
        weight::SEMI_BOLD,
        weight::BOLD,
        weight::HEAVY,
        weight::BLACK,
    ];
    NAMED
        .map(|named| {
            let face = Font::systemFontOfSize_weight(size, named);
            (named, (weight_coordinate(&face) - coordinate).abs())
        })
        .into_iter()
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(named, _)| named)
        .expect("`NAMED` holds nine weights, so the minimum exists")
}

/// The coordinate `font` takes on the `OpenType` `wght` axis: its configured
/// variation, or the axis default when it leaves the axis there — `CoreText`
/// reports only non-default variation values.
///
/// A face with no `wght` axis breaks the contract: the system face is
/// variable. It is reported and the axis's default, `400` (regular), is
/// answered.
///
/// # Panics
///
/// Never in practice: `CoreText` documents variation axis values as numbers.
#[must_use]
pub fn weight_coordinate(font: &Font) -> f64 {
    use objc2_core_text::{
        CTFont, kCTFontVariationAxisDefaultValueKey, kCTFontVariationAxisIdentifierKey,
    };
    use objc2_foundation::{NSArray, NSDictionary, NSNumber};

    // CoreText identifies an axis by its four-byte tag read as an integer.
    let wght = NSNumber::new_u32(u32::from_be_bytes(*b"wght"));
    // SAFETY: `NSFont` and `UIFont` are toll-free bridged with `CTFont`.
    let font = unsafe { &*core::ptr::from_ref(font).cast::<CTFont>() };
    // SAFETY: `CTFontCopyVariation` reads a live font.
    if let Some(variation) = unsafe { font.variation() } {
        // SAFETY: `CFDictionary` is toll-free bridged with `NSDictionary`;
        // the variation maps axis-identifier numbers to number values.
        let variation = unsafe {
            &*core::ptr::from_ref(&*variation).cast::<NSDictionary<NSNumber, NSNumber>>()
        };
        if let Some(value) = variation.objectForKey(&wght) {
            return value.doubleValue();
        }
    }
    // SAFETY: `CTFontCopyVariationAxes` reads a live font.
    let Some(axes) = (unsafe { font.variation_axes() }) else {
        contract_violation!(
            "the font has no variation axes, so no `wght` axis; the regular weight coordinate is answered"
        );
        return REGULAR_WEIGHT_COORDINATE;
    };
    // SAFETY: `CFArray` and `CFDictionary` are toll-free bridged with their
    // Foundation counterparts; each axis is a dictionary keyed by strings.
    let axes = unsafe {
        &*core::ptr::from_ref(&*axes).cast::<NSArray<NSDictionary<NSString, AnyObject>>>()
    };
    // SAFETY: the axis keys are `CFString` constants, toll-free bridged with
    // `NSString`.
    let (identifier_key, default_key) = unsafe {
        (
            &*core::ptr::from_ref(kCTFontVariationAxisIdentifierKey).cast::<NSString>(),
            &*core::ptr::from_ref(kCTFontVariationAxisDefaultValueKey).cast::<NSString>(),
        )
    };
    // `CoreText` documents the axis identifier and default as numbers.
    let number = |value: Retained<AnyObject>| {
        value
            .downcast::<NSNumber>()
            .expect("CoreText documents variation axis values as numbers")
    };
    let default = axes
        .iter()
        .find(|entry| {
            entry
                .objectForKey(identifier_key)
                .is_some_and(|identifier| *number(identifier) == *wght)
        })
        .and_then(|entry| entry.objectForKey(default_key))
        .map(|default| number(default).doubleValue());
    default.unwrap_or_else(|| {
        contract_violation!(
            "the font does not vary along the `wght` axis; the regular weight coordinate is answered"
        );
        REGULAR_WEIGHT_COORDINATE
    })
}

/// The `OpenType` `wght` axis default: regular weight is `400`.
const REGULAR_WEIGHT_COORDINATE: f64 = 400.0;

/// The point size of `font`.
///
/// # Safety
///
/// See the module safety note.
unsafe fn point_size(font: &Font) -> f64 {
    #[cfg(target_os = "ios")]
    {
        // SAFETY: see the module safety note.
        unsafe { font.pointSize() }
    }
    #[cfg(target_os = "macos")]
    {
        font.pointSize()
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
