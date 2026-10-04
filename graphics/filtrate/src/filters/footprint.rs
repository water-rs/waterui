//! Footprints of the built-in spatial filters.
//!
//! Each is non-decreasing in its parameters' magnitudes, as
//! [`SpatialFilter::footprint_of`](crate::SpatialFilter::footprint_of)
//! requires, so an executor can bound an animation by evaluating it at the
//! parameters' largest magnitudes.

use crate::Footprint;

/// A radius the stage rounds to whole pixels and clamps at zero.
pub const fn rounded(radius: f32) -> f32 {
    radius.round().max(0.0)
}

/// A radius the stage rounds to whole pixels and clamps at one.
pub const fn rounded_at_least_one(radius: f32) -> f32 {
    radius.round().max(1.0)
}

/// A pixel offset read through a nearest sampler: the texel reached is at
/// most the offset rounded up.
pub const fn offset(pixels: f32) -> f32 {
    pixels.abs().ceil()
}

/// [`Pixellate`](super::Pixellate): the cell centre is at most half a cell
/// away.
pub fn pixellate(params: [f32; 1]) -> Footprint {
    Footprint::pixels((params[0].max(1.0) * 0.5).ceil())
}

/// [`Crystallize`](super::Crystallize): the chosen seed lies in one of the
/// 3x3 neighbouring cells, jittered by at most 0.4 cells, so at most 1.9
/// cells away on either axis.
pub fn crystallize(params: [f32; 1]) -> Footprint {
    Footprint::pixels((params[0].max(1.0) * 1.9).ceil())
}

/// [`MotionBlur`](super::MotionBlur): `radius` taps each way, each filtered
/// bilinearly, so one texel further.
pub fn motion_blur(params: [f32; 2]) -> Footprint {
    let radius = rounded(params[0]);
    Footprint::pixels(if radius > 0.0 { radius + 1.0 } else { 0.0 })
}

/// [`ZoomBlur`](super::ZoomBlur): taps along `uv + (center − uv)·amount·t`
/// reach `amount·|center − uv|`, worst at the image corner farthest from
/// `center` — which the shader never clamps, so an out-of-range centre
/// reaches further still. Bilinear taps: one texel further.
pub fn zoom_blur(params: [f32; 3]) -> Footprint {
    // Below the stage's amount threshold it reads only its own texel.
    if params[0] <= 0.0001 {
        return Footprint::ZERO;
    }
    // The params arrive as magnitudes, so a centre bound `m` stands for a
    // centre anywhere in `[-m, m]²` — the farthest image corner is at most
    // `m + 1` away on either axis (a centre at `−m`, corner `1`).
    let farthest_corner = (params[1].abs() + 1.0).hypot(params[2].abs() + 1.0);
    Footprint::new(1.0, params[0].max(0.0) * farthest_corner)
}

/// [`EdgeWork`](super::EdgeWork): the gradient taps at `radius`, at least
/// one.
pub const fn edge_work(params: [f32; 2]) -> Footprint {
    Footprint::pixels(rounded_at_least_one(params[0]))
}

/// The circular distortions — twirl and vortex — keep their samples inside
/// the radius ball around the centre, so a sample moves at most two radii in
/// isotropic units (1.0 = the shorter edge), and the filtered sampler
/// reaches one texel further.
pub fn radial(params: [f32; 4]) -> Footprint {
    Footprint::new(1.0, 2.0 * params[2].max(0.001))
}

/// [`PinchDistortion`](super::PinchDistortion): `pow(t, 1 + scale)` keeps
/// the sample inside the radius ball while `scale >= -1`; below that the
/// factor diverges toward the centre, and any texel can be reached.
pub fn pinch(params: [f32; 4]) -> Footprint {
    Footprint::new(1.0, bounded_extent(params[2], params[3]))
}

/// [`BumpDistortion`](super::BumpDistortion): `1 + scale·t²` stays positive
/// inside the ball while `scale >= -1`; below that it crosses zero, the
/// displacement diverges and any texel can be reached.
pub fn bump(params: [f32; 4]) -> Footprint {
    Footprint::new(1.0, bounded_extent(params[2], params[3]))
}

/// The extent a circular distortion reaches — the whole image when the
/// displacement can leave the radius ball, two radii otherwise.
fn bounded_extent(radius: f32, scale: f32) -> f32 {
    // `scale` arrives as a magnitude: the shader's scale can be as low as
    // `−scale`, so a magnitude past 1 allows a divergent scale below −1.
    if scale.abs() > 1.0 {
        1.0
    } else {
        2.0 * radius.max(0.001)
    }
}
