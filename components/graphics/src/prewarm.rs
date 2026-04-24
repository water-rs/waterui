//! Simple shader loading utilities.
//!
//! Provides compile-time shader loading via `include_shader!` macro.

use crate::shared_context::{SharedContextError, init_shared_context, shared_context};

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

/// Backwards-compatible alias for [`ShaderSource`].
pub type PrewarmedShader = ShaderSource;

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
pub const SHADER_SURFACE_PRELUDE: &str = include_str!("shaders/prelude.wgsl");

static FLOWING_GRADIENT_SHADER_SURFACE: ShaderSource = ShaderSource::new(
    "shaders/flowing_gradient.wgsl#shader_surface",
    concat!(
        include_str!("shaders/prelude.wgsl"),
        include_str!("shaders/flowing_gradient.wgsl")
    ),
);

/// Built-in WGSL modules that should be prewarmed after GPU context init.
pub static BUILTIN_SHADER_SOURCES: &[ShaderSource] = &[
    ShaderSource::new(
        "shaders/animated_mesh_gradient.wgsl",
        include_str!("shaders/animated_mesh_gradient.wgsl"),
    ),
    ShaderSource::new(
        "shaders/mesh_gradient.wgsl",
        include_str!("shaders/mesh_gradient.wgsl"),
    ),
    ShaderSource::new("shaders/blit.wgsl", include_str!("shaders/blit.wgsl")),
    FLOWING_GRADIENT_SHADER_SURFACE,
];

/// Returns the pre-composed `ShaderSurface` source for built-in flowing gradient.
#[must_use]
pub fn flowing_gradient_shader_surface_source() -> &'static ShaderSource {
    &FLOWING_GRADIENT_SHADER_SURFACE
}

/// Prewarms built-in shaders into the shared module cache.
///
/// # Errors
///
/// Returns shared-context initialization failures.
pub fn prewarm_builtin_shaders() -> Result<usize, SharedContextError> {
    if !crate::shared_context::is_initialized() {
        init_shared_context()?;
    }
    let ctx = shared_context();
    {
        let guard = ctx.read();
        for shader in BUILTIN_SHADER_SOURCES {
            let _ = guard.get_or_create_static_shader(shader);
        }
    }

    Ok(BUILTIN_SHADER_SOURCES.len())
}

/// Starts asynchronous built-in shader prewarming without blocking startup.
pub fn spawn_builtin_shader_prewarm() {
    let enabled = std::env::var("WATERUI_GPU_PREWARM")
        .ok()
        .is_none_or(|value| !matches!(value.trim(), "0" | "false" | "FALSE"));
    if !enabled {
        tracing::info!("[ShaderPrewarm] disabled by WATERUI_GPU_PREWARM");
        return;
    }

    let result = std::thread::Builder::new()
        .name("waterui-gpu-prewarm".to_owned())
        .spawn(|| match prewarm_builtin_shaders() {
            Ok(count) => {
                crate::shared_context::save_pipeline_cache();
                tracing::debug!("[ShaderPrewarm] completed {} shader modules", count);
            }
            Err(error) => {
                tracing::warn!("[ShaderPrewarm] skipped due to init error: {error}");
            }
        });

    if let Err(error) = result {
        tracing::warn!("[ShaderPrewarm] failed to spawn worker: {error}");
    }
}
