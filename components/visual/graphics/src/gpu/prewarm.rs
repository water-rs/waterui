//! Simple shader loading utilities.
//!
//! Provides compile-time shader loading via `include_shader!` macro.

use crate::shared_context::GpuRuntime;

/// A shader source loaded at compile time.
#[derive(Debug, Clone, Copy)]
pub struct ShaderSource {
    /// The label for the shader (file path).
    pub label: &'static str,
    /// The WGSL source code.
    pub source: &'static str,
    /// Stable compile-time hash of WGSL source.
    pub source_hash: u64,
}

impl ShaderSource {
    /// Create a new shader source.
    #[must_use]
    pub const fn new(label: &'static str, source: &'static str) -> Self {
        Self {
            label,
            source,
            source_hash: fnv1a64(source.as_bytes()),
        }
    }
}

/// Macro to include a shader file at compile time.
///
/// Returns a `ShaderSource` with the file path as label and contents as source.
#[macro_export]
macro_rules! include_shader {
    ($path:literal) => {
        $crate::prewarm::ShaderSource::new(
            $path,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/", $path)),
        )
    };
}

/// Macro to include a fragment-only shader file at compile time.
/// Used by `ShaderSurface` which auto-prepends the standard prelude.
#[macro_export]
macro_rules! include_fragment_shader {
    ($path:literal) => {
        $crate::prewarm::ShaderSource::new(
            $path,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/", $path)),
        )
    };
}

const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// Shared WGSL prelude prepended to fragment-only shader surfaces.
pub const SHADER_SURFACE_PRELUDE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/shaders/prelude.wgsl"
));

const FLOWING_GRADIENT_SHADER_SURFACE: ShaderSource = ShaderSource::new(
    "shaders/flowing_gradient.wgsl#shader_surface",
    concat!(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/prelude.wgsl"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/flowing_gradient.wgsl"
        ))
    ),
);

/// Built-in WGSL modules that should be prewarmed after GPU context init.
pub const BUILTIN_SHADER_SOURCES: &[ShaderSource] = &[
    ShaderSource::new(
        "shaders/animated_mesh_gradient.wgsl",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/animated_mesh_gradient.wgsl"
        )),
    ),
    ShaderSource::new(
        "shaders/mesh_gradient.wgsl",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/mesh_gradient.wgsl"
        )),
    ),
    ShaderSource::new(
        "shaders/blit.wgsl",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/blit.wgsl"
        )),
    ),
    FLOWING_GRADIENT_SHADER_SURFACE,
];

/// Returns the pre-composed `ShaderSurface` source for built-in flowing gradient.
#[must_use]
pub const fn flowing_gradient_shader_surface_source() -> &'static ShaderSource {
    &FLOWING_GRADIENT_SHADER_SURFACE
}

/// Prewarms built-in shaders into `runtime`'s device-bound module cache.
pub fn prewarm_builtin_shaders(runtime: &GpuRuntime) {
    let context = runtime.context();
    for shader in BUILTIN_SHADER_SOURCES {
        let _ = context.shader_cache().get_or_create_prehashed(
            context.device.as_ref(),
            shader.label,
            shader.source,
            shader.source_hash,
        );
    }
}
