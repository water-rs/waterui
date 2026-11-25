//! The WGSL preludes user shaders compile against: the contract every
//! backend composes a producer, a shader paint or a backdrop shader with, so
//! a shader means the same on each.

use filtrate_core::space::WGSL;

/// The colour prelude: the conversions a producer writes premultiplied
/// working colour through. It is written against
/// [`filtrate_core::space::WGSL`], which precedes it.
pub const COLOR: &str = include_str!("wgsl/color.wgsl");

/// The shader-paint prelude: the `uniforms` (`time`, `resolution`) and
/// `params` (`array<vec4<f32>, 16>`) globals, and the `VertexOutput` a
/// paint's `main` fragment entry point takes.
pub const PAINT: &str = include_str!("wgsl/paint.wgsl");

/// The fragment entry point a shader paint defines.
pub const PAINT_ENTRY: &str = "main";

/// Maximum number of scalar `f32` parameters a shader paint carries (`16` `vec4<f32>` slots).
pub const PAINT_MAX_PARAMS: usize = 64;

/// Why a user's shader paint WGSL or parameter configuration is invalid.
///
/// Parse and validation errors carry `naga`'s rendered diagnostic with line and
/// column numbers counted in the application's own WGSL source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ShaderError {
    /// The WGSL source failed to parse.
    #[error("shader WGSL failed to parse:\n{0}")]
    Parse(String),
    /// The WGSL module parsed but failed validation.
    #[error("shader WGSL failed validation:\n{0}")]
    Validation(String),
    /// The module declares no `@fragment fn main` entry point.
    #[error("shader WGSL declares no `@fragment fn {PAINT_ENTRY}` entry point")]
    MissingEntryPoint,
    /// More parameters were bound than the uniform block holds.
    #[error("shader accepts at most {max} parameters, got {count}")]
    TooManyParams {
        /// Number of parameters supplied.
        count: usize,
        /// Maximum supported parameters.
        max: usize,
    },
}

/// Validates a shader paint's WGSL fragment `source` against the engine prelude.
///
/// The prelude is appended after `source` so line and column numbers in
/// diagnostics stay in the author's own WGSL source.
///
/// # Errors
///
/// Returns [`ShaderError`] if `source` fails WGSL parsing, fails `naga`
/// validation under core WebGPU capabilities, or defines no `@fragment fn main`.
#[cfg(feature = "validate")]
pub fn validate_paint_wgsl(source: &str) -> Result<(), ShaderError> {
    use cherenkov_shader::naga::{
        ShaderStage,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };

    let full = [source, WGSL, COLOR, PAINT].join("\n");
    let module =
        wgsl::parse_str(&full).map_err(|error| ShaderError::Parse(error.emit_to_string(&full)))?;
    Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .map_err(|error| ShaderError::Validation(error.emit_to_string(&full)))?;
    if !module
        .entry_points
        .iter()
        .any(|entry| entry.stage == ShaderStage::Fragment && entry.name == PAINT_ENTRY)
    {
        return Err(ShaderError::MissingEntryPoint);
    }
    Ok(())
}

/// The backdrop shader contract as declarations.
///
/// It holds `BackdropPixel`, `BackdropField`, `BackdropSibling`,
/// `move_space` and the `SPACE_*` constants, and the queries
/// `backdrop_sample`, `backdrop_sample_level`, `backdrop_field`,
/// `backdrop_sibling_count`, `backdrop_sibling_index` and
/// `backdrop_sibling`, whose bodies only declare their signatures: an
/// evaluator binds every call to them to its own capture and field. It
/// is written against [`filtrate_core::space::WGSL`], which precedes it.
pub const BACKDROP: &str = include_str!("wgsl/backdrop.wgsl");

/// The function a backdrop shader defines:
/// `fn(BackdropPixel, array<vec4<f32>, 16>) -> vec4<f32>`.
pub const BACKDROP_ENTRY: &str = "backdrop_effect";

/// The content-effect shader contract as declarations.
///
/// It holds `ContentPixel`, `move_space`, the `SPACE_*` constants, and
/// `content_sample(q)` over the layer's isolated content image. It is
/// written against [`filtrate_core::space::WGSL`], which precedes it.
pub const CONTENT: &str = include_str!("wgsl/content.wgsl");

/// The function a content-effect shader defines:
/// `fn(ContentPixel, array<vec4<f32>, 16>) -> vec4<f32>`.
pub const CONTENT_ENTRY: &str = "content_effect";

/// `source` composed after the working-space conversions and [`COLOR`]:
/// the module a producer's shader compiles to.
#[must_use]
pub fn producer_module(source: &str) -> String {
    [filtrate_core::space::WGSL, COLOR, source].join("\n")
}

/// The module a shader paint's fragment `source` compiles to:
/// [`producer_module`] of the paint prelude followed by the source.
#[must_use]
pub fn paint_module(source: &str) -> String {
    [filtrate_core::space::WGSL, COLOR, PAINT, source].join("\n")
}

/// The module a backdrop shader's `source` is evaluated in when the
/// evaluator binds its capture reads.
///
/// It is the working-space conversions, then [`BACKDROP`], then the source,
/// which names the prelude's types and functions and so cannot be parsed
/// apart from it.
#[must_use]
pub fn backdrop_module(source: &str) -> String {
    [filtrate_core::space::WGSL, BACKDROP, source].join("\n")
}

/// The module a content-effect shader's `source` is validated and evaluated
/// in when the evaluator binds `content_sample`.
#[must_use]
pub fn content_module(source: &str) -> String {
    [WGSL, CONTENT, source].join("\n")
}
