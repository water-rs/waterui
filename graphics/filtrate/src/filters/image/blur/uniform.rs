//! Blur filter implementation.

use crate::{
    Filter, FilterParam, Footprint, OperatingSpace, ParamSource, Placed, SignalVisitor,
    SpatialFilter, SpatialStage, StageCollector, filters::footprint, kind,
};

/// The separable box blur's stage: one axis, specialized per pass.
const BOX_BLUR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/shaders/image/blur/box_blur.wgsl"
));

/// The horizontal pass of a separable box blur of radius parameter 0.
pub const HORIZONTAL: SpatialStage = SpatialStage {
    name: "box_blur_horizontal",
    source: BOX_BLUR,
    params: &[ParamSource::Param(0), ParamSource::Constant(&[1.0, 0.0])],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[],
};

/// The vertical pass of a separable box blur of radius parameter 0.
const VERTICAL: SpatialStage = SpatialStage {
    name: "box_blur_vertical",
    source: BOX_BLUR,
    params: &[ParamSource::Param(0), ParamSource::Constant(&[0.0, 1.0])],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[],
};

/// Applies a box blur effect to an image.
///
/// It runs as two separable passes, horizontal then vertical, that share
/// one radius, so its footprint is the radius.
///
/// # Parameters
///
/// - `radius`: Blur radius in pixels, rounded (0.0 = no blur)
///
/// # Example
///
/// ```rust
/// # use filtrate::{Filter, Footprint, SpatialFilter};
/// use filtrate::filters::Blur;
///
/// let soft = Blur(5.0_f32);
/// let heavy = Blur(20.0_f32);
/// # assert_eq!(soft.params(), [5.0]);
/// # assert_eq!(heavy.footprint(), Footprint::pixels(20.0));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Blur<T>(pub T);

impl<T: FilterParam> Filter for Blur<T> {
    type Kind = kind::Spatial;
    type Params = [f32; 1];

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&HORIZONTAL));
        c.spatial(Placed::new(&VERTICAL));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.0);
    }
}

impl<T: FilterParam> SpatialFilter for Blur<T> {
    fn footprint_of(params: &[f32; 1]) -> Footprint {
        Footprint::pixels(footprint::rounded(params[0]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blur_params() {
        let filter = Blur(10.0f32);
        assert_eq!(filter.params(), [10.0]);
        assert_eq!(filter.footprint(), Footprint::pixels(10.0));
    }
}
