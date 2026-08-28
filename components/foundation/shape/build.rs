//! Compiles the shape GPU shaders when the GPU feature is enabled.

#[cfg(feature = "gpu")]
use std::path::PathBuf;

fn main() {
    #[cfg(feature = "gpu")]
    {
        let manifest_dir = PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"),
        );
        shaderloom::build::compile_wgsl_shader(
            manifest_dir.join("src/shaders/morph.wgsl"),
            "morph",
        );
    }
}
