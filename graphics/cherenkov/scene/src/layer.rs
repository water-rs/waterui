use serde::{Deserialize, Serialize};

use crate::{BackdropEffectSpec, BlendMode, BlendSpace, Draw, Paint, ResourceHash, Shape};
use kurbo::{Affine, Rect, Vec2};

/// One item in a layer's ordered item list: a child layer, a group of draws
/// or a draw command.
///
/// Serialized externally tagged (`{"draw": ..}` / `{"layer": ..}`): internally
/// and untagged serde representations cannot nest enums within the buffered
/// content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Item {
    /// A draw command.
    Draw(Draw),
    /// A child layer.
    Layer(Layer),
    /// An isolated group of draws.
    Group(Group),
}

/// One item in a [`Group`]'s ordered member list: a draw or a nested group.
/// A group has no child layers — it scopes draw commands only, like the
/// display-list group it maps to.
///
/// Serialized externally tagged, like [`Item`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupItem {
    /// A draw command.
    Draw(Draw),
    /// A nested group.
    Group(Group),
}

/// An isolated group of draws, the display-list `group`: members composite
/// with each other in `blend_space`, then the group composites onto the
/// enclosing level with `opacity` and `blend`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// Ordered members.
    #[serde(default)]
    pub items: Vec<GroupItem>,
    /// Group opacity, `0.0..=1.0`. `1.0` (the default) leaves alpha unchanged.
    #[serde(default = "Group::default_opacity")]
    pub opacity: f64,
    /// The blend mode used when compositing onto the parent.
    #[serde(default)]
    pub blend: BlendMode,
    /// The space the group blends and its members composite in: `linear`
    /// (the default) composites premultiplied linear values; `srgb-encoded`
    /// composites sRGB-encoded values.
    #[serde(default)]
    pub blend_space: BlendSpace,
}

impl Group {
    const fn default_opacity() -> f64 {
        1.0
    }
}

/// A layer of the scene tree.
///
/// Items are drawn in order into the layer's own buffer; the layer is then
/// composited onto its parent with `transform`, `clip`, `opacity` and
/// `blend` applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// The layer's affine transform relative to its parent.
    #[serde(default)]
    pub transform: Affine,
    /// An optional clip shape (in the layer's own coordinate space) masking
    /// the whole layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Shape>,
    /// Group opacity, `0.0..=1.0`. `1.0` (the default) leaves alpha unchanged.
    #[serde(default = "Layer::default_opacity")]
    pub opacity: f64,
    /// The blend mode used when compositing onto the parent.
    #[serde(default)]
    pub blend: BlendMode,
    /// The id of the [`crate::BackdropGroup`] this layer samples, if any.
    /// A member layer must have a `clip`; the group's capture runs through
    /// the group's filters and is drawn as the bottom-most content inside
    /// that clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<u32>,
    /// A filter applied to the layer's isolated content, in device pixels,
    /// before `opacity` and `blend`; the layer clip masks its output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Box<LayerFilter>>,
    /// The per-member effect applied to the backdrop composite
    /// ([`crate::BackdropEffectSpec`]). Only meaningful with `backdrop`;
    /// `None` is the plain bilinear sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop_effect: Option<BackdropEffectSpec>,
    /// The layer's scroll offset: content and children are translated by
    /// `-scroll_offset` inside the layer's clip; `transform` is untouched.
    /// Zero (the default) draws them untranslated.
    #[serde(default, skip_serializing_if = "vec2_is_zero")]
    pub scroll_offset: Vec2,
    /// A projective pose (#84): the layer is flattened into a layer-local
    /// image that is projected when it composes onto its parent. `None`
    /// (the default) keeps the layer affine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<Box<Projection>>,
    /// One-time motion for this layer: an animation or decay that runs
    /// from its `from` state and comes to rest at the layer's static
    /// properties. Engines that cannot animate report it unsupported and
    /// fall back to the settled scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    /// Ordered items: child layers and draw commands.
    #[serde(default)]
    pub items: Vec<Item>,
    /// Draw items whose value changes every frame, indexed into `items`
    /// (the target must be an `Item::Draw`). Frame `n` uses
    /// `frames[n % len]`; frame 0 is the static scene the oracle renders.
    /// Only the cherenkov adapters honour `live` (as engine slot updates);
    /// others render frame 0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live: Vec<Live>,
    /// The parley input of a text layer, whose `items` are the text's
    /// lowering: only `glyphs` items, rectangle `fill`s, and plain
    /// [`Group`]s isolating the glyph runs of a synthetic bold. Only
    /// the cherenkov adapters honour `text`, recording it through the
    /// engine's parley adapter; the oracle and the other renderers draw
    /// the items. See [`crate::TextSource`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Box<crate::TextSource>>,
}

