//! Renders `space/constants.rs` into the WGSL fragment of the
//! working-space ↔ sRGB conversion (`space::WGSL`), so shaders and the CPU
//! conversion read one definition of the matrices and the transfer.

#[path = "src/space/constants.rs"]
#[allow(
    dead_code,
    reason = "the build script renders only the constants the WGSL fragment declares"
)]
mod constants;

use std::fmt;
use std::path::PathBuf;

use askama::Template;

/// An `f32` as a WGSL literal: the decimal of its exact value, so the
/// shader compiler's parse lands on the same `f32` bits.
struct Literal(f32);

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", f64::from(self.0))
    }
}

/// A row-major Rust matrix as the columns a WGSL `mat3x3` constructor
/// takes.
fn columns(rows: &[[f32; 3]; 3]) -> [[Literal; 3]; 3] {
    std::array::from_fn(|column| std::array::from_fn(|row| Literal(rows[row][column])))
}

#[derive(Template)]
#[template(path = "space.wgsl", escape = "none")]
struct SpaceWgsl {
    p3_to_srgb: [[Literal; 3]; 3],
    srgb_to_p3: [[Literal; 3]; 3],
    slope: Literal,
    offset: Literal,
    scale: Literal,
    gamma: Literal,
    inverse_gamma: Literal,
    linear_break: Literal,
    encoded_break: Literal,
}

fn main() {
    println!("cargo::rerun-if-changed=src/space/constants.rs");
    println!("cargo::rerun-if-changed=templates/space.wgsl");
    let fragment = SpaceWgsl {
        p3_to_srgb: columns(&constants::P3_TO_SRGB),
        srgb_to_p3: columns(&constants::SRGB_TO_P3),
        slope: Literal(constants::SRGB_SLOPE),
        offset: Literal(constants::SRGB_OFFSET),
        scale: Literal(constants::SRGB_SCALE),
        gamma: Literal(constants::SRGB_GAMMA),
        inverse_gamma: Literal(constants::SRGB_INVERSE_GAMMA),
        linear_break: Literal(constants::SRGB_LINEAR_BREAK),
        encoded_break: Literal(constants::SRGB_ENCODED_BREAK),
    }
    .render()
    .expect("the space fragment renders");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("space.wgsl"), fragment).expect("OUT_DIR is writable");
}
