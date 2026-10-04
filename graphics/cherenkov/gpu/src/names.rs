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
/// A stroked bitmap glyph run.
pub const GLYPH_STROKE: &str = "glyph-stroke";
/// The unsupported feature: a refraction or shader backdrop effect on a
/// member whose clip has no analytic shape (a path/mask clip).
pub const BACKDROP_EFFECT_SDF_PATH: &str = "backdrop-effect-sdf-path";
/// A colour font (COLR, CBDT or sbix).
pub const COLOR_FONT: &str = "color-font";
/// A path clip whose rasterized mask does not fit the atlas.
pub const PATH_CLIP_TOO_LARGE: &str = "path-clip-too-large";
