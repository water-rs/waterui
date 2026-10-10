//! Backdrop sampling attached to layers: a [`BackdropSample`] names the
//! group a layer samples.
//!
//! An optional [`BackdropEffect`] is evaluated inside the member's
//! composite against the group's shared filtered capture. Effects work in
//! the extended linear Display P3 working space and are never clamped.
//!
//! The engine-independent parameters of a group and of a custom effect —
//! the [`CaptureScale`] and [`CaptureLevels`] a group captures at, the
//! [`BackdropSpec`] it is created with, and the [`BackdropShaderSource`]
//! a backdrop shader compiles from — live here too, so a widget theme
//! can declare them in a [`MaterialRegistry`](crate::MaterialRegistry)
//! without an engine.

use crate::union_cap;

use std::borrow::Cow;

use crate::BackdropShaderId;
use crate::ops::BackdropId;

/// A per-member effect evaluated in the member's composite against the
/// group's shared filtered capture. Extended linear P3, no clamping.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BackdropEffect {
    /// A 3×4 premultiplied RGBA matrix in filtrate's `ColorMatrix` layout:
    /// row-major, three rows of `[r, g, b, bias]` applied as
    /// `dot(row, c)` to the premultiplied sample (the bias column thus
    /// scales with alpha); alpha passes through.
    Color(ColorMatrix),
    /// Displaces the sample point along the member's inward edge normal:
    /// `q = p - n · strength · t²` with `t = clamp(1 + d/depth, 0, 1)`,
    /// `d` the signed device-space distance to the clip edge.
    Refraction(Refraction),
    /// A highlight inside the clip's rim: `c = sample(p)`,
    /// `t = clamp(1 + d / width, 0, 1)` and
    /// `c.rgb += color.rgb · color.a · gain · t²`, alpha unchanged.
    Rim(Rim),
    /// Samples the capture pyramid at a blur level ramping from `edge`
    /// at the clip's edge to `interior` deep inside it
    /// (`backdrop_sample_level`).
    Level(LevelRamp),
    /// A registered backdrop shader with its uniforms (declared order).
    Shader(BackdropShaderEffect),
}

impl From<ColorMatrix> for BackdropEffect {
    fn from(matrix: ColorMatrix) -> Self {
        Self::Color(matrix)
    }
}

impl From<Refraction> for BackdropEffect {
    fn from(refraction: Refraction) -> Self {
        Self::Refraction(refraction)
    }
}

impl From<Rim> for BackdropEffect {
    fn from(rim: Rim) -> Self {
        Self::Rim(rim)
    }
}

impl From<LevelRamp> for BackdropEffect {
    fn from(ramp: LevelRamp) -> Self {
        Self::Level(ramp)
    }
}

impl From<BackdropShaderEffect> for BackdropEffect {
    fn from(shader: BackdropShaderEffect) -> Self {
        Self::Shader(shader)
    }
}

/// A 3×4 premultiplied colour matrix (filtrate `ColorMatrix` layout).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColorMatrix(pub [f32; 12]);

/// Displaces the backdrop sample point along the member's edge normal.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Refraction {
    /// The rim depth over which the displacement decays, in device
    /// pixels; must be positive.
    pub depth: f32,
    /// The maximum displacement at the edge, in device pixels; must be
    /// non-negative.
    pub strength: f32,
}

/// A highlight inside the member clip's rim, additive on the sample.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rim {
    /// The rim's width inside the clip edge, in device pixels; must be
    /// positive.
    pub width: f32,
    /// The highlight colour, straight-alpha linear Display P3 (the
    /// working space); must be finite.
    pub color: [f32; 4],
    /// The highlight's gain, finite; values above 1 push the rim above
    /// SDR white.
    pub gain: f32,
}

/// A blur-level ramp over the member clip's signed distance.
///
/// `t = clamp(1 + d / depth, 0, 1)` (the same `t` as [`Refraction`] and
/// [`Rim`]) and `level = interior + (edge − interior)·t`, the member's
/// composite reading `backdrop_sample_level(p, level)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "LevelRampFields"))]
pub struct LevelRamp {
    depth: f32,
    edge: f32,
    interior: f32,
}

/// Why a [`LevelRamp`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LevelRampError {
    /// The depth is not a finite positive number.
    #[error("the level ramp depth must be finite and above 0")]
    Depth,
    /// The edge or interior level is NaN or infinite.
    #[error("the level ramp's edge and interior levels must be finite")]
    NonFiniteLevel,
}

