//! Compiles the waveform GPU shader for the selected target.

use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    shaderloom::build::compile_wgsl_shader(
        manifest_dir.join("src/shader_waveform.wgsl"),
        "waveform",
    );
}
