//! Plain resource identifiers: the values recorded commands and updates
//! name for fonts, images, shaders and backdrop shaders. A render target
//! assigns them; the recording layer only carries them.

use crate::glyph::FontId;
use crate::paint::{ImageId, ShaderId};

/// A resource registered with a render target that installed content can
/// draw.
///
/// A target frees a released resource only once no installed content draws
/// it. A backdrop shader is never named by a command: content samples
/// fonts, images and shader paints only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceId {
    /// A font.
    Font(FontId),
    /// An image.
    Image(ImageId),
    /// A user shader.
    Shader(ShaderId),
    /// A backdrop effect shader.
    BackdropShader(BackdropShaderId),
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Font(id) => write!(f, "font {}", id.raw()),
            Self::Image(id) => write!(f, "image {}", id.raw()),
            Self::Shader(id) => write!(f, "shader {}", id.raw()),
            Self::BackdropShader(id) => write!(f, "backdrop shader {}", id.raw()),
        }
    }
}

/// The largest image a render target admits, in each dimension and in
/// total texels.
///
/// The target reports it once, when its renderer is initialized, and it
/// does not change afterwards: a registration the limits do not
/// [`admit`](Self::admits) is rejected before any work is queued, so the
/// content that made it sees the error instead of a failed render.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageLimits {
    /// Largest admitted width or height, in pixels.
    pub max_dimension: u32,
    /// Largest admitted texel count (`width * height`).
    pub max_texels: u64,
}

impl ImageLimits {
    /// No limit: every size is admitted.
    pub const UNLIMITED: Self = Self {
        max_dimension: u32::MAX,
        max_texels: u64::MAX,
    };

    /// Whether a `width` × `height` image fits the limits.
    #[must_use]
    pub fn admits(&self, width: u32, height: u32) -> bool {
        width != 0
            && height != 0
            && width <= self.max_dimension
            && height <= self.max_dimension
            && u64::from(width) * u64::from(height) <= self.max_texels
    }

    /// The largest admitted size at `width`:`height`'s aspect ratio, to
    /// integer precision. `(0, 0)` when the limits admit nothing — a
    /// zero `max_dimension` or `max_texels` — and a side under a pixel
    /// keeps one pixel, with the texel bound carried by the side the
    /// ratio left standing.
    #[must_use]
    pub fn fit(&self, width: u32, height: u32) -> (u32, u32) {
        if self.admits(width, height) {
            return (width, height);
        }
        if width == 0 || height == 0 || self.max_dimension == 0 || self.max_texels == 0 {
            return (0, 0);
        }
        let (mut w, mut h) = (u128::from(width), u128::from(height));
        let max = u128::from(self.max_dimension);
        let texels = u128::from(self.max_texels);
        // Each bound scales uniformly and floors, so the pair it leaves
        // is still admitted. The dimension bound applies to the long
        // side; the texel bound scales each side by `sqrt(T / (w·h))`.
        let long = w.max(h);
        if long > max {
            w = w * max / long;
            h = h * max / long;
        }
        if w != 0 && h != 0 && w * h > texels {
            let (a, b) = (w, h);
            w = (a * texels / b).isqrt();
            h = (b * texels / a).isqrt();
        }
        // Only a side floored to zero needs repair: one pixel, with the
        // texel bound carried by the side the ratio left standing.
        if w == 0 {
            w = 1;
            h = h.min(texels).max(1);
        }
        if h == 0 {
            h = 1;
            w = w.min(texels).max(1);
        }
        (
            u32::try_from(w).expect("fit only shrinks a u32 size"),
            u32::try_from(h).expect("fit only shrinks a u32 size"),
        )
    }
}

impl std::fmt::Display for ImageLimits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} px/side, {} texels",
            self.max_dimension, self.max_texels
        )
    }
}

/// Identifier of a backdrop effect shader, allocated by the target that
/// registers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BackdropShaderId(u64);

impl BackdropShaderId {
    /// Creates an identifier from a target-assigned raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::ImageLimits;

    const LIMITS: ImageLimits = ImageLimits {
        max_dimension: 16384,
        max_texels: 6_291_456,
    };

    #[test]
    fn admits_checks_each_dimension_and_the_texel_count() {
        assert!(LIMITS.admits(1, 1));
        assert!(LIMITS.admits(16384, 1));
        assert!(LIMITS.admits(2508, 2508));
        assert!(!LIMITS.admits(16385, 1));
        assert!(!LIMITS.admits(1, 16385));
        assert!(!LIMITS.admits(4096, 4096));
        assert!(!LIMITS.admits(0, 4));
        assert!(!LIMITS.admits(4, 0));
    }

    #[test]
    fn an_admitted_size_is_its_own_fit() {
        assert_eq!(LIMITS.fit(64, 64), (64, 64));
        assert_eq!(LIMITS.fit(16384, 1), (16384, 1));
    }

    #[test]
    fn fit_scales_to_the_binding_limit() {
        // The dimension binds: 20000px of width becomes 16384 and the
        // height scales by the same ratio.
        assert_eq!(LIMITS.fit(20000, 16), (16384, 13));
        // The texel budget binds below the dimension: the 16384 square
        // the dimension would leave still holds too many texels.
        assert_eq!(LIMITS.fit(32768, 32768), (2508, 2508));
        assert_eq!(LIMITS.fit(4096, 4096), (2508, 2508));
    }

    #[test]
    fn fit_keeps_one_pixel_on_a_side_the_ratio_shrinks_to_zero() {
        // A strip too wide for the dimension: the floored height is
        // zero, so the width carries the texel bound alone.
        assert_eq!(
            ImageLimits {
                max_dimension: 16384,
                max_texels: 100,
            }
            .fit(20000, 1),
            (100, 1)
        );
        assert_eq!(
            ImageLimits {
                max_dimension: u32::MAX,
                max_texels: 6_291_456,
            }
            .fit(100_000_000, 1),
            (6_291_456, 1)
        );
    }

    #[test]
    fn fit_fits_what_it_returns() {
        for (limits, size) in [
            (LIMITS, (20000, 16)),
            (LIMITS, (8192, 8192)),
            (LIMITS, (1, 40000)),
            (
                ImageLimits {
                    max_dimension: u32::MAX,
                    max_texels: 6_291_456,
                },
                (100_000_000, 1),
            ),
        ] {
            let (w, h) = limits.fit(size.0, size.1);
            assert!(limits.admits(w, h), "{limits:?}.fit{size:?} = ({w}, {h})");
        }
    }

    #[test]
    fn fit_reports_zero_when_nothing_is_admitted() {
        assert_eq!(LIMITS.fit(0, 16), (0, 0));
        assert_eq!(
            ImageLimits {
                max_dimension: 0,
                max_texels: 16,
            }
            .fit(4, 4),
            (0, 0)
        );
    }
}