impl LevelRamp {
    /// A ramp from level `edge` at the clip's edge to level `interior`
    /// deep inside it, decaying over `depth` device pixels.
    ///
    /// # Errors
    /// [`LevelRampError::Depth`] unless `depth` is finite and above 0,
    /// [`LevelRampError::NonFiniteLevel`] unless `edge` and `interior`
    /// are finite.
    pub fn new(depth: f32, edge: f32, interior: f32) -> Result<Self, LevelRampError> {
        if !(depth.is_finite() && depth > 0.0) {
            return Err(LevelRampError::Depth);
        }
        if !(edge.is_finite() && interior.is_finite()) {
            return Err(LevelRampError::NonFiniteLevel);
        }
        Ok(Self {
            depth,
            edge,
            interior,
        })
    }

    /// The rim depth the level decays over inside the clip edge, in
    /// device pixels; finite and positive.
    #[must_use]
    pub const fn depth(self) -> f32 {
        self.depth
    }

    /// The pyramid level at the clip's edge (`t = 1`); finite.
    #[must_use]
    pub const fn edge(self) -> f32 {
        self.edge
    }

    /// The pyramid level deep inside the clip (`t = 0`); finite.
    #[must_use]
    pub const fn interior(self) -> f32 {
        self.interior
    }
}

/// A [`LevelRamp`]'s serialized fields, validated into the ramp.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct LevelRampFields {
    depth: f32,
    edge: f32,
    interior: f32,
}

#[cfg(feature = "serde")]
impl TryFrom<LevelRampFields> for LevelRamp {
    type Error = LevelRampError;

    fn try_from(fields: LevelRampFields) -> Result<Self, Self::Error> {
        Self::new(fields.depth, fields.edge, fields.interior)
    }
}

/// A registered backdrop shader with its uniforms, in declared order.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BackdropShaderEffect {
    /// The shader, as registered with the consumer.
    pub shader: BackdropShaderId,
    /// Uniform values, packed four per `vec4` and zero-filled; at most 64.
    pub uniforms: Vec<f32>,
    /// The registered source's reach in device pixels.
    #[cfg_attr(feature = "serde", serde(skip))]
    reach: f32,
}

impl BackdropShaderEffect {
    /// A shader effect: `shader` is the registered shader's id, `uniforms`
    /// its uniform values in declared order, and `reach` the registered
    /// source's sampling reach in device pixels.
    #[must_use]
    pub const fn new(shader: BackdropShaderId, uniforms: Vec<f32>, reach: f32) -> Self {
        Self {
            shader,
            uniforms,
            reach,
        }
    }
}

impl BackdropEffect {
    /// The effect's sampling reach in device pixels: how far beyond the
    /// member's bounds its samples can land. `Color`, `Rim` and `Level`
    /// read the member pixel only; `Refraction` reaches `strength`; a
    /// shader reaches its registered `reach`.
    #[must_use]
    pub const fn reach(&self) -> f32 {
        match self {
            Self::Color(_) | Self::Rim(_) | Self::Level(_) => 0.0,
            Self::Refraction(r) => r.strength,
            Self::Shader(s) => s.reach,
        }
    }
}

/// A member composite's outer extent in device pixels: finite and
/// non-negative. [`BackdropSample::outer`] draws the member's field a
/// band this wide past its clip edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BackdropOuter(f32);

/// Why a [`BackdropOuter`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackdropOuterError {
    /// The extent is NaN or infinite.
    #[error("the outer extent is not finite")]
    NonFinite,
    /// The extent is below 0.
    #[error("the outer extent must not be negative")]
    Negative,
}

impl BackdropOuter {
    /// No outer extent: the composite keeps the clip's coverage.
    pub const ZERO: Self = Self(0.0);

    /// An outer extent of `px` device pixels.
    ///
    /// # Errors
    /// [`BackdropOuterError::NonFinite`] when `px` is NaN or infinite,
    /// [`BackdropOuterError::Negative`] when it is below 0.
    pub fn new(px: f32) -> Result<Self, BackdropOuterError> {
        if !px.is_finite() {
            return Err(BackdropOuterError::NonFinite);
        }
        if px < 0.0 {
            return Err(BackdropOuterError::Negative);
        }
        Ok(Self(px))
    }

    /// The extent in device pixels; finite and non-negative.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// The device pixels one logical pixel of a member's recording spans:
/// finite and above 0.
///
/// [`BackdropSample::scale`] hands it to the member's backdrop shader as
/// `px.scale`, so a recipe written in logical pixels converts its
/// lengths without recovering the scale from the member's geometry.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct RecordingScale(f32);

/// Why a [`RecordingScale`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecordingScaleError {
    /// The scale is NaN or infinite.
    #[error("the recording scale is not finite")]
    NonFinite,
    /// The scale is not above 0.
    #[error("the recording scale must be above 0")]
    OutOfRange,
}

