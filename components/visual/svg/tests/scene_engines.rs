//! Renders the same SVG through both scene engines.
//!
//! Icons are the reason the hybrid engine exists: an icon draws through
//! `Scene2D`, and on a device without indirect execution the compute pipeline
//! takes the process down rather than drawing it. This machine's GPU can run
//! either engine, which is how the two are compared here.
//!
//! The PNGs are written out to be looked at. The engines rasterize
//! differently, so nothing about them is asserted pixel-wise.

use waterui_graphics::shared_context::SceneEngine;
use waterui_graphics::{GpuRuntime, OffscreenRenderConfig, OffscreenSize, SceneView};
use waterui_svg::SvgSceneContent;
use waterui_testing::TestArtifacts;

const STROKED_ICON: &str = include_str!("data/stroked_icon.svg");
const PAINTED_ICON: &str = include_str!("data/painted_icon.svg");

#[test]
fn both_scene_engines_render_an_svg() {
    let artifacts = TestArtifacts::new("svg");
    std::fs::create_dir_all(artifacts.case_dir("scene_engines"))
        .expect("output directory must be creatable");
    let runtime = pollster::block_on(GpuRuntime::new())
        .expect("scene engine comparison requires a working GPU runtime");
    let size = OffscreenSize::try_from_pixels(192, 192).expect("test size must be valid");

    for (content, icon_name) in [(STROKED_ICON, "stroked"), (PAINTED_ICON, "painted")] {
        for (engine, engine_name) in [
            (SceneEngine::Classic, "classic"),
            (SceneEngine::Hybrid, "hybrid"),
        ] {
            let surface = SceneView::new(SvgSceneContent::new(content)).into_gpu_surface();
            let config = OffscreenRenderConfig::new(size)
                .format(wgpu::TextureFormat::Rgba8Unorm)
                .scene_engine(engine);
            let mut env = waterui_core::Environment::new();
            let output = pollster::block_on(surface.render_offscreen(&runtime, config, &mut env))
                .expect("offscreen render should succeed");
            output
                .save_png(
                    artifacts.snapshot_path("scene_engines", format!("{icon_name}_{engine_name}")),
                )
                .expect("png should be written");
        }
    }
}
