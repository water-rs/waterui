//! Plain resource identifiers: the values recorded commands and updates
//! name for fonts, images, shaders and backdrop shaders. A render target
//! assigns them; the recording layer only carries them.

use crate::error::ResourceError;
use crate::glyph::FontId;
use crate::paint::{ImageId, ShaderId};
use crate::text::TextLayoutId;

/// A resource registered with a render target that installed content can
/// draw.
///
/// A target frees a released resource only once no installed content draws
/// it. A backdrop shader is never named by a command: content samples
/// fonts, images, text layouts and shader paints only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceId {
    /// A font.
    Font(FontId),
    /// An image.
    Image(ImageId),
    /// A text layout.
    TextLayout(TextLayoutId),
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
            Self::TextLayout(id) => write!(f, "text layout {}", id.raw()),
            Self::Shader(id) => write!(f, "shader {}", id.raw()),
            Self::BackdropShader(id) => write!(f, "backdrop shader {}", id.raw()),
        }
    }
}

/// Storage costs selected by a renderer for registered image formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageStorage {
    /// Bytes per stored RGBA8 pixel.
    pub rgba8: u8,
    /// Bytes per stored RGBA16F pixel.
    pub rgba16f: u8,
    /// Whether ASTC remains compressed in storage.
    pub astc4x4: bool,
    /// Whether ETC2 remains compressed in storage.
    pub etc2: bool,
    /// Whether BC7 remains compressed in storage.
    pub bc7: bool,
}

impl ImageStorage {
    /// Uncompressed GPU storage, decoding compressed sources to RGBA8.
    pub const GPU: Self = Self {
        rgba8: 4,
        rgba16f: 8,
        astc4x4: false,
        etc2: false,
        bc7: false,
    };

    /// CPU storage: every source decodes to four f32 channels.
    pub const CPU: Self = Self {
        rgba8: 16,
        rgba16f: 16,
        ..Self::GPU
    };

    /// Stored bytes, including complete compressed blocks at image edges.
    #[must_use]
    pub fn bytes(self, width: u32, height: u32, format: crate::ImageFormat) -> u128 {
        use crate::ImageFormat as F;
        let compressed = match format {
            F::Astc4x4 => self.astc4x4,
            F::Etc2Rgba => self.etc2,
            F::Bc7 => self.bc7,
            F::Rgba8 | F::Rgba16F => false,
        };
        if compressed {
            u128::from(width.div_ceil(4)) * u128::from(height.div_ceil(4)) * 16
        } else {
            let pixel = if format == F::Rgba16F {
                self.rgba16f
            } else {
                self.rgba8
            };
            u128::from(width) * u128::from(height) * u128::from(pixel)
        }
    }
}

/// Immutable dimensions, storage costs and shared byte budget for registered images.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageLimits {
    /// Largest admitted width or height, in pixels.
    pub max_dimension: u32,
    /// Maximum stored bytes held by all registered images together.
    pub max_bytes: u64,
    /// The renderer's storage costs, including device compression support.
    pub storage: ImageStorage,
}

impl ImageLimits {
    /// Unbounded dimensions and byte budget, using uncompressed GPU storage.
    pub const UNLIMITED: Self = Self {
        max_dimension: u32::MAX,
        max_bytes: u64::MAX,
        storage: ImageStorage::GPU,
    };

    /// Whether an image fits on its own.
    #[must_use]
    pub fn admits<F: crate::Format>(&self, width: u32, height: u32) -> bool {
        width != 0
            && height != 0
            && width <= self.max_dimension
            && height <= self.max_dimension
            && self.storage.bytes(width, height, F::FORMAT) <= u128::from(self.max_bytes)
    }

    /// Checks dimensions and stored bytes before queuing an upload.
    ///
    /// # Errors
    /// Returns [`ResourceError::TooLarge`] when the image does not fit alone.
    pub fn check<F: crate::Format>(&self, width: u32, height: u32) -> Result<(), ResourceError> {
        if self.admits::<F>(width, height) {
            Ok(())
        } else {
            Err(ResourceError::TooLarge {
                width,
                height,
                limits: *self,
            })
        }
    }

    /// Checks an image beside `others` stored bytes, which are already within the budget.
    ///
    /// # Errors
    /// Returns [`ResourceError::TooLarge`] when the image does not fit alone,
    /// or [`ResourceError::OverBudget`] when it does not fit beside the others.
    ///
    /// # Panics
    /// Panics if `others` exceeds the budget; callers pass admitted resident bytes.
    pub fn check_beside<F: crate::Format>(
        &self,
        width: u32,
        height: u32,
        others: u64,
    ) -> Result<u64, ResourceError> {
        self.check::<F>(width, height)?;
        let bytes = u64::try_from(self.storage.bytes(width, height, F::FORMAT))
            .expect("admitted bytes fit the u64 budget");
        let available = self.max_bytes - others;
        if bytes > available {
            return Err(ResourceError::OverBudget {
                width,
                height,
                available,
                limits: *self,
            });
        }
        Ok(bytes)
    }

