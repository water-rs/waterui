//! Generate a Code128 PNG using the GPU barcode renderer.

use std::path::PathBuf;

use rxing::BarcodeFormat;
use waterui_barcode::{BarcodeRenderer, BarcodeSource};
use waterui_graphics::{GpuRuntime, GpuSurface, OffscreenRenderConfig, OffscreenSize};

mod support;

#[test]
fn generate_code128_png_offscreen() {
    let content = std::env::var("WATERUI_CODE128_CONTENT")
        .unwrap_or_else(|_| "HELLO-WATERUI-128".to_string());
    let out_path = std::env::var("WATERUI_CODE128_OUT").map_or_else(
        |_| PathBuf::from("/tmp/generated_code128.png"),
        PathBuf::from,
    );

    let renderer = BarcodeRenderer::new(
        BarcodeSource::code128(content.clone()).expect("static test payload must encode"),
    );
    let size = OffscreenSize::try_from_pixels(1024, 256).expect("valid output size");
    let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
    let runtime = pollster::block_on(GpuRuntime::new())
        .expect("Code128 export test requires a working GPU runtime");
    let mut env = waterui_core::Environment::new();
    let output =
        pollster::block_on(GpuSurface::new(renderer).render_offscreen(&runtime, config, &mut env))
            .expect("offscreen Code128 render should succeed");
    assert_eq!(
        output.rgba8.len(),
        (output.width * output.height * 4) as usize
    );
    assert_eq!(support::decode(&output, BarcodeFormat::CODE_128), content);

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("output directory should be creatable");
    }
    output.save_png(&out_path).expect("png should be writable");
    assert!(out_path.exists(), "png file should exist at {out_path:?}");
}
