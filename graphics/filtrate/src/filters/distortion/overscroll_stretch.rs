//! Overscroll stretch filter implementation.

use crate::Filter;

/// How much the stretch varies with the distance from the pulled edge.
///
/// HWUI's `INTERPOLATION_STRENGTH_VALUE` (`StretchEffect.cpp`). At `0` every
/// position stretches alike; at `1` the far edge does not stretch at all.
pub const STRETCH_INTERPOLATION_STRENGTH: f32 = 0.7;

/// Android's edge-weighted overscroll stretch.
///
/// HWUI's stretch shader (`frameworks/base`
/// `libs/hwui/effects/StretchEffect.cpp`,
/// `stretchShader` with the uniforms `StretchEffect::getShader` derives),
/// the `StretchEffectBehavior::ShaderHWUI` rendering of a stretched
/// `RenderNode`.
///
/// The warp runs in a frame: `frame` is the affine map
/// `[a, b, c, d, e, f]` (`x' = a·x + c·y + e`, `y' = b·x + d·y + f`) from the
/// frame's own space to input pixels, and must be invertible. The frame's
/// own space spans `[0, size[0]] × [0, size[1]]`, the stretched node's
/// bounds, which HWUI's shader evaluates in. `stretch` is HWUI's merged
/// stretch direction per axis: positive pulls the left or top edge,
/// negative the right or bottom one, and zero leaves the axis alone.
///
/// Per axis of extent `D` and stretch `s`, an output position `p` inside
/// the frame samples the input at `computeOverscroll(p)`:
///
/// - `s > 0`: `D / (1 + s) − (D − p) / (1 + s·mix(1, (D − p) / D, 0.7))`;
/// - `s < 0`: `D − (D / (1 − s) − p / (1 − s·mix(1, p / D, 0.7)))`.
///
/// The visible content is the first (or last) `D / (1 + |s|)` of the
/// frame, stretched over all of it, and stretched most at the pulled edge:
/// `0.7` is [`STRETCH_INTERPOLATION_STRENGTH`]. Positions outside the
/// frame pass through unchanged — HWUI draws a stretched node only inside
/// its bounds. Samples are bilinear and stay half a pixel inside the frame
/// along each of its axes: the shader's `SkTileMode::kClamp`,
/// `SkFilterMode::kLinear` content sampler reads a snapshot of the node, so
/// the pulled edge repeats the node's edge row rather than blending what
/// lies beyond the bounds.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "distortion/overscroll_stretch.wgsl",
    footprint_fn = crate::filters::footprint::overscroll_stretch,
    constants = [STRETCH_INTERPOLATION_STRENGTH]
)]
pub struct OverscrollStretch<T> {
    /// The frame's own space to input pixels, `[a, b, c, d, e, f]`.
    pub frame: [T; 6],
    /// The frame's width and height in its own space.
    pub size: [T; 2],
    /// The stretch direction per axis, HWUI's `mStretchDirection`.
    pub stretch: [T; 2],
}