    /// Largest size at the requested aspect ratio that fits alone, to integer precision.
    /// Returns `(0, 0)` when no pixel fits. Subpixel sides retain one pixel.
    ///
    /// # Panics
    /// Never: the result only shrinks the input dimensions.
    #[must_use]
    pub fn fit<F: crate::Format>(&self, width: u32, height: u32) -> (u32, u32) {
        if self.admits::<F>(width, height) {
            return (width, height);
        }
        if width == 0 || height == 0 || !self.admits::<F>(1, 1) {
            return (0, 0);
        }
        let long = width.max(height);
        let size = |side: u32| {
            (
                u32::try_from(u64::from(width) * u64::from(side) / u64::from(long))
                    .expect("shrinks width")
                    .max(1),
                u32::try_from(u64::from(height) * u64::from(side) / u64::from(long))
                    .expect("shrinks height")
                    .max(1),
            )
        };
        let (mut low, mut high) = (1, long.min(self.max_dimension));
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            let (w, h) = size(middle);
            if self.admits::<F>(w, h) {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        size(low)
    }
}

impl std::fmt::Display for ImageLimits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} px/side, {} bytes",
            self.max_dimension, self.max_bytes
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
        max_bytes: 6_291_456 * 4,
        storage: super::ImageStorage::GPU,
    };

    #[test]
    fn admission_checks_dimensions_and_stored_bytes() {
        assert!(LIMITS.admits::<crate::Rgba8>(1, 1));
        assert!(LIMITS.admits::<crate::Rgba8>(16384, 1));
        assert!(LIMITS.admits::<crate::Rgba8>(2508, 2508));
        assert!(!LIMITS.admits::<crate::Rgba8>(16385, 1));
        assert!(!LIMITS.admits::<crate::Rgba8>(1, 16385));
        assert!(!LIMITS.admits::<crate::Rgba8>(4096, 4096));
        assert!(!LIMITS.admits::<crate::Rgba8>(0, 4));
        assert!(!LIMITS.admits::<crate::Rgba8>(4, 0));
        let bytes = ImageLimits {
            max_bytes: 64,
            max_dimension: 16,
            storage: super::ImageStorage {
                bc7: true,
                ..super::ImageStorage::GPU
            },
        };
        assert_eq!(bytes.check_beside::<crate::Rgba8>(4, 4, 0).unwrap(), 64);
        assert!(!bytes.admits::<crate::Rgba16F>(4, 4));
        assert_eq!(bytes.check_beside::<crate::Bc7>(5, 5, 0).unwrap(), 64);
        assert_eq!(bytes.fit::<crate::Bc7>(16, 16), (8, 8));
        let cpu = ImageLimits {
            storage: super::ImageStorage::CPU,
            ..bytes
        };
        assert_eq!(cpu.check_beside::<crate::Rgba8>(2, 2, 0).unwrap(), 64);
        assert!(!cpu.admits::<crate::Bc7>(4, 4));
    }

    #[test]
    fn an_admitted_size_is_its_own_fit() {
        assert_eq!(LIMITS.fit::<crate::Rgba8>(64, 64), (64, 64));
        assert_eq!(LIMITS.fit::<crate::Rgba8>(16384, 1), (16384, 1));
    }

    #[test]
    fn fit_scales_to_the_binding_limit() {
        // The dimension binds: 20000px of width becomes 16384 and the
        // height scales by the same ratio.
        assert_eq!(LIMITS.fit::<crate::Rgba8>(20000, 16), (16384, 13));
        // The texel budget binds below the dimension: the 16384 square
        // the dimension would leave still holds too many texels.
        assert_eq!(LIMITS.fit::<crate::Rgba8>(32768, 32768), (2508, 2508));
        assert_eq!(LIMITS.fit::<crate::Rgba8>(4096, 4096), (2508, 2508));
    }

    #[test]
    fn fit_keeps_one_pixel_on_a_side_the_ratio_shrinks_to_zero() {
        // A strip too wide for the dimension: the floored height is
        // zero, so the width carries the texel bound alone.
        assert_eq!(
            ImageLimits {
                max_dimension: 16384,
                max_bytes: 100 * 4,
                storage: super::ImageStorage::GPU,
            }
            .fit::<crate::Rgba8>(20000, 1),
            (100, 1)
        );
        assert_eq!(
            ImageLimits {
                max_dimension: u32::MAX,
                max_bytes: 6_291_456 * 4,
                storage: super::ImageStorage::GPU,
            }
            .fit::<crate::Rgba8>(100_000_000, 1),
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
                    max_bytes: 6_291_456 * 4,
                    storage: super::ImageStorage::GPU,
                },
                (100_000_000, 1),
            ),
        ] {
            let (w, h) = limits.fit::<crate::Rgba8>(size.0, size.1);
            assert!(
                limits.admits::<crate::Rgba8>(w, h),
                "{limits:?}.fit{size:?} = ({w}, {h})"
            );
        }
    }

    #[test]
    fn fit_reports_zero_when_nothing_is_admitted() {
        assert_eq!(LIMITS.fit::<crate::Rgba8>(0, 16), (0, 0));
        assert_eq!(
            ImageLimits {
                max_dimension: 0,
                max_bytes: 16 * 4,
                storage: super::ImageStorage::GPU,
            }
            .fit::<crate::Rgba8>(4, 4),
            (0, 0)
        );
    }
}
