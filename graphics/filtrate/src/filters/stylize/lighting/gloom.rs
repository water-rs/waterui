//! Gloom filter implementation.

use crate::{
    AuxSource, Filter, FilterParam, Footprint, OperatingSpace, ParamSource, Placed, SignalVisitor,
    SpatialFilter, SpatialStage, StageCollector, filters::footprint, kind,
};

/// The second pass: finishes the box blur vertically and subtracts the glow
/// from the first pass's input.
const COMPOSITE: SpatialStage = SpatialStage {
    name: "gloom_composite",
    source: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/stylize/lighting/glow_composite.wgsl"
    )),
    params: &[
        ParamSource::Param(0),
        ParamSource::Param(1),
        ParamSource::Constant(&[-1.0]),
    ],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[AuxSource::PreviousStageInput],
};

/// Dulls highlights by subtracting a blurred copy of the bright regions.
///
/// Runs as two separable passes: a horizontal pass extracts thresholded
/// highlight energy, and a vertical pass finishes the blur and subtracts the
/// glow from the input. The footprint is the radius.
#[derive(Debug, Clone)]
pub struct Gloom<T> {
    /// Blur radius of the darkening halo, in pixels (at least one).
    pub radius: T,
    /// Strength of the subtractive darkening (0.0 = none).
    pub intensity: T,
    /// Luminance threshold below which pixels contribute no darkening.
    pub threshold: T,
}

impl<T: FilterParam> Filter for Gloom<T> {
    type Kind = kind::Spatial;
    type Params = [f32; 3];

    fn params(&self) -> Self::Params {
        [
            self.radius.snapshot(),
            self.intensity.snapshot(),
            self.threshold.snapshot(),
        ]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&super::bloom::EXTRACT));
        c.spatial(Placed::new(&COMPOSITE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.radius);
        v.visit(1, &self.intensity);
        v.visit(2, &self.threshold);
    }
}

impl<T: FilterParam> SpatialFilter for Gloom<T> {
    fn footprint_of(params: &[f32; 3]) -> Footprint {
        Footprint::pixels(footprint::rounded_at_least_one(params[0]))
    }
}
