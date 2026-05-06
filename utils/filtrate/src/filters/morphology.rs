//! 3x3 morphological filters (erosion, dilation, gradient).

use crate::FilterDerive;

/// 3x3 per-channel minimum (morphological erosion). Shrinks bright regions
/// and grows dark ones — useful for thinning text or removing isolated
/// bright noise.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(spatial, shader = "morphology_min.wgsl")]
pub struct MorphologyMin;

/// 3x3 per-channel maximum (morphological dilation). Grows bright regions
/// and shrinks dark ones — the dual of [`MorphologyMin`].
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(spatial, shader = "morphology_max.wgsl")]
pub struct MorphologyMax;

/// 3x3 morphological gradient: per-channel `max - min` over the
/// neighbourhood. Highlights boundaries between regions of differing
/// luminance.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(spatial, shader = "morphology_gradient.wgsl")]
pub struct MorphologyGradient;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn morphology_filters_are_spatial_zero_param() {
        assert!(!MorphologyMin::COLOR_ONLY);
        assert!(!MorphologyMax::COLOR_ONLY);
        assert!(!MorphologyGradient::COLOR_ONLY);
        assert_eq!(MorphologyMin.params().len(), 0);
        assert_eq!(MorphologyMax.params().len(), 0);
        assert_eq!(MorphologyGradient.params().len(), 0);
    }
}
