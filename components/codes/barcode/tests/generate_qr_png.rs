//! Generate a QR PNG using the GPU barcode renderer.

use std::path::PathBuf;

use waterui_barcode::{BarcodeRenderer, BarcodeSource};
use waterui_graphics::{GpuRuntime, GpuSurface, OffscreenRenderConfig, OffscreenSize};

#[test]
fn generate_qr_png_offscreen() {
    let content =
        std::env::var("WATERUI_QR_CONTENT").unwrap_or_else(|_| "https://waterui.dev".to_string());
    let out_path = std::env::var("WATERUI_QR_OUT").map_or_else(|_| PathBuf::from("target/generated_qr.png"), PathBuf::from);

    let renderer = BarcodeRenderer::new(BarcodeSource::qr(content));
    let size = OffscreenSize::try_from_pixels(768, 768).expect("valid output size");
    let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
    let runtime = pollster::block_on(GpuRuntime::new())
        .expect("QR export test requires a working GPU runtime");
    let mut env = waterui_core::Environment::new();
    let output = pollster::block_on(GpuSurface::new(renderer).render_offscreen(
        &runtime,
        config,
        &mut env,
    ))
    .expect("offscreen QR render should succeed");
    assert_eq!(
        output.rgba8.len(),
        (output.width * output.height * 4) as usize
    );

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("output directory should be creatable");
    }
    output.save_png(&out_path).expect("png should be writable");
    assert!(out_path.exists(), "png file should exist at {out_path:?}");
}
