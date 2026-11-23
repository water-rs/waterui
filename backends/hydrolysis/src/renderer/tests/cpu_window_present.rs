//! End-to-end verification of the `cherenkov-cpu` window presentation pixel
//! path (`render_raster_0rgb_into`), covering initial presentation, scale
//! factor updates, and window resizing.

use std::rc::Rc;

use kurbo::{Affine, Rect};
use waterui::ViewExt as _;
use waterui::graphics::Color;

use super::{MinimalTestTheme, pumped_test_environment};
use crate::engine::RasterEngineState;
use crate::renderer::{FontFamilyResolution, FrameGeometry, HydrolysisRenderer, working_color};
use crate::text::ParleyEngine;

#[test]
fn cpu_window_presentation_renders_and_resizes_0rgb_frames() {
    let state = RasterEngineState::new().expect("CPU raster engine starts");
    let env = pumped_test_environment();
    let text = ParleyEngine::system(FontFamilyResolution::Strict);
    let mut renderer = HydrolysisRenderer::with_engine(Rc::new(MinimalTestTheme::default()), text);

    let view = Color::srgb_hex("#E03131").size(24.0, 16.0);
    renderer.prepare_window_tree(view, &env);

    assert!(renderer.flush_window_tree(
        &env,
        Rect::new(0.0, 0.0, 24.0, 16.0),
        Affine::IDENTITY,
        Affine::IDENTITY,
    ));
    let mut first = vec![u32::MAX; 24 * 16];
    renderer
        .render_raster_0rgb_into(
            &state,
            FrameGeometry {
                display_scale: 1.0,
                width: 24,
                height: 16,
                base_color: working_color(peniko::Color::from_rgba8(0x11, 0x22, 0x33, 0xFF)),
            },
            &mut first,
        )
        .expect("initial 1x CPU window frame renders");
    assert!(
        first.iter().all(|&pixel| pixel == 0x00E0_3131),
        "1x window buffer must hold opaque 0x00RRGGBB pixels, got {:#010x}",
        first[0]
    );

    // Resize logical viewport to 32x20 at 2x display scale (64x40 physical
    // pixels): the 24x16 centered red view sits inside the #112233 base color.
    assert!(renderer.flush_window_tree(
        &env,
        Rect::new(0.0, 0.0, 32.0, 20.0),
        Affine::scale(2.0),
        Affine::IDENTITY,
    ));
    let mut resized = vec![u32::MAX; 64 * 40];
    renderer
        .render_raster_0rgb_into(
            &state,
            FrameGeometry {
                display_scale: 2.0,
                width: 64,
                height: 40,
                base_color: working_color(peniko::Color::from_rgba8(0x11, 0x22, 0x33, 0xFF)),
            },
            &mut resized,
        )
        .expect("resized 2x CPU window frame renders");
    assert_eq!(
        resized[20 * 64 + 32],
        0x00E0_3131,
        "center of resized 2x frame must render the view color"
    );
    assert_eq!(
        resized[0], 0x0011_2233,
        "margin outside the centered 24x16 view must show the window clear color"
    );
}
