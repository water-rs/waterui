//! Feature-name strings carried by
//! [`RenderError::Unsupported`](cherenkov::RenderError) and
//! [`ResourceError::Unsupported`](cherenkov::ResourceError). These are the
//! names the deleted `Unsupported` enum displayed; the benchmark harness
//! maps them back to scene features.

/// A user shader paint.
pub const SHADER: &str = "shader-paint";

/// A filter requiring an auxiliary GPU image with no CPU texels.
pub const FILTER_GPU_IMAGE: &str = "filter-gpu-image";
/// A backdrop member without a clip shape.
pub const BACKDROP_UNCLIPPED: &str = "backdrop-unclipped";
/// A backdrop group whose filter footprint cannot be bounded.
pub const BACKDROP_FOOTPRINT: &str = "backdrop-footprint";
/// A backdrop group anchored at the walk's root layer: its capture
/// would run before anything paints — the clear colour.
pub const BACKDROP_ANCHOR_AT_ROOT: &str = "backdrop-anchor-at-root";
/// A member of an anchored backdrop group that paints before the anchor.
pub const BACKDROP_MEMBER_BEFORE_ANCHOR: &str = "backdrop-member-before-anchor";
/// A member of an anchored backdrop group outside the anchor's canvas.
pub const BACKDROP_MEMBER_OUTSIDE_ANCHOR_CANVAS: &str = "backdrop-member-outside-anchor-canvas";
/// A per-member backdrop effect that reads the clip's signed distance
/// on a clip without an analytic boundary (`Path`/`Line`).
pub const BACKDROP_EFFECT_SDF_PATH: &str = "backdrop-effect-sdf-path";
/// A backdrop effect shader — a GPU-only capability.
pub const BACKDROP_SHADER: &str = "backdrop-shader";
/// A backdrop union group with more members than the per-pixel fold can
/// evaluate in registers.
pub const BACKDROP_UNION_MEMBERS: &str = "backdrop-union-members";
/// A union member whose clip's analytic shape is degenerate (a
/// zero-radius circle or ellipse): no SDF exists to fold.
pub const BACKDROP_UNION_DEGENERATE_MEMBER: &str = "backdrop-union-degenerate-member";
/// A stroked glyph run.
pub const GLYPH_STROKE: &str = "glyph-stroke";
/// A colour font construct this backend cannot render: a bitmap-only
/// font (CBDT/sbix without outlines) or an unmapped COLR paint.
pub const COLOR_FONT: &str = "color-font";

/// A shape without a fillable silhouette casts no shadow.
pub const SHADOW: &str = "shadow";
