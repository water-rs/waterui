//! Shader loading utilities.
//!
//! This module provides compile-time shader loading via the `include_shader!` macro.
//! Shaders are compiled on-demand during `GpuRenderer::setup()`, naturally parallelized
//! when multiple GpuSurfaces are initialized together.

use std::borrow::Cow;

/// A shader loaded at compile time.
///
/// This struct holds the shader label and source code, loaded via `include_shader!`.
#[derive(Debug, Clone)]
pub struct PrewarmedShader {
    /// The label of the shader (usually the file path).
    pub label: &'static str,
    /// The WGSL source code.
    pub source: Cow<'static, str>,
}

impl PrewarmedShader {
    /// Create a new shader descriptor.
    /// This is typically called by the `include_shader!` macro.
    pub const fn new(label: &'static str, source: &'static str) -> Self {
        Self {
            label,
            source: Cow::Borrowed(source),
        }
    }
}

/// Macro to include a shader file at compile time.
///
/// Usage: `static MY_SHADER: PrewarmedShader = include_shader!("path/to/shader.wgsl");`
#[macro_export]
macro_rules! include_shader {
    ($path:literal) => {{
        const SOURCE: &str = include_str!($path);
        const PATH: &str = $path;
        $crate::prewarm::PrewarmedShader::new(PATH, SOURCE)
    }};
}

/// Macro to include a fragment-only shader file at compile time.
///
/// This is used by `ShaderSurface` which auto-prepends the standard prelude.
#[macro_export]
macro_rules! include_fragment_shader {
    ($path:literal) => {{
        const SOURCE: &str = include_str!($path);
        const PATH: &str = $path;
        $crate::prewarm::PrewarmedShader::new(PATH, SOURCE)
    }};
}