impl RecordingScale {
    /// One device pixel per logical pixel: a recording in device pixels.
    pub const ONE: Self = Self(1.0);

    /// A recording whose logical pixel spans `scale` device pixels.
    ///
    /// # Errors
    /// [`RecordingScaleError::NonFinite`] when `scale` is NaN or
    /// infinite, [`RecordingScaleError::OutOfRange`] unless it is above 0.
    pub fn new(scale: f32) -> Result<Self, RecordingScaleError> {
        if !scale.is_finite() {
            return Err(RecordingScaleError::NonFinite);
        }
        if scale <= 0.0 {
            return Err(RecordingScaleError::OutOfRange);
        }
        Ok(Self(scale))
    }

    /// The device pixels per logical pixel; finite and above 0.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl Default for RecordingScale {
    fn default() -> Self {
        Self::ONE
    }
}

/// A layer's backdrop sample: the group it samples and an optional
/// per-member effect evaluated in the member's composite against the
/// shared filtered capture.
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropSample {
    /// The sampled group.
    group: BackdropId,
    /// The per-member effect applied in the member's composite.
    effect: Option<BackdropEffect>,
    /// How far the member's own field extends its composite past the
    /// clip edge, in device pixels.
    outer: BackdropOuter,
    /// The device pixels per logical pixel of the member's recording.
    recording_scale: RecordingScale,
}

impl BackdropSample {
    /// A plain sample of `group`, with no per-member effect.
    #[must_use]
    pub const fn new(group: BackdropId) -> Self {
        Self {
            group,
            effect: None,
            outer: BackdropOuter::ZERO,
            recording_scale: RecordingScale::ONE,
        }
    }

    /// A sample of `group` with a per-member `effect`, evaluated in the
    /// member's composite against the shared filtered capture.
    #[must_use]
    pub fn with_effect(group: BackdropId, effect: impl Into<BackdropEffect>) -> Self {
        Self {
            group,
            effect: Some(effect.into()),
            outer: BackdropOuter::ZERO,
            recording_scale: RecordingScale::ONE,
        }
    }

    /// An outer extent of `extent`: the member's composite covers where
    /// its field is below it, not only where the clip covers — a band
    /// that wide beyond the clip edge, antialiased from the field.
    /// [`BackdropOuter::ZERO`] keeps today's coverage on a standalone
    /// member; under a union the member's coverage is its ownership
    /// weight times `field < outer` either way.
    #[must_use]
    pub fn outer(self, extent: BackdropOuter) -> Self {
        Self {
            outer: extent,
            ..self
        }
    }

    /// The device pixels one logical pixel of the member's recording
    /// spans, read by its backdrop shader as `px.scale`
    /// ([`BackdropShaderSource`]). [`RecordingScale::ONE`] — the
    /// default — states a recording in device pixels. Built-in effects
    /// take device-pixel parameters and never read it.
    #[must_use]
    pub fn scale(self, scale: RecordingScale) -> Self {
        Self {
            recording_scale: scale,
            ..self
        }
    }

    /// The sampled group.
    #[must_use]
    pub const fn group(&self) -> BackdropId {
        self.group
    }

    /// The per-member effect, when the sample was made with
    /// [`BackdropSample::with_effect`].
    #[must_use]
    pub const fn effect(&self) -> Option<&BackdropEffect> {
        self.effect.as_ref()
    }

    /// The member's outer extent ([`BackdropOuter::ZERO`] unless
    /// [`BackdropSample::outer`] set it).
    #[must_use]
    pub const fn outer_extent(&self) -> BackdropOuter {
        self.outer
    }

    /// The member's recording scale ([`RecordingScale::ONE`] unless
    /// [`BackdropSample::scale`] set it).
    #[must_use]
    pub const fn recording_scale(&self) -> RecordingScale {
        self.recording_scale
    }
}

nami_core::impl_constant!(BackdropSample);

/// The resolution a backdrop group captures at, as a fraction `s` of
/// device resolution, `0 < s ≤ 1`; fixed when the group is created
/// (the engine's `Surface::backdrop_group`).
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

