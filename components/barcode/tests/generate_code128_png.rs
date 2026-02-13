//! Generate a Code128 PNG using the GPU barcode renderer.

use std::path::PathBuf;

use waterui_barcode::{BarcodeRenderer, BarcodeSource};
use waterui_graphics::{GpuSurface, OffscreenRenderConfig, OffscreenSize, wgpu};

#[test]
fn generate_code128_png_offscreen() {
    let content = std::env::var("WATERUI_CODE128_CONTENT")
        .unwrap_or_else(|_| "HELLO-WATERUI-128".to_string());
    let out_path = std::env::var("WATERUI_CODE128_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/generated_code128.png"));

    let renderer = BarcodeRenderer::new(BarcodeSource::code128(content));
    let size = OffscreenSize::try_from_pixels(1024, 256).expect("valid output size");
    let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
    let output = GpuSurface::new(renderer)
        .render_offscreen(config)
        .expect("offscreen Code128 render should succeed");
    assert_eq!(
        output.rgba8.len(),
        (output.width * output.height * 4) as usize
    );
    let opaque_pixels = output.rgba8.chunks_exact(4).filter(|px| px[3] > 0).count();
    assert!(
        opaque_pixels > 0,
        "offscreen Code128 output must not be fully transparent"
    );
    let first = &output.rgba8[..4];
    let has_variation = output.rgba8.chunks_exact(4).any(|px| px != first);
    assert!(
        has_variation,
        "offscreen Code128 output must not be a flat color"
    );

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("output directory should be creatable");
    }
    output.save_png(&out_path).expect("png should be writable");
    assert!(out_path.exists(), "png file should exist at {out_path:?}");
}
