//! Compiles the barcode GPU shaders for the selected target.

use std::path::PathBuf;

fn main() {
    let shader_root =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"))
            .join("src/shaders");
    waterui_build_support::shader::compile_wgsl_shader(
        shader_root.join("qr_mask_effect.wgsl"),
        "qr_mask_effect",
    );
    waterui_build_support::shader::compile_wgsl_shader(
        shader_root.join("qr_render.wgsl"),
        "qr_render",
    );
}
