//! Feature-name strings carried by
//! [`RenderError::Unsupported`](cherenkov::RenderError) and
//! [`ResourceError::Unsupported`](cherenkov::ResourceError). These are the
//! names the deleted `Unsupported` enum displayed; the benchmark harness
//! maps them back to scene features.

/// A general path.
pub const PATH: &str = "path";
/// A shape without a fillable silhouette casts no shadow.
pub const SHADOW: &str = "shadow";
/// A user shader paint.
pub const SHADER: &str = "shader-paint";
/// A backdrop group member without a clip.
pub const BACKDROP_UNCLIPPED: &str = "backdrop-unclipped";
/// A backdrop filter footprint too large to bound.
pub const BACKDROP_FOOTPRINT: &str = "backdrop-footprint";
/// A backdrop group anchored at the walk's root layer: its capture
/// would run before anything paints — the clear colour.
pub const BACKDROP_ANCHOR_AT_ROOT: &str = "backdrop-anchor-at-root";
/// A backdrop group anchored at a projective layer: the layer is a
/// flattening boundary, so the anchor's canvas is not the member's.
pub const BACKDROP_ANCHOR_PROJECTIVE: &str = "backdrop-anchor-projective";
/// A backdrop group anchored at a `Layer::id` no layer in the scene
/// carries.
pub const BACKDROP_UNKNOWN_ANCHOR: &str = "backdrop-unknown-anchor";
/// A member of an anchored backdrop group that paints before the anchor.
pub const BACKDROP_MEMBER_BEFORE_ANCHOR: &str = "backdrop-member-before-anchor";
/// A member of an anchored backdrop group outside the anchor's canvas.
pub const BACKDROP_MEMBER_OUTSIDE_ANCHOR_CANVAS: &str = "backdrop-member-outside-anchor-canvas";
/// A stroked bitmap glyph run.
pub const GLYPH_STROKE: &str = "glyph-stroke";
/// The unsupported feature: a refraction or shader backdrop effect on a
/// member whose clip has no analytic shape (a path/mask clip).
pub const BACKDROP_EFFECT_SDF_PATH: &str = "backdrop-effect-sdf-path";
/// A colour font (COLR, CBDT or sbix).
pub const COLOR_FONT: &str = "color-font";
/// A backdrop union group with more members than the fragment-side fold
/// can evaluate in registers.
pub const BACKDROP_UNION_MEMBERS: &str = "backdrop-union-members";
/// A union member whose clip's analytic shape is degenerate (a
/// zero-radius circle or ellipse): no SDF exists to fold.
pub const BACKDROP_UNION_DEGENERATE_MEMBER: &str = "backdrop-union-degenerate-member";
/// A path clip whose rasterized mask does not fit the atlas.
pub const PATH_CLIP_TOO_LARGE: &str = "path-clip-too-large";
