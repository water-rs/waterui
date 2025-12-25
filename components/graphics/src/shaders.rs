//! Shared shader sources for GPU rendering.

use crate::prewarm::ShaderSource;

/// Shared blit shader for blitting textures to screen.
/// Used by Canvas, SVG, and Barcode renderers.
pub static BLIT: ShaderSource = crate::include_shader!("shaders/blit.wgsl");
