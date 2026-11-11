//! The WGSL preludes user shaders compile against: the contract every
//! backend composes a producer or a shader paint with, so a shader means
//! the same on each.

use crate::ShaderSource;

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

/// `source` composed after the working-space conversions and [`COLOR`]:
/// the module a producer's shader compiles to.
#[must_use]
pub fn producer_module(source: &str) -> String {
    [filtrate_core::space::WGSL, COLOR, source].join("\n")
}

/// The module a shader paint compiles to: [`producer_module`] of the
/// paint prelude followed by the paint's fragment source.
#[must_use]
pub fn paint_module(source: &ShaderSource) -> String {
    [filtrate_core::space::WGSL, COLOR, PAINT, &source.source].join("\n")
}
