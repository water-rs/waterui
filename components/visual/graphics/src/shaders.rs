//! Shared shader sources for GPU rendering.

use crate::prewarm::ShaderSource;

/// Shared blit shader for compositing scene textures.
pub const BLIT: ShaderSource = crate::include_shader!("shaders/blit.wgsl");
