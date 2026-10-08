use serde::{Deserialize, Serialize};

/// A backdrop group: one backdrop capture shared by every layer that
/// references its `id` ([`crate::Layer::backdrop`]), plus the filter chain
/// applied to that capture once.
///
/// The group's capture is taken at the moment its first member (in paint
/// order) begins, over the content already drawn into the member's
/// compositing canvas; `filters` then run over the capture and every member
/// composites the filtered result as the bottom-most content inside its own
/// clip. A member layer must have a clip ([`crate::Scene::load`] validates
/// this) or the scene fails to load.
///
/// The member's compositing canvas is the nearest enclosing level a
/// capture cannot look through: a layer isolated for a
/// [`crate::Layer::filter`] or a non-Normal [`crate::Layer::blend`], a
/// projective layer's local image ([`crate::Layer::projection`]), else
/// the surface. A capture looks through every other kind of level —
/// pass-through and `opacity < 1` layers alike — compositing each one's
/// partial contents at full opacity, so a member under a translucent
/// ancestor samples what lies behind it. The looked-through levels'
/// opacities still apply when the enclosing frame composites them, so a
/// fading backdrop panel fades rather than disappearing.
///
/// The member — its backdrop sample and its content — composites as a
/// whole with its own [`crate::Layer::opacity`] and
/// [`crate::Layer::blend`]: a filter covers the member's items, never
/// the sample. An unfiltered member's sample lands in its own canvas;
/// a filtered member's sample lands beside its filtered content in an
/// outer member scope when opacity or blend is not a no-op (else in the
/// enclosing canvas), so the member's blend applies to the sample for
/// both member kinds.
///
/// The capture is taken at `scale` times device resolution: capture texel
/// `(i, j)` holds the area-weighted mean of the canvas over the device rect
/// `[i/s, (i+1)/s) × [j/s, (j+1)/s)` clipped to the canvas, `filters` run on
/// that grid with their parameters in capture texels, and members sample
/// it bilinearly at device point `p · s`. `1.0` is the 1:1 capture.
///
/// A `levels` count above one reduces the filtered capture (level 0) into
/// a pyramid: level `k` texel `(i, j)` is the mean of level `k−1` texels
/// `(2i..=2i+1, 2j..=2j+1)`, a partial box at the grid's edge averaging the
/// texels present. Members read deeper levels through
/// [`BackdropEffectSpec::Level`]'s trilinear sample, which a variable-blur
/// material needs; `1` is the single-level capture.
///
/// A `union` smoothing distance `k` (device pixels, finite and above 0 —
/// [`crate::Scene::load`] validates it) makes every member composite
/// against one shared field: the quadratic smooth minimum of all member
/// distances, folded in ascending order with `k`. Each member's
/// composite coverage becomes its antialiased ownership weight — member
/// `i` has `f_i = d₂ − d_i` (`d₂` the smallest distance among the other
/// members) and weight `a_i = clamp(0.5 + f_i/|∇f_i|, 0, 1)`,
/// `|∇f_i| = |∇d₂ − ∇d_i|` the ownership boundary's own slope,
/// normalized to `a_i/Σ_j a_j` so the weights partition unity — times
/// the antialiased coverage of `field < outer`,
/// replacing the member's clip coverage (the member's own content stays
/// clipped; see [`crate::Layer::backdrop_outer`]). Every member needs an
/// analytic clip — a path clip fails `backdrop-effect-sdf-path`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackdropGroup {
    /// The id member layers reference.
    pub id: u32,
    /// The filters applied to the capture, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<BackdropFilter>,
    /// The capture scale `s`, `0 < s ≤ 1` ([`crate::Scene::load`]
    /// validates it).
    pub scale: f64,
    /// The pyramid's level count, from `1` to [`Self::MAX_LEVELS`]
    /// ([`crate::Scene::load`] validates it); `1` is the single-level
    /// capture.
    #[serde(default = "default_levels", skip_serializing_if = "is_default_levels")]
    pub levels: u32,
    /// The [`crate::Layer::id`] of the layer the capture anchors at: the
    /// capture is taken beneath that layer, at its paint-order position
    /// before the layer's own content and children. Every member must
    /// then paint after the anchor in the anchor's compositing canvas —
    /// its descendants or its later siblings there; any other member
    /// fails the render
    /// (`backdrop-member-before-anchor` /
    /// `backdrop-member-outside-anchor-canvas`). `None` (the default)
    /// captures at the first member's paint-order position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<std::num::NonZeroU32>,
    /// The union field's smoothing distance `k` in device pixels
    /// ([`crate::Scene::load`] validates it above 0); `None` composites
    /// every member against its own clip only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub union: Option<f64>,
}

