//! A `ResolvedFont` as the platform face every text-bearing port sets.
//!
//! The platform bold trait resolves the way `AppKit`/`UIKit` resolve it: a
//! theme slot declares the weight the platform gives its text style under
//! the bold trait (Semibold for body text), and any other face gets the bold
//! symbolic trait added to its realized descriptor, which turns a regular
//! fixed-size system font Bold.

use cocoa_ui::Retained;
use waterui::text::font::{FontDesign, FontWeight, ResolvedFont};

/// The platform weight of a `FontWeight` on the `UIFont`/`NSFont` scale.
pub(super) const fn platform_weight(weight: FontWeight) -> f64 {
    use cocoa_ui::font::weight;
    match weight {
        FontWeight::Thin => weight::THIN,
        FontWeight::UltraLight => weight::ULTRA_LIGHT,
        FontWeight::Light => weight::LIGHT,
        FontWeight::Normal => weight::REGULAR,
        FontWeight::Medium => weight::MEDIUM,
        FontWeight::SemiBold => weight::SEMI_BOLD,
        FontWeight::Bold => weight::BOLD,
        FontWeight::UltraBold => weight::HEAVY,
        FontWeight::Black => weight::BLACK,
    }
}

/// How a resolved font's weight reaches the platform face.
#[derive(Debug, PartialEq, Eq)]
struct FaceWeight {
    /// The weight the face is built at.
    weight: FontWeight,
    /// Whether the bold symbolic trait is added to the built face.
    bold_trait: bool,
}

/// A bold font with a declared bold weight is built at that weight; a bold
/// font without one is built at its own weight and takes the trait.
const fn face_weight(resolved: &ResolvedFont) -> FaceWeight {
    match (resolved.bold(), resolved.bold_weight()) {
        (true, Some(declared)) => FaceWeight {
            weight: declared,
            bold_trait: false,
        },
        (bold_trait, _) => FaceWeight {
            weight: resolved.weight(),
            bold_trait,
        },
    }
}

/// The comma-separated candidates of a CSS-style family list, trimmed with
/// the empties dropped — `fontFamilyCandidates` in the Swift port.
fn family_candidates(family: &str) -> impl Iterator<Item = &str> {
    family
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
}

/// The platform face a resolved font names: a named family resolved through
/// its candidate list — generics first, then `fontWithName:` — or the design
/// face, in the platform's bold face when the font asks for it and
/// italicized when the chunk asks for it.
///
/// A face with no bold or italic variant keeps its own, as the platform's
/// trait resolution does.
///
/// # Panics
///
/// When the family names no installed font and no generic — the same
/// `fatalError` the Swift port raises.
pub(super) fn platform_font(
    mtm: cocoa_ui::MainThreadMarker,
    resolved: &ResolvedFont,
    italic: bool,
) -> Retained<cocoa_ui::Font> {
    let size = f64::from(resolved.size());
    let face = face_weight(resolved);
    let weight = platform_weight(face.weight);
    let mut font = match resolved.family() {
        Some(family) if !family.is_empty() => {
            let mut resolved_font = None;
            for candidate in family_candidates(family) {
                resolved_font = match candidate {
                    "system" | "sans-serif" => Some(cocoa_ui::font::system(mtm, size, weight)),
                    _ => cocoa_ui::font::named(candidate, size),
                };
                if resolved_font.is_some() {
                    break;
                }
            }
            resolved_font.unwrap_or_else(|| {
                panic!(
                    "WaterUI: font family '{family}' not found. Ensure the font is bundled and registered."
                )
            })
        }
        _ => match resolved.design() {
            FontDesign::Default => cocoa_ui::font::system(mtm, size, weight),
            FontDesign::Monospaced => cocoa_ui::font::monospaced(mtm, size, weight),
        },
    };
    if face.bold_trait
        && let Some(bold_font) = cocoa_ui::font::bold_variant(&font)
    {
        font = bold_font;
    }
    if italic && let Some(italic_font) = cocoa_ui::font::italic_variant(&font) {
        font = italic_font;
    }
    font
}

