//! Compiles the shape GPU shaders when the GPU feature is enabled.

use std::path::PathBuf;

fn main() {
    if std::env::var_os("CARGO_FEATURE_GPU").is_none() {
        return;
    }
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    waterui_build_support::shader::compile_wgsl_shader(
        manifest_dir.join("src/shaders/morph.wgsl"),
        "morph",
    );
}