/// A layer's projective pose. With column vectors, the layer's
/// local-to-parent map is
///
/// `embed(transform) · T(pivot) · matrix · T(0, 0, depth) · Ry(tilt.y) ·
/// Rx(tilt.x) · T(−pivot)`,
///
/// evaluated on the `z = 0` plane: a point `(x, y)` maps to `(X/W, Y/W)`
/// and is visible only where `W > 0`. A projective layer must carry a
/// clip, whose bounds are its finite local source domain. It is a
/// flattening boundary: its content, clip, filter and children render in
/// its own local space (filter parameters in local raster pixels), and
/// the completed image is projected with the layer's opacity and blend.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Projection {
    /// The projection base: a 4×4 row-major matrix on column vectors,
    /// positive Z toward the viewer. Identity by default.
    #[serde(default = "Projection::identity")]
    pub matrix: [[f64; 4]; 4],
    /// Rotation about X (`tilt.x`) then Y (`tilt.y`), radians.
    #[serde(default, skip_serializing_if = "vec2_is_zero")]
    pub tilt: Vec2,
    /// Translation along Z, layer units.
    #[serde(default, skip_serializing_if = "f64_is_zero")]
    pub depth: f64,
    /// The local point the projection, tilt and depth are centred on.
    #[serde(default, skip_serializing_if = "vec2_is_zero")]
    pub pivot: Vec2,
}

impl Projection {
    /// The identity matrix.
    #[must_use]
    pub const fn identity() -> [[f64; 4]; 4] {
        [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ]
    }

    /// A perspective camera at `distance` in front of the `z = 0` plane:
    /// `w = 1 − z / distance`.
    #[must_use]
    pub const fn perspective(distance: f64) -> [[f64; 4]; 4] {
        let mut rows = Self::identity();
        rows[3][2] = -1.0 / distance;
        rows
    }
}

impl Default for Projection {
    fn default() -> Self {
        Self {
            matrix: Self::identity(),
            tilt: Vec2::ZERO,
            depth: 0.0,
            pivot: Vec2::ZERO,
        }
    }
}

/// A filtrate filter on a layer's isolated content, in premultiplied
/// linear Display P3 device pixels. Spatial filters clamp at the surface
/// edge.
///
/// Serialized externally tagged, like [`Item`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayerFilter {
    /// `rgb' = M · rgb + bias · a` on premultiplied colour, alpha
    /// unchanged. Rows are `[r, g, b, bias]` for red, green and blue.
    ColorMatrix {
        /// The 3x4 matrix, row-major.
        matrix: [f64; 12],
    },
    /// `first`, then `second`, as one chained filter.
    ColorMatrixChain {
        /// The matrix applied first.
        first: [f64; 12],
        /// The matrix applied second.
        second: [f64; 12],
    },
    /// A separable Gaussian blur of standard deviation `sigma` pixels.
    GaussianBlur {
        /// Standard deviation in pixels.
        sigma: f64,
    },
    /// A separable box blur: the mean of `2r + 1` texels per axis,
    /// `r = round(radius)`.
    BoxBlur {
        /// Radius in pixels, rounded.
        radius: f64,
    },
    /// Blends each pixel's unpremultiplied colour with the sampled straight
    /// texel, mixes by `amount`, and re-premultiplies with the unchanged alpha.
    BlendImage {
        /// BLAKE3 hash of the image blob in `resources/` (PNG).
        image: ResourceHash,
        /// Mix factor, clamped to `0.0..=1.0`.
        amount: f64,
        /// The blend operator.
        mode: FilterBlend,
    },
}

/// The operators of [`LayerFilter::BlendImage`]: filtrate's blend
/// composite, on non-premultiplied operand values. Soft light, colour
/// dodge and burn and the HSL modes differ from W3C [`BlendMode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FilterBlend {
    /// `top`.
    Normal,
    /// `base · top`.
    Multiply,
    /// `1 − (1 − base)(1 − top)`.
    Screen,
    /// Hard light with the operands swapped.
    Overlay,
    /// `min(base, top)`.
    Darken,
    /// `max(base, top)`.
    Lighten,
    /// `base ∓ …` with the `√base` upper branch.
    SoftLight,
    /// `2·base·top` below `top = ½`, screen above.
    HardLight,
    /// `|base − top|`.
    Difference,
    /// `base + top − 2·base·top`.
    Exclusion,
    /// `base / max(1 − top, 10⁻⁴)`.
    ColorDodge,
    /// `1 − (1 − base) / max(top, 10⁻⁴)`.
    ColorBurn,
    /// HSL: hue of `top`.
    Hue,
    /// HSL: saturation of `top`.
    Saturation,
    /// HSL: hue and saturation of `top`.
    Color,
    /// HSL: lightness of `top`.
    Luminosity,
}

