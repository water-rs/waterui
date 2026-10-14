//! Gaussian blur filter implementation.

use crate::{
    Filter, FilterParam, Footprint, OperatingSpace, ParamSource, Placed, SignalVisitor,
    SpatialFilter, SpatialStage, StageCollector, kind,
};

/// The separable gaussian blur's stage: one axis, specialized per pass.
const GAUSSIAN_BLUR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/shaders/image/blur/gaussian_blur.wgsl"
));

/// Gaussian blur kernel radius in multiples of the standard deviation.
pub const GAUSSIAN_RADIUS_PER_SIGMA: f32 = 4.0;

/// The horizontal pass, averaging in the working space.
const HORIZONTAL: SpatialStage = SpatialStage {
    name: "gaussian_blur_horizontal",
    source: GAUSSIAN_BLUR,
    params: &[ParamSource::Param(0), ParamSource::Constant(&[1.0, 0.0])],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[],
};

/// The vertical pass, averaging in the working space.
const VERTICAL: SpatialStage = SpatialStage {
    name: "gaussian_blur_vertical",
    source: GAUSSIAN_BLUR,
    params: &[ParamSource::Param(0), ParamSource::Constant(&[0.0, 1.0])],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[],
};

/// The horizontal pass, averaging encoded sRGB. Its own name keeps caches
/// keyed by stage name from confusing it with the working-space pass.
const HORIZONTAL_SRGB: SpatialStage = SpatialStage {
    name: "gaussian_blur_horizontal_srgb",
    source: GAUSSIAN_BLUR,
    params: &[ParamSource::Param(0), ParamSource::Constant(&[1.0, 0.0])],
    space: OperatingSpace::Srgb,
    shape: None,
    aux: &[],
};

/// The vertical pass, averaging encoded sRGB.
const VERTICAL_SRGB: SpatialStage = SpatialStage {
    name: "gaussian_blur_vertical_srgb",
    source: GAUSSIAN_BLUR,
    params: &[ParamSource::Param(0), ParamSource::Constant(&[0.0, 1.0])],
    space: OperatingSpace::Srgb,
    shape: None,
    aux: &[],
};

/// Applies a separable gaussian blur: a horizontal then a vertical pass.
///
/// The blur averages in its operating space: the linear working space by
/// default, or encoded sRGB through [`in_space`](Self::in_space), where an
/// edge between dark and light settles halfway in encoded value rather than
/// in light.
///
/// # Parameters
///
/// - `sigma`: Gaussian standard deviation in pixels; the kernel radius, and
///   the footprint, is `ceil(4 * sigma)`.
/// - `space`: The colour space the taps are averaged in.
///
/// # Example
///
/// ```rust
/// # use filtrate::{Filter, Footprint, SpatialFilter};
/// use filtrate::OperatingSpace;
/// use filtrate::filters::GaussianBlur;
///
/// let soft = GaussianBlur::new(2.0_f32);
/// let encoded = GaussianBlur::new(2.0_f32).in_space(OperatingSpace::Srgb);
/// # assert_eq!(soft.space, OperatingSpace::Working);
/// # assert_eq!(encoded.params(), [2.0]);
/// # assert_eq!(encoded.footprint(), Footprint::pixels(8.0));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct GaussianBlur<T> {
    /// The gaussian standard deviation in pixels.
    pub sigma: T,
    /// The colour space the taps are averaged in.
    pub space: OperatingSpace,
}

impl<T> GaussianBlur<T> {
    /// A blur of standard deviation `sigma` pixels, averaging in the linear
    /// working space.
    #[must_use]
    pub const fn new(sigma: T) -> Self {
        Self {
            sigma,
            space: OperatingSpace::Working,
        }
    }

    /// The same blur, averaging in `space`.
    #[must_use]
    pub const fn in_space(mut self, space: OperatingSpace) -> Self {
        self.space = space;
        self
    }
}

impl<T: FilterParam> Filter for GaussianBlur<T> {
    type Kind = kind::Spatial;
    type Params = [f32; 1];

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.sigma.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        let (horizontal, vertical) = match self.space {
            OperatingSpace::Working => (&HORIZONTAL, &VERTICAL),
            OperatingSpace::Srgb => (&HORIZONTAL_SRGB, &VERTICAL_SRGB),
        };
        c.spatial(Placed::new(horizontal));
        c.spatial(Placed::new(vertical));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.sigma);
    }
}

impl<T: FilterParam> SpatialFilter for GaussianBlur<T> {
    fn footprint_of(params: &[f32; 1]) -> Footprint {
        Footprint::pixels((params[0].max(0.001) * GAUSSIAN_RADIUS_PER_SIGMA).ceil())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_footprint_matches_kernel_radius() {
        assert_eq!(
            GaussianBlur::new(2.0f32).footprint(),
            Footprint::pixels(8.0)
        );
        assert_eq!(
            GaussianBlur::new(0.4f32).footprint(),
            Footprint::pixels(2.0)
        );
        assert_eq!(
            GaussianBlur::new(0.001f32).footprint(),
            Footprint::pixels(1.0)
        );
    }

    #[test]
    fn wgsl_radius_matches_rust_constant() {
        let radius = format!("{GAUSSIAN_RADIUS_PER_SIGMA:.1}");
        let declaration = format!("const RADIUS_PER_SIGMA: f32 = {radius};");
        assert!(
            GAUSSIAN_BLUR
                .lines()
                .any(|line| line == declaration.as_str()),
            "WGSL must declare `{declaration}`"
        );
    }
}
