//! Compiles the particle GPU shaders for the selected target.

use std::path::PathBuf;

fn main() {
    let source_root =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"))
            .join("src");
    shaderloom::build::compile_wgsl_shader(
        source_root.join("particle_compute.wgsl"),
        "particle_compute",
    );
    shaderloom::build::compile_wgsl_shader(
        source_root.join("particle_render.wgsl"),
        "particle_render",
    );
}
