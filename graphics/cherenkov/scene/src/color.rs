use serde::{Deserialize, Serialize};

/// A colour space tag carried by every [`Color`].
///
/// Values `Srgb` and `DisplayP3` are encoded (non-linear) spaces;
/// `LinearSrgb`, `LinearP3` and `Rec2020` are linear-light. The scene's
/// working space is always linear Display P3.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ColorSpace {
    /// sRGB, gamma encoded.
    #[default]
    Srgb,
    /// Display P3, gamma encoded with the sRGB transfer function.
    DisplayP3,
    /// Linear-light sRGB (same primaries as sRGB, no transfer).
    LinearSrgb,
    /// Linear-light Display P3 — the suite's working space.
    LinearP3,
    /// Linear-light Rec. 2020.
    Rec2020,
}

/// An RGBA colour tagged with its colour space.
///
/// Components may exceed `1.0` for extended-range / HDR content.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    /// The space `components` is expressed in.
    pub space: ColorSpace,
    /// `[r, g, b, a]`; `a` is an un-premultiplied coverage in `0.0..=1.0`.
    pub components: [f32; 4],
}

impl Color {
    /// An sRGB colour with an opaque alpha.
    #[must_use]
    pub const fn srgb(r: f32, g: f32, b: f32) -> Self {
        Self::new(ColorSpace::Srgb, [r, g, b, 1.0])
    }

    /// A colour in `space`. Components may exceed `1.0` for HDR.
    #[must_use]
    pub const fn new(space: ColorSpace, components: [f32; 4]) -> Self {
        Self { space, components }
    }

    /// This colour with a different alpha.
    #[must_use]
    pub const fn with_alpha(self, alpha: f32) -> Self {
        let mut components = self.components;
        components[3] = alpha;
        Self {
            space: self.space,
            components,
        }
    }

    /// `true` when any colour channel exceeds `1.0` (HDR / extended range).
    #[must_use]
    pub fn is_hdr(self) -> bool {
        self.components[..3].iter().any(|&c| c > 1.0)
    }

    /// `true` when the colour space is not within the sRGB gamut or is HDR.
    #[must_use]
    pub fn is_wide_gamut(self) -> bool {
        self.is_hdr()
            || matches!(
                self.space,
                ColorSpace::DisplayP3 | ColorSpace::LinearP3 | ColorSpace::Rec2020
            )
    }
}