/// The platform weight a resolved font's face carries, for ports that hand
/// the platform a weight rather than a face (symbol configurations, a
/// monospaced-digit readout).
pub(super) fn resolved_weight(mtm: cocoa_ui::MainThreadMarker, resolved: &ResolvedFont) -> f64 {
    let face = face_weight(resolved);
    if face.bold_trait {
        cocoa_ui::font::weight_of(&platform_font(mtm, resolved, false))
    } else {
        platform_weight(face.weight)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use waterui::reactive::Signal;
    use waterui::text::font::{Body, Font};
    use waterui::text::font::{FontWeight, ResolvedFont};
    use waterui_backend_core::Environment;

    use super::{FaceWeight, face_weight, family_candidates, platform_weight};

    #[test]
    fn family_candidates_splits_trims_and_drops_empties() {
        let candidates: Vec<&str> = family_candidates("Menlo , Monaco ,,  Courier New ").collect();
        assert_eq!(candidates, ["Menlo", "Monaco", "Courier New"]);
        assert!(family_candidates("").next().is_none());
        assert!(family_candidates(" ,  ").next().is_none());
        assert_eq!(family_candidates("system").collect::<Vec<_>>(), ["system"]);
    }

    #[test]
    fn platform_weight_maps_every_weight() {
        use cocoa_ui::font::weight;
        let table: [(FontWeight, f64); 9] = [
            (FontWeight::Thin, weight::THIN),
            (FontWeight::UltraLight, weight::ULTRA_LIGHT),
            (FontWeight::Light, weight::LIGHT),
            (FontWeight::Normal, weight::REGULAR),
            (FontWeight::Medium, weight::MEDIUM),
            (FontWeight::SemiBold, weight::SEMI_BOLD),
            (FontWeight::Bold, weight::BOLD),
            (FontWeight::UltraBold, weight::HEAVY),
            (FontWeight::Black, weight::BLACK),
        ];
        for (weight, expected) in table {
            assert_eq!(platform_weight(weight).to_bits(), expected.to_bits());
        }
    }

    fn resolve(font: &Font) -> ResolvedFont {
        let slot =
            ResolvedFont::new(17.0, FontWeight::Normal).with_bold_weight(FontWeight::SemiBold);
        let env = Environment::new().store::<Body, waterui::reactive::Computed<ResolvedFont>>(
            waterui::reactive::Computed::constant(slot),
        );
        font.resolve(&env).snapshot()
    }

    /// Bold on a theme slot is the weight the slot declares, the platform's
    /// own answer for its text style; an explicit weight 700 stays 700.
    #[test]
    fn bold_on_a_slot_builds_the_declared_weight() {
        assert_eq!(
            face_weight(&resolve(&Font::new(Body).bold())),
            FaceWeight {
                weight: FontWeight::SemiBold,
                bold_trait: false
            }
        );
        assert_eq!(
            face_weight(&resolve(&Font::new(Body).weight(FontWeight::Bold))),
            FaceWeight {
                weight: FontWeight::Bold,
                bold_trait: false
            }
        );
    }

    /// A resized slot is a fixed-size face: it keeps its own weight and the
    /// bold symbolic trait resolves it, as `systemFont(ofSize:)` does.
    #[test]
    fn bold_on_a_fixed_face_takes_the_symbolic_trait() {
        assert_eq!(
            face_weight(&resolve(&Font::new(Body).size(17.0).bold())),
            FaceWeight {
                weight: FontWeight::Normal,
                bold_trait: true
            }
        );
        assert_eq!(
            face_weight(&resolve(&Font::new(Body))),
            FaceWeight {
                weight: FontWeight::Normal,
                bold_trait: false
            }
        );
    }
}
