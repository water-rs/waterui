//! Compile-time inclusion for user-authored `ShaderSurface` fragments.

/// A labeled WGSL fragment loaded with `include_str!`.
#[derive(Debug, Clone, Copy)]
pub struct ShaderSource {
    /// Source label used by graphics diagnostics.
    pub label: &'static str,
    /// WGSL fragment source.
    pub source: &'static str,
}

impl ShaderSource {
    /// Creates a labeled shader fragment.
    #[must_use]
    pub const fn new(label: &'static str, source: &'static str) -> Self {
        Self { label, source }
    }
}

/// Includes a user-authored fragment shader relative to the invoking crate's `src` directory.
#[macro_export]
macro_rules! include_fragment_shader {
    ($path:literal) => {
        $crate::shader_source::ShaderSource::new(
            $path,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/", $path)),
        )
    };
}

/// Shared WGSL prelude prepended to dynamic `ShaderSurface` fragments.
pub const SHADER_SURFACE_PRELUDE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/shaders/prelude.wgsl"
));