/// How strongly a backdrop union field blends the group's member shapes
/// at their shared edge, in device pixels.
///
/// A group created with a union ([`BackdropSpec::union`]) replaces every
/// member's own clip field with one shared field for its composite: the
/// quadratic smooth minimum of all member distances, folded in ascending
/// order with the smoothing distance `k` — between two members the
/// smin blends over a band `k` wide centred on their boundary. The
/// member's own distance stays available to effects as `own_sdf` (see
/// [`BackdropShaderSource`]).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct BackdropUnion(f32);

/// Why a [`BackdropUnion`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackdropUnionError {
    /// The smoothing distance is NaN or infinite.
    #[error("the union smoothing is not finite")]
    NonFinite,
    /// The smoothing distance is not above 0.
    #[error("the union smoothing must be above 0")]
    OutOfRange,
}

impl BackdropUnion {
    /// The most members a union group may carry.
    ///
    /// Groups larger than this are rejected with the
    /// `backdrop-union-members` `Unsupported` error on every
    /// implementation. The single definition lives in
    /// `union_cap::UNION_MAX_MEMBERS`, which the gpu build script and
    /// the oracle read directly — neither depends on this crate.
    pub const MAX_MEMBERS: u32 = union_cap::UNION_MAX_MEMBERS;

    /// A union field smoothed over `smoothing` device pixels.
    ///
    /// # Errors
    /// [`BackdropUnionError::NonFinite`] when `smoothing` is NaN or
    /// infinite, [`BackdropUnionError::OutOfRange`] unless it is above 0.
    pub fn new(smoothing: f32) -> Result<Self, BackdropUnionError> {
        if !smoothing.is_finite() {
            return Err(BackdropUnionError::NonFinite);
        }
        if smoothing <= 0.0 {
            return Err(BackdropUnionError::OutOfRange);
        }
        Ok(Self(smoothing))
    }

    /// The smoothing distance `k`, in device pixels.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }

    /// How far below the smallest member distance the fold can draw the
    /// union field with `members` members: `r(1) = 0` and
    /// `r(i + 1) = r(i) + (k − r(i))² / (4k)`, so `r(n) < k`. Draw
    /// bounds and capture footprints pad every member by `r(n)` plus
    /// its `outer` extent.
    ///
    /// `members` above [`MAX_MEMBERS`](Self::MAX_MEMBERS) cannot occur in
    /// a lowered group (the cap is enforced at planning time); it still
    /// returns a value so callers stay total.
    #[must_use]
    pub fn inflation(self, members: usize) -> f64 {
        let k = f64::from(self.0);
        let mut r = 0.0;
        for _ in 1..members {
            let d = k - r;
            r += d * d / (4.0 * k);
        }
        r
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
/// the single bilinear capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CaptureLevels(u32);

/// Why a [`CaptureLevels`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureLevelsError {
    /// The count is not in `1..=CaptureLevels::MAX`.
    #[error("the capture level count must be between 1 and {}", CaptureLevels::MAX)]
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

/// How a backdrop group captures and combines its members.
///
/// Used by the engine's `Surface::backdrop_group` and
/// `Surface::backdrop_group_unfiltered`.
///
/// The spec fixes the group's capture [`scale`](BackdropSpec::scale), how
/// many capture [`levels`](BackdropSpec::levels) the group's pyramid keeps
/// and the group's [`anchor`](BackdropSpec::anchor) layer when the capture
/// is taken away from the first member; it is the parameter object the
/// group is created with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropSpec {
    scale: CaptureScale,
    levels: CaptureLevels,
    anchor: Option<crate::LayerId>,
    union: Option<BackdropUnion>,
}

impl BackdropSpec {
    /// The 1:1, single-level capture.
    pub const FULL: Self = Self::new(CaptureScale::FULL, CaptureLevels::ONE);

    /// A group captured at `scale`, keeping `levels` capture levels.
    #[must_use]
    pub const fn new(scale: CaptureScale, levels: CaptureLevels) -> Self {
        Self {
            scale,
            levels,
            anchor: None,
            union: None,
        }
    }

    /// Anchors the group's capture at `layer`'s paint-order position, before
    /// the anchor's own content and its children: the group's capture reads
    /// the anchor's compositing canvas as it stands at the anchor's paint
    /// position — one capture per anchored group — so anchored members
    /// never see each other or content painted after the anchor. A member
    /// can be any layer painting after the anchor in the anchor's
    /// compositing canvas, or the frame fails instead of falling back to
    /// the first-member rule.
    ///
    /// A spec without an anchor keeps the first-member rule: the capture is
    /// taken where the group's first member paints.
    #[must_use]
    pub const fn anchor(mut self, layer: crate::LayerId) -> Self {
        self.anchor = Some(layer);
        self
    }

