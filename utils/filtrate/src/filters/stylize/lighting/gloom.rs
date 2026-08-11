//! Gloom filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Dulls highlights by subtracting a blurred copy of the bright regions.
///
/// Runs as two separable passes: a horizontal pass extracts thresholded
/// highlight energy, and a vertical pass finishes the blur and subtracts
/// it from the original input.
#[derive(Debug, Clone)]
pub struct Gloom<T> {
    /// Blur radius of the darkening halo, in pixels.
    pub radius: T,
    /// Strength of the subtractive darkening (0.0 = none).
    pub intensity: T,
    /// Luminance threshold below which pixels contribute no darkening.
    pub threshold: T,
}

impl<T: FilterParam> Gloom<T> {
    /// Uniform slot layout across both passes: the horizontal pass consumes
    /// `[radius, threshold]`, the vertical pass `[radius, intensity]`. Both
    /// `params` and `visit_signals` derive from this single list, so the
    /// orderings cannot drift apart.
    const fn param_slots(&self) -> [&T; 4] {
        [&self.radius, &self.threshold, &self.radius, &self.intensity]
    }
}

impl<T: FilterParam> Filter for Gloom<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 4];

    fn params(&self) -> Self::Params {
        self.param_slots().map(FilterParam::snapshot)
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/stylize/lighting/gloom_horizontal.wgsl"
            )),
            2,
        );
        c.spatial_shader_with_original(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/stylize/lighting/gloom_vertical.wgsl"
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