/// Per-frame values for one draw item: `frames[n % len]` replaces the
/// item's value on frame `n`. Every entry must be the same `Draw` variant
/// as `items[item]`; `frames[0]` equals the base item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Live {
    /// Index into the owning layer's `items`.
    pub item: usize,
    /// The draw's value per frame.
    pub frames: Vec<crate::Draw>,
}

/// A layer's one-time motion, applied once when it enters the scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "motion", content = "value")]
pub enum Motion {
    /// Unwrapped scalar rotation about `pivot`. The layer's static transform
    /// is the final composed transform; the adapter recovers its base matrix.
    Rotation {
        /// Starting angle in radians, preserving winding.
        from: f64,
        /// Final angle in radians, preserving winding.
        to: f64,
        /// Local pivot, before rotation.
        pivot: Vec2,
        /// How the angle moves.
        animation: MotionAnimation,
    },
    /// `transform` starts at `from` and animates to the layer's
    /// `transform`.
    Transform {
        /// The transform the layer starts at.
        from: Affine,
        /// How it moves to the static `transform`.
        animation: MotionAnimation,
    },
    /// The projection's `tilt` starts at `from` and animates to the
    /// layer's static `projection.tilt`, preserving winding. The layer
    /// must carry a `projection`.
    Tilt {
        /// The tilt the layer starts at, radians.
        from: Vec2,
        /// How it moves to the static tilt.
        animation: MotionAnimation,
    },
    /// `scroll_offset` starts at `from` and decays with `velocity`
    /// (deceleration per second), optionally rubber-banding to `bounds`.
    /// It must come to rest at the layer's static `scroll_offset` (the
    /// generator guarantees this).
    Scroll {
        /// The scroll offset the layer starts at.
        from: Vec2,
        /// The fling velocity in px/s.
        velocity: Vec2,
        /// Deceleration in px/s².
        deceleration: f64,
        /// Optional rubber-band bounds.
        bounds: Option<Rect>,
    },
    /// The paint operand of draw item `item` starts at `from` and animates
    /// to the item's recorded paint — the recorded-content counterpart of
    /// the layer motions above. `None` on an engine that does not animate
    /// recorded operands.
    Paint {
        /// Index into `items`; the entry must be an `Item::Draw` carrying a
        /// paint operand (`fill`, `stroke` or `glyphs`).
        item: usize,
        /// The paint the operand starts at. Boxed like
        /// [`Paint::Transformed`]'s inner paint to keep `Motion` small.
        from: Box<Paint>,
        /// How it moves to the item's static paint.
        animation: MotionAnimation,
    },
}

/// How a [`Motion::Transform`] animates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "animation", content = "value")]
pub enum MotionAnimation {
    /// A physical spring (`response` seconds, `damping` ratio).
    Spring {
        /// The spring's response period.
        response: f64,
        /// The spring's damping ratio; below 1.0 overshoots.
        damping: f64,
    },
    /// A Bézier-timed curve.
    Curve {
        /// Duration in milliseconds.
        duration_ms: u64,
        /// First control point x.
        x1: f64,
        /// First control point y.
        y1: f64,
        /// Second control point x.
        x2: f64,
        /// Second control point y.
        y2: f64,
    },
}

/// Serde helper: a zero `scroll_offset` is left out of the JSON.
fn vec2_is_zero(v: &Vec2) -> bool {
    *v == Vec2::ZERO
}

/// Serde helper: a zero `depth` is left out of the JSON.
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )
)]
fn f64_is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl Layer {
    const fn default_opacity() -> f64 {
        1.0
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            transform: Affine::IDENTITY,
            clip: None,
            opacity: 1.0,
            blend: BlendMode::Normal,
            backdrop: None,
            filter: None,
            backdrop_effect: None,
            scroll_offset: Vec2::ZERO,
            projection: None,
            motion: None,
            items: Vec::new(),
            live: Vec::new(),
            text: None,
        }
    }
}