impl BackdropGroup {
    /// The most pyramid levels a group declares; [`crate::Scene::load`]
    /// rejects a scene file asking for more.
    pub const MAX_LEVELS: u32 = 8;

    /// A plain group description: unanchored, no union field — set
    /// `anchor` or `union` on the returned value to compose them.
    #[must_use]
    pub const fn new(id: u32, filters: Vec<BackdropFilter>, scale: f64, levels: u32) -> Self {
        Self {
            id,
            filters,
            scale,
            levels,
            anchor: None,
            union: None,
        }
    }
}

const fn default_levels() -> u32 {
    1
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if takes a reference"
)]
const fn is_default_levels(levels: &u32) -> bool {
    *levels == 1
}

/// One filter in a backdrop group's capture chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "filter", content = "value", rename_all = "kebab-case")]
pub enum BackdropFilter {
    /// A separable Gaussian blur (filtrate's `GaussianBlur`): kernel radius
    /// `⌈3σ⌉`, clamp-to-edge sampling at the capture boundary.
    GaussianBlur {
        /// The blur's standard deviation in capture texels.
        sigma: f64,
    },
    /// A 3×4 colour matrix (filtrate's `ColorMatrix<T>([T;12])`): three rows
    /// of four applied to the premultiplied `[r, g, b, a]` pixel —
    /// `out_i = dot(row_i, pixel)` for `i` in `0..3`, the fourth column a
    /// bias that scales with alpha. The output alpha is the input alpha.
    ColorMatrix {
        /// The 12 coefficients, row-major.
        matrix: [f64; 12],
    },
}

/// A member layer's per-member effect on its backdrop composite
/// ([`crate::Layer::backdrop_effect`]).
///
/// The effect samples the group's filtered capture inside the member's
/// clip; `ColorMatrix` needs only the member's own pixel, while
/// `Refraction` and `RimLight` read the clip's signed distance and are
/// unsupported on a path clip (`backdrop-effect-sdf-path`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", content = "value", rename_all = "kebab-case")]
pub enum BackdropEffectSpec {
    /// A 3×4 colour matrix on the premultiplied sampled pixel, the same
    /// layout as [`BackdropFilter::ColorMatrix`].
    ColorMatrix {
        /// The 12 coefficients, row-major.
        matrix: [f64; 12],
    },
    /// Edge-following refraction: the sample point pulls inward along the
    /// clip's unit outward normal by `strength · t²` where
    /// `t = clamp(1 + d / depth, 0, 1)` and `d` is the signed distance to
    /// the clip edge (negative inside).
    Refraction {
        /// How deep inside the clip the displacement fades out, in pixels.
        depth: f64,
        /// The maximum displacement at the edge, in pixels.
        strength: f64,
    },
    /// A highlight inside the clip's rim: `c = sample(p)`,
    /// `t = clamp(1 + d / width, 0, 1)` and
    /// `c.rgb += color.rgb · color.a · gain · t²`, alpha unchanged.
    RimLight {
        /// The rim's width inside the clip edge, in pixels.
        width: f64,
        /// The highlight colour, straight-alpha linear Display P3 (the
        /// scene's working space).
        color: [f64; 4],
        /// The highlight's gain; values above 1 push the rim above SDR
        /// white.
        gain: f64,
    },
    /// A trilinear pyramid read at a per-pixel level: `c =
    /// sample_level(p, interior_level + (edge_level −
    /// interior_level)·t)` with `t = clamp(1 + d / depth, 0, 1)` — a
    /// blur that fades from `edge_level` at the clip's edge to
    /// `interior_level` deep inside. SDF-required like
    /// `Refraction`.
    #[serde(rename_all = "kebab-case")]
    Level {
        /// How deep inside the clip the level fades out, in pixels.
        depth: f64,
        /// The pyramid level at the clip's edge (`t = 1`).
        edge_level: f64,
        /// The pyramid level deep inside the clip (`t = 0`).
        interior_level: f64,
    },
}
