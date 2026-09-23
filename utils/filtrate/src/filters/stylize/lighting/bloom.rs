//! Bloom filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adds a bright glow around high-luminance regions.
#[derive(Debug, Clone)]
pub struct Bloom<T>(pub [T; 3]);

impl<T: FilterParam> Filter for Bloom<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 4];
    type Fragments = &'static str;

    fn params(&self) -> Self::Params {
        let [radius, intensity, threshold] = &self.0;
        [
            radius.snapshot(),
            threshold.snapshot(),
            radius.snapshot(),
            intensity.snapshot(),
        ]
    }

    fn fragments(&self) -> Self::Fragments {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/stylize/lighting/bloom_vertical.wgsl"
        ))
    }

    fn pass_count(&self) -> u32 {
        2
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
        let [radius, intensity, threshold] = &self.0;
        v.visit(0, radius);
        v.visit(1, threshold);
        v.visit(2, radius);
        v.visit(3, intensity);
    }
}
