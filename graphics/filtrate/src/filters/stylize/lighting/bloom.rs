//! Bloom filter implementation.

use crate::{
    AuxSource, Filter, FilterParam, Footprint, OperatingSpace, ParamSource, Placed, SignalVisitor,
    SpatialFilter, SpatialStage, StageCollector, filters::footprint, kind,
};

/// The first pass, shared by bloom and gloom: thresholded highlight energy,
/// box-accumulated horizontally. Parameters: radius, threshold.
pub(super) const EXTRACT: SpatialStage = SpatialStage {
    name: "glow_extract",
    source: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/stylize/lighting/glow_extract.wgsl"
    )),
    params: &[ParamSource::Param(0), ParamSource::Param(2)],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[],
};

/// The second pass: finishes the box blur vertically and adds the glow
/// onto the first pass's input.
const COMPOSITE: SpatialStage = SpatialStage {
    name: "bloom_composite",
    source: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/stylize/lighting/glow_composite.wgsl"
    )),
    params: &[
        ParamSource::Param(0),
        ParamSource::Param(1),
        ParamSource::Constant(&[1.0]),
    ],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[AuxSource::PreviousStageInput],
};

/// Adds a bright glow around high-luminance regions.
///
/// Runs as two separable passes: a horizontal pass extracts thresholded
/// highlight energy, and a vertical pass finishes the blur and adds the
/// glow onto the input. The footprint is the radius.
#[derive(Debug, Clone)]
pub struct Bloom<T> {
    /// Blur radius of the glow, in pixels (at least one).
    pub radius: T,
    /// Strength of the additive glow (0.0 = none).
    pub intensity: T,
    /// Luminance threshold below which pixels contribute no glow.
    pub threshold: T,
}

impl<T: FilterParam> Filter for Bloom<T> {
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
        c.spatial(Placed::new(&EXTRACT));
        c.spatial(Placed::new(&COMPOSITE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.radius);
        v.visit(1, &self.intensity);
        v.visit(2, &self.threshold);
    }
}

impl<T: FilterParam> SpatialFilter for Bloom<T> {
    fn footprint_of(params: &[f32; 3]) -> Footprint {
        Footprint::pixels(footprint::rounded_at_least_one(params[0]))
    }
}
