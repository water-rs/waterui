//! The WGSL preludes user shaders compile against: the contract every
//! backend composes a producer, a shader paint or a backdrop shader with, so
//! a shader means the same on each.

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

/// The backdrop shader contract as declarations.
///
/// It holds `BackdropPixel`, `BackdropField`, `move_space` and the `SPACE_*`
/// constants, and the capture reads `backdrop_sample`,
/// `backdrop_sample_level` and `backdrop_field`, whose bodies only declare
/// their signatures: an evaluator binds every call to them to its own
/// capture and field. It is written against [`filtrate_core::space::WGSL`],
/// which precedes it.
pub const BACKDROP: &str = include_str!("wgsl/backdrop.wgsl");

/// The function a backdrop shader defines:
/// `fn(BackdropPixel, array<vec4<f32>, 16>) -> vec4<f32>`.
pub const BACKDROP_ENTRY: &str = "backdrop_effect";

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
