//! Compiles the static Filtrate GPU shaders for the selected target.

use std::path::PathBuf;

fn main() {
    let shader_root =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"))
            .join("src/shaders/shared");
    waterui_build_support::shader::compile_wgsl_shader(
        shader_root.join("blit.wgsl"),
        "filter_blit",
    );
    waterui_build_support::shader::compile_wgsl_shader(
        shader_root.join("multi_input_filter.wgsl"),
        "multi_input_filter",
    );
}
