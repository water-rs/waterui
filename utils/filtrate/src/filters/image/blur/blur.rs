//! Blur filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Applies a box blur effect to an image.
///
/// This is a spatial filter that samples neighboring pixels, so it
/// cannot be fused with other filters. It requires its own GPU pass.
///
/// # Parameters
///
/// - `radius`: Blur radius in pixels (0.0 = no blur, higher = more blur)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Blur;
///
/// let soft = Blur(5.0);
/// let heavy = Blur(20.0);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Blur<T>(pub T);

impl<T: FilterParam> Filter for Blur<T> {
    /// Blur samples neighboring pixels, so it cannot be fused.
    const COLOR_ONLY: bool = false;

    // Separable two-pass blur uses the same radius in both passes.
    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        let radius = self.0.snapshot();
        [radius, radius]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        // The standalone shader is retained for legacy direct rendering paths;
        // the planner uses `collect_stages` which emits the separable pair.
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/image/blur/blur.wgsl"))
    }

    fn pass_count(&self) -> u32 {
        2
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/image/blur/blur_horizontal.wgsl")), 1);
        c.spatial_shader(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/image/blur/blur_vertical.wgsl")), 1);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        // The radius drives both separable passes; broadcast to indices 0
        // and 1 so animation events update both pass uniforms.
        v.visit(0, &self.0);
        v.visit(1, &self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blur_params() {
        let filter = Blur(10.0f32);
        assert_eq!(filter.params(), [10.0, 10.0]);
    }

    #[test]
    fn test_blur_not_color_only() {
        assert!(!Blur::<f32>::COLOR_ONLY);
    }
}
