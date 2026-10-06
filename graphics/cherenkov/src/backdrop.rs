//! Backdrop sampling: the per-member effect types a layer edit carries
//! live in `cherenkov-record` (a layer edit names them without the
//! engine); the engine-side shader *source* — the WGSL the render loop
//! compiles — stays here.
//!
//! A [`BackdropSample`] names the group a layer samples; an optional
//! [`BackdropEffect`] is evaluated inside the member's composite against
//! the group's shared filtered capture. Effects work in the extended
//! linear Display P3 working space and are never clamped.

use std::borrow::Cow;

/// The resolution a backdrop group captures at, as a fraction `s` of
/// device resolution, `0 < s ≤ 1`; fixed when the group is created
/// ([`Surface::backdrop_group`](crate::Surface::backdrop_group)).
///
/// The capture grid is anchored at the device origin: capture texel
/// `(i, j)` covers the device rect `[i/s, (i+1)/s) × [j/s, (j+1)/s)` and
/// holds the area-weighted mean of the backdrop over that rect's part
/// inside the surface, so a region of `w × h` device pixels resolves into
/// about `⌈w·s⌉ × ⌈h·s⌉` texels. The group's filter chain runs on the
/// capture, its footprint counted in capture texels — a device-pixel apron
/// of footprint ÷ `s` — and members sample it bilinearly at `p · s` for a
/// device point `p`. [`CaptureScale::FULL`] is the 1:1 capture.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct CaptureScale(f32);

/// Why a [`CaptureScale`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureScaleError {
    /// The scale is NaN or infinite.
    #[error("the capture scale is not finite")]
    NonFinite,
    /// The scale is not in `(0, 1]`.
    #[error("the capture scale must be above 0 and at most 1")]
    OutOfRange,
}

impl CaptureScale {
    /// Device resolution: the capture copies the backdrop 1:1.
    pub const FULL: Self = Self(1.0);

    /// A capture at `scale` times device resolution.
    ///
    /// # Errors
    /// [`CaptureScaleError::NonFinite`] when `scale` is NaN or infinite,
    /// [`CaptureScaleError::OutOfRange`] unless `0 < scale ≤ 1`.
    pub fn new(scale: f32) -> Result<Self, CaptureScaleError> {
        if !scale.is_finite() {
            return Err(CaptureScaleError::NonFinite);
        }
        if scale <= 0.0 || scale > 1.0 {
            return Err(CaptureScaleError::OutOfRange);
        }
        Ok(Self(scale))
    }

    /// The scale `s`.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }

    /// Whether this is [`CaptureScale::FULL`]: the capture is a 1:1 copy.
    #[must_use]
    pub fn is_full(self) -> bool {
        // The constructor bounds the scale above by 1.
        self.0 >= 1.0
    }
}

/// The number of capture levels a backdrop group keeps, between 1 and
/// [`CaptureLevels::MAX`]; `n` is fixed when the group is created.
///
/// Level 0 is the filtered capture itself. A group with `n > 1` reduces
/// that level into `n − 1` progressively half-sized levels: level `k`
/// texel `(i, j)` is the mean of level `k − 1` texels
/// `(2i..=2i+1, 2j..=2j+1)` — an exact 2×2 box, partial boxes at the grid
/// edge averaging the texels present. Level `k` texel `(i, j)` covers
/// capture texels `[2^k·i, 2^k·(i+1)) × [2^k·j, 2^k·(j+1))` on the same
/// device-anchored grid, so a member's motion never shifts any level's
/// grid. Members read the pyramid with `backdrop_sample_level`
/// ([`BackdropShaderSource`]) or a `Level` member effect; one level is
/// today's single bilinear capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CaptureLevels(u32);

/// Why a [`CaptureLevels`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureLevelsError {
    /// The count is not in `1..=CaptureLevels::MAX`.
    #[error("the capture level count must be between 1 and 8")]
    OutOfRange,
}

impl CaptureLevels {
    /// One level: the plain filtered capture only.
    pub const ONE: Self = Self(1);
    /// The most levels a group may keep.
    pub const MAX: u32 = 8;

    /// A group keeping `n` capture levels.
    ///
    /// # Errors
    /// [`CaptureLevelsError::OutOfRange`] unless `n` is between 1 and
    /// [`CaptureLevels::MAX`].
    pub const fn new(n: u32) -> Result<Self, CaptureLevelsError> {
        if n == 0 || n > Self::MAX {
            return Err(CaptureLevelsError::OutOfRange);
        }
        Ok(Self(n))
    }

    /// The level count `n`.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// How a backdrop group captures and combines its members
/// ([`Surface::backdrop_group`], [`Surface::backdrop_group_unfiltered`]).
///
/// The spec fixes the group's capture [`scale`](BackdropSpec::scale) and
/// how many capture [`levels`](BackdropSpec::levels) the group's pyramid
/// keeps. It is the parameter object the group is created with; a later
/// change adds a member-union option to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropSpec {
    scale: CaptureScale,
    /// The group's level count `n`: how many capture levels the pyramid
    /// keeps (`1..=CaptureLevels::MAX`).
    pub levels: CaptureLevels,
}

