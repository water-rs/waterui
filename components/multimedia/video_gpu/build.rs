//! Compiles the planar and spherical video GPU shaders for the selected target.

use std::fs;
use std::path::PathBuf;

fn main() {
    shaderloom::build::compile_wgsl_source(
        "waterkit-codec/yuv_to_rgba.wgsl",
        waterkit_codec::YUV_COLOR_SHADER_WGSL,
        "video_yuv",
    );

    let spherical_path =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"))
            .join("src/shaders/spherical_video_render.wgsl");
    println!("cargo:rerun-if-changed={}", spherical_path.display());
    let spherical = fs::read_to_string(&spherical_path).unwrap_or_else(|error| {
        panic!(
            "failed to read spherical video shader {}: {error}",
            spherical_path.display()
        )
    });
    shaderloom::build::compile_wgsl_source(
        "waterkit-codec/yuv_to_rgba.wgsl+spherical_video_render.wgsl",
        &format!("{}{spherical}", waterkit_codec::YUV_COLOR_SHADER_WGSL),
        "video_yuv_spherical",
    );
}
