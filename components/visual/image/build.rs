//! Compiles the image renderer GPU shader for the selected target.

use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    waterui_build_support::shader::compile_wgsl_shader(
        manifest_dir.join("src/shaders/image_render.wgsl"),
        "image_render",
    );
}
