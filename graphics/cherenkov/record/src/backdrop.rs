//! Backdrop sampling attached to layers: a [`BackdropSample`] names the
//! group a layer samples.
//!
//! An optional [`BackdropEffect`] is evaluated inside the member's
//! composite against the group's shared filtered capture. Effects work in
//! the extended linear Display P3 working space and are never clamped.

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
}

impl BackdropSample {
    /// A plain sample of `group`, with no per-member effect.
    #[must_use]
    pub const fn new(group: BackdropId) -> Self {
        Self {
            group,
            effect: None,
            outer: BackdropOuter::ZERO,
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
            group: self.group,
            effect: self.effect,
            outer: extent,
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
}