impl BackdropSpec {
    /// The 1:1, single-level capture.
    pub const FULL: Self = Self {
        scale: CaptureScale::FULL,
        levels: CaptureLevels::ONE,
    };

    /// A group captured at `scale`, keeping one level.
    #[must_use]
    pub const fn new(scale: CaptureScale) -> Self {
        Self {
            scale,
            levels: CaptureLevels::ONE,
        }
    }

    /// Keeps `levels` capture levels on the group's pyramid.
    #[must_use]
    pub const fn levels(self, levels: CaptureLevels) -> Self {
        Self {
            scale: self.scale,
            levels,
        }
    }

    /// The capture scale `s`.
    #[must_use]
    pub const fn scale(self) -> CaptureScale {
        self.scale
    }
}

impl From<CaptureScale> for BackdropSpec {
    fn from(scale: CaptureScale) -> Self {
        Self::new(scale)
    }
}

/// A WGSL shader compiled for the backdrop composite contract
/// ([`Engine::backdrop_shader`](crate::Engine::backdrop_shader)), not a
/// shader paint.
///
/// The source defines
///
/// ```wgsl
/// fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>,
///                    size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32>
/// ```
///
/// where `p` is the member pixel centre in device space, `sdf` the signed
/// distance to the member's clip edge (negative inside), `normal` the
/// unit outward normal, `size` the member's device bounds size, and
/// `params` the effect uniforms packed four per `vec4`, zero-filled.
/// `fn backdrop_sample(q: vec2<f32>) -> vec4<f32>` bilinearly samples the
/// filtered capture at device point `q` — at `q · s` on a group's capture
/// grid ([`CaptureScale`]) — clamped to its region. For a group keeping
/// `n` levels ([`BackdropSpec::levels`]),
/// `fn backdrop_sample_level(q: vec2<f32>, level: f32) -> vec4<f32>`
/// trilinearly samples the pyramid: `level` clamps to `[0, n − 1]` and
/// the read is bilinear at `q · s / 2^k` on `k = floor(level)` and
/// `k = ceil(level)`, mixed by `fract(level)`; `backdrop_sample` is the
/// level-0 read. The return value is premultiplied and written unclamped.
#[derive(Clone, Debug)]
pub struct BackdropShaderSource {
    /// The fragment source, without the backend's prelude.
    pub source: Cow<'static, str>,
    /// The maximum displacement `backdrop_sample` coordinates can take,
    /// in device pixels; non-negative and finite. The group's capture
    /// region grows by this much around the member so displaced samples
    /// stay inside it.
    pub reach: f32,
}

impl BackdropShaderSource {
    /// A backdrop effect shader from WGSL.
    pub fn wgsl(source: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: source.into(),
            reach: 0.0,
        }
    }

    /// Sets the reach (see [`BackdropShaderSource::reach`]).
    #[must_use]
    pub fn reach(self, px: f32) -> Self {
        Self {
            source: self.source,
            reach: px,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BackdropSpec, CaptureLevels, CaptureLevelsError, CaptureScale, CaptureScaleError};

    #[test]
    fn capture_scale_is_finite_and_in_the_unit_interval() {
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(CaptureScale::new(scale), Err(CaptureScaleError::NonFinite));
        }
        for scale in [0.0, -0.0, -0.25, 1.0 + f32::EPSILON, 2.0] {
            assert_eq!(CaptureScale::new(scale), Err(CaptureScaleError::OutOfRange));
        }
        assert_eq!(CaptureScale::new(1.0), Ok(CaptureScale::FULL));
        assert!(CaptureScale::FULL.is_full());
        let quarter = CaptureScale::new(0.25).expect("in range");
        assert!(!quarter.is_full());
        assert!((quarter.get() - 0.25).abs() <= f32::EPSILON);
    }

    #[test]
    fn capture_levels_is_within_one_and_max() {
        for n in [0, CaptureLevels::MAX + 1, u32::MAX] {
            assert_eq!(CaptureLevels::new(n), Err(CaptureLevelsError::OutOfRange));
        }
        assert_eq!(CaptureLevels::new(1), Ok(CaptureLevels::ONE));
        assert_eq!(CaptureLevels::new(CaptureLevels::MAX).unwrap().get(), 8);
    }

    #[test]
    fn backdrop_spec_defaults_to_one_level() {
        let quarter = CaptureScale::new(0.25).expect("in range");
        let spec = BackdropSpec::from(quarter);
        assert_eq!(spec.scale(), quarter);
        assert_eq!(spec.levels, CaptureLevels::ONE);
        let spec = spec.levels(CaptureLevels::new(4).expect("in range"));
        assert_eq!(spec.levels.get(), 4);
        assert!(BackdropSpec::FULL.scale().is_full());
    }
}
