//! Luma curve filter implementation.

use crate::Filter;

/// The luma coefficients of sRGB primaries (ITU-R BT.709).
///
/// The Y row of the sRGB to XYZ matrix: the `LumaCurve` stage's `constants`
/// and its CPU kernel's luma — the stage operates in sRGB, not the
/// working space.
#[allow(
    clippy::redundant_pub_crate,
    reason = "the `pub use` chains in `color` and `filters` would carry a `pub` item into filtrate's public API"
)]
pub(crate) const SRGB_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Moves luma along a tone curve and scales the chroma around it, in sRGB.
///
/// On straight-alpha sRGB colour `c` with the BT.709 luma `Y = 0.2126 r + 0.7152 g + 0.0722 b`:
///
/// ```text
/// f(Y) = (1 − amount)·Y + amount·bezier(Y) + offset
/// out  = f(Y) + chroma·(c − Y)
/// ```
///
/// where `bezier` is the cubic Bézier over the four `curve` control values,
/// `(1−t)³v0 + 3t(1−t)²v1 + 3t²(1−t)v2 + t³v3`, defined on `[0, 1]`: a luma
/// outside it evaluates the curve at the nearer end, while the linear term
/// and the chroma stay extended. The stage operates in sRGB (sRGB primaries,
/// sRGB transfer); executors convert around it. It is not a linear map.
///
/// # Parameters
///
/// - `curve`: The Bézier control values `v0..v3` of the tone curve over luma
/// - `amount`: How much of the curve replaces the identity (0.0 = none, 1.0 = all)
/// - `chroma`: The gain on `c − Y` (0.0 = grey, 1.0 = unchanged chroma)
/// - `offset`: Added to every channel
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::LumaCurve;
///
/// let lifted = LumaCurve {
///     curve: [0.9_f32, 0.83, 0.925, 0.815],
///     amount: 0.75,
///     chroma: 0.375,
///     offset: 0.1,
/// };
/// # assert_eq!(lifted.params(), [0.9, 0.83, 0.925, 0.815, 0.75, 0.375, 0.1]);
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    color,
    shader = "color/adjustment/luma_curve.wgsl",
    linear = false,
    space = srgb,
    constants = [SRGB_LUMA[0], SRGB_LUMA[1], SRGB_LUMA[2]],
    cpu = crate::cpu::luma_curve
)]
pub struct LumaCurve<T> {
    /// The Bézier control values `v0..v3` of the tone curve over luma.
    pub curve: [T; 4],
    /// How much of the curve replaces the identity.
    pub amount: T,
    /// The gain on the chroma `c − Y`.
    pub chroma: T,
    /// The offset added to every channel.
    pub offset: T,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorFilter, Filter, OperatingSpace};

    #[test]
    fn luma_curve_params_flatten_in_field_order() {
        let filter = LumaCurve {
            curve: [0.1_f32, 0.2, 0.3, 0.4],
            amount: 0.5,
            chroma: 0.6,
            offset: 0.7,
        };
        assert_eq!(filter.params(), [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7]);
    }

    #[test]
    fn luma_curve_is_a_non_linear_srgb_stage() {
        struct Space(Option<OperatingSpace>);
        impl crate::StageCollector for Space {
            fn color(&mut self, stage: crate::Placed<crate::ColorStage>) {
                self.0 = Some(stage.stage.space);
            }
            fn spatial(&mut self, _: crate::Placed<crate::SpatialStage>) {
                panic!("the luma curve is a colour stage");
            }
        }
        const { assert!(!<LumaCurve<f32> as ColorFilter>::LINEAR) };
        let mut space = Space(None);
        LumaCurve {
            curve: [0.0_f32; 4],
            amount: 0.0,
            chroma: 1.0,
            offset: 0.0,
        }
        .collect_stages(&mut space);
        assert_eq!(space.0, Some(OperatingSpace::Srgb));
    }
}