    /// A group whose members composite against the shared union field
    /// (see [`BackdropUnion`]).
    #[must_use]
    pub const fn union(self, union: BackdropUnion) -> Self {
        Self {
            union: Some(union),
            ..self
        }
    }

    /// The capture scale `s`.
    #[must_use]
    pub const fn scale(self) -> CaptureScale {
        self.scale
    }

    /// The level count `n`: how many capture levels the pyramid keeps.
    #[must_use]
    pub const fn levels(self) -> CaptureLevels {
        self.levels
    }

    /// The anchor layer, when the group is anchored; `None` under the
    /// first-member rule.
    #[must_use]
    pub const fn anchor_layer(self) -> Option<crate::LayerId> {
        self.anchor
    }

    /// The group's union field, when one was declared.
    #[must_use]
    pub const fn union_field(self) -> Option<BackdropUnion> {
        self.union
    }
}

impl From<CaptureScale> for BackdropSpec {
    fn from(scale: CaptureScale) -> Self {
        Self::new(scale, CaptureLevels::ONE)
    }
}

/// A WGSL shader compiled for the backdrop composite contract (the
/// engine's `Engine::backdrop_shader`), not a shader paint.
///
/// The source defines
///
/// ```wgsl
/// struct BackdropPixel {
///     p: vec2<f32>, sdf: f32, normal: vec2<f32>,
///     own_sdf: f32, size: vec2<f32>, scale: f32,
/// }
/// fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32>
/// ```
///
/// where `px.p` is the member pixel centre in device space, `px.sdf` the
/// signed distance to the field's edge (negative inside) — the group's
/// union field when it has one ([`BackdropUnion`]), else the member's
/// own clip field — `px.normal` that field's unit outward normal,
/// `px.own_sdf` the signed distance to the member's own clip edge,
/// `px.size` the member's device bounds size, `px.scale` the device
/// pixels per logical pixel of the member's recording
/// ([`BackdropSample::scale`]), and `params` the effect
/// uniforms packed four per `vec4`, zero-filled. Without a union,
/// `px.sdf == px.own_sdf` and `px.normal` is the member's own normal.
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
    use super::{
        BackdropSpec, BackdropUnion, BackdropUnionError, CaptureLevels, CaptureLevelsError,
        CaptureScale, CaptureScaleError, RecordingScale, RecordingScaleError,
    };

    #[test]
    fn recording_scale_is_finite_and_positive() {
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                RecordingScale::new(scale),
                Err(RecordingScaleError::NonFinite)
            );
        }
        for scale in [0.0, -0.0, -2.0] {
            assert_eq!(
                RecordingScale::new(scale),
                Err(RecordingScaleError::OutOfRange)
            );
        }
        assert_eq!(RecordingScale::new(1.0), Ok(RecordingScale::ONE));
        assert_eq!(RecordingScale::default(), RecordingScale::ONE);
        assert!((RecordingScale::new(3.0).expect("in range").get() - 3.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn backdrop_union_is_finite_and_positive() {
        for k in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(BackdropUnion::new(k), Err(BackdropUnionError::NonFinite));
        }
        for k in [0.0, -0.0, -1.0] {
            assert_eq!(BackdropUnion::new(k), Err(BackdropUnionError::OutOfRange));
        }
        let union = BackdropUnion::new(12.0).expect("in range");
        assert!((union.get() - 12.0).abs() <= f32::EPSILON);
        let spec = BackdropSpec::FULL.union(union);
        assert_eq!(spec.union_field(), Some(union));
        assert_eq!(BackdropSpec::FULL.union_field(), None);
    }

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
        assert_eq!(
            CaptureLevels::new(CaptureLevels::MAX).unwrap().get(),
            CaptureLevels::MAX
        );
        assert_eq!(
            CaptureLevelsError::OutOfRange.to_string(),
            format!(
                "the capture level count must be between 1 and {}",
                CaptureLevels::MAX
            )
        );
    }

    #[test]
    fn backdrop_spec_defaults_to_one_level() {
        let quarter = CaptureScale::new(0.25).expect("in range");
        let spec = BackdropSpec::from(quarter);
        assert_eq!(spec.scale(), quarter);
        assert_eq!(spec.levels(), CaptureLevels::ONE);
        let spec = BackdropSpec::new(quarter, CaptureLevels::new(4).expect("in range"));
        assert_eq!(spec.scale(), quarter);
        assert_eq!(spec.levels().get(), 4);
        assert!(BackdropSpec::FULL.scale().is_full());
        assert_eq!(BackdropSpec::FULL.levels(), CaptureLevels::ONE);
    }
}
