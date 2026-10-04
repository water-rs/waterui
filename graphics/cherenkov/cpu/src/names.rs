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
/// A per-member backdrop effect that reads the clip's signed distance
/// on a clip without an analytic boundary (`Path`/`Line`).
pub const BACKDROP_EFFECT_SDF_PATH: &str = "backdrop-effect-sdf-path";
/// A backdrop effect shader — a GPU-only capability.
pub const BACKDROP_SHADER: &str = "backdrop-shader";
/// A stroked glyph run.
pub const GLYPH_STROKE: &str = "glyph-stroke";
/// A colour font construct this backend cannot render: a bitmap-only
/// font (CBDT/sbix without outlines) or an unmapped COLR paint.
pub const COLOR_FONT: &str = "color-font";

/// A shape without a fillable silhouette casts no shadow.
pub const SHADOW: &str = "shadow";
