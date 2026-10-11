//! Unsharp mask filter implementation.

use crate::{
    AuxSource, Filter, FilterParam, Footprint, OperatingSpace, ParamSource, Placed, SignalVisitor,
    SpatialFilter, SpatialStage, StageCollector,
    filters::{footprint, image::blur::HORIZONTAL},
    kind,
};

/// The second pass: finishes the blur vertically and sharpens the first
/// pass's input against it.
const SHARPEN: SpatialStage = SpatialStage {
    name: "unsharp_mask",
    source: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/image/convolution/unsharp_mask.wgsl"
    )),
    params: &[ParamSource::Param(0), ParamSource::Param(1)],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[AuxSource::PreviousStageInput],
};

/// Sharpens image detail using an unsharp mask.
///
/// Runs as two separable passes — the shared horizontal box blur followed
/// by a vertical pass that finishes the blur and sharpens the first pass's
/// input against it (`original + (original - blurred) * intensity`). The
/// separable pair costs `2(2r+1)` taps per pixel instead of `(2r+1)^2`, and
/// the footprint is the radius.
#[derive(Debug, Clone)]
pub struct UnsharpMask<T> {
    /// Blur radius of the mask, in pixels.
    pub radius: T,
    /// Sharpening strength (0.0 = none).
    pub intensity: T,
}

impl<T: FilterParam> Filter for UnsharpMask<T> {
    type Kind = kind::Spatial;
    type Params = [f32; 2];

    fn params(&self) -> Self::Params {
        [self.radius.snapshot(), self.intensity.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&HORIZONTAL));
        c.spatial(Placed::new(&SHARPEN));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.radius);
        v.visit(1, &self.intensity);
    }
}

impl<T: FilterParam> SpatialFilter for UnsharpMask<T> {
    fn footprint_of(params: &[f32; 2]) -> Footprint {
        Footprint::pixels(footprint::rounded(params[0]))
    }
}
