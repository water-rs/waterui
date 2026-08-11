//! Unsharp mask filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Sharpens image detail using an unsharp mask.
///
/// Runs as two separable passes — the shared horizontal box blur followed
/// by a vertical pass that finishes the blur and sharpens against the
/// untouched original (`original + (original - blurred) * intensity`).
/// The separable pair costs `2(2r+1)` taps per pixel instead of `(2r+1)^2`.
#[derive(Debug, Clone)]
pub struct UnsharpMask<T> {
    /// Blur radius of the mask, in pixels.
    pub radius: T,
    /// Sharpening strength (0.0 = none).
    pub intensity: T,
}

impl<T: FilterParam> UnsharpMask<T> {
    /// Uniform slot layout across both passes: the horizontal blur consumes
    /// `[radius]`, the vertical pass `[radius, intensity]`. Both `params`
    /// and `visit_signals` derive from this single list, so the orderings
    /// cannot drift apart.
    const fn param_slots(&self) -> [&T; 3] {
        [&self.radius, &self.radius, &self.intensity]
    }
}

impl<T: FilterParam> Filter for UnsharpMask<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 3];

    fn params(&self) -> Self::Params {
        self.param_slots().map(FilterParam::snapshot)
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/image/blur/blur_horizontal.wgsl"
            )),
            1,
        );
        c.spatial_shader_with_original(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/image/convolution/unsharp_mask_vertical.wgsl"
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
