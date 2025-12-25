//! Simple shader loading utilities.
//!
//! Provides compile-time shader loading via `include_shader!` macro.

/// A shader source loaded at compile time.
#[derive(Debug, Clone, Copy)]
pub struct ShaderSource {
    /// The label for the shader (file path).
    pub label: &'static str,
    /// The WGSL source code.
    pub source: &'static str,
}

impl ShaderSource {
    /// Create a new shader source.
    #[must_use]
    pub const fn new(label: &'static str, source: &'static str) -> Self {
        Self { label, source }
    }
}

/// Macro to include a shader file at compile time.
///
/// Returns a `ShaderSource` with the file path as label and contents as source.
#[macro_export]
macro_rules! include_shader {
    ($path:literal) => {
        $crate::prewarm::ShaderSource::new($path, include_str!($path))
    };
}

/// Macro to include a fragment-only shader file at compile time.
/// Used by `ShaderSurface` which auto-prepends the standard prelude.
#[macro_export]
macro_rules! include_fragment_shader {
    ($path:literal) => {
        $crate::prewarm::ShaderSource::new($path, include_str!($path))
    };
}

// Re-export for backwards compatibility
pub type PrewarmedShader = ShaderSource;
