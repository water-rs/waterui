//! Bloom filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adds a bright glow around high-luminance regions.
///
/// Runs as two separable passes: a horizontal pass extracts thresholded
/// highlight energy, and a vertical pass finishes the blur and composites
/// the glow onto the original input additively.
#[derive(Debug, Clone)]
pub struct Bloom<T> {
    /// Blur radius of the glow, in pixels.
    pub radius: T,
    /// Strength of the additive glow (0.0 = none).
    pub intensity: T,
    /// Luminance threshold below which pixels contribute no glow.
    pub threshold: T,
}

impl<T: FilterParam> Bloom<T> {
    /// Uniform slot layout across both passes: the horizontal pass consumes
    /// `[radius, threshold]`, the vertical pass `[radius, intensity]`. Both
    /// `params` and `visit_signals` derive from this single list, so the
    /// orderings cannot drift apart.
    const fn param_slots(&self) -> [&T; 4] {
        [&self.radius, &self.threshold, &self.radius, &self.intensity]
    }
}

impl<T: FilterParam> Filter for Bloom<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 4];

    fn params(&self) -> Self::Params {
        self.param_slots().map(FilterParam::snapshot)
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/stylize/lighting/bloom_horizontal.wgsl"
            )),
            2,
        );
        c.spatial_shader_with_original(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/stylize/lighting/bloom_vertical.wgsl"
            )),
            2,
        );
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        for (index, param) in self.param_slots().into_iter().enumerate() {
            v.visit(index, param);
        }
    }
}
