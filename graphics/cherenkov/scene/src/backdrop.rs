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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackdropGroup {
    /// The id member layers reference.
    pub id: u32,
    /// The filters applied to the capture, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<BackdropFilter>,
}

/// One filter in a backdrop group's capture chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "filter", content = "value", rename_all = "kebab-case")]
pub enum BackdropFilter {
    /// A separable Gaussian blur (filtrate's `GaussianBlur`): kernel radius
    /// `⌈3σ⌉`, clamp-to-edge sampling at the capture boundary.
    GaussianBlur {
        /// The blur's standard deviation in pixels.
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
}
