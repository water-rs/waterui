//! Desktop reproduction probe for the ESP32-S3 fallback-level crash:
//! drives the exact firmware demo scene through `vello_cpu` at the scalar
//! fallback SIMD level (the only level available on Xtensa).

use kurbo::{Affine, Rect, Shape};
use peniko::Color;
use vello_cpu::{Level, Pixmap, RenderContext, RenderMode, RenderSettings, Resources};

fn render_banded(mode: RenderMode) {
    let settings = RenderSettings {
        level: Level::Fallback(fearless_simd::Fallback::new()),
        num_threads: 0,
        render_mode: mode,
    };
    let mut resources = Resources::new();
    // Full screen plus the three stacked fills, rendered band-by-band with
    // a translated transform, exactly like dew's painter.
    for band_y in (0..64u32).step_by(16) {
        let mut ctx = RenderContext::new_with(96, 16, settings);
        let shift = Affine::translate((0.0, -f64::from(band_y)));
        for (y0, y1, color) in [
            (0.0, 21.0, Color::from_rgb8(220, 30, 30)),
            (21.0, 43.0, Color::from_rgb8(30, 30, 220)),
            (43.0, 64.0, Color::from_rgb8(30, 200, 200)),
        ] {
            ctx.set_transform(shift);
            ctx.set_paint(color);
            ctx.fill_path(&Rect::new(0.0, y0, 96.0, y1).to_path(0.05));
        }
        ctx.flush();
        let mut pixmap = Pixmap::new(96, 16);
        ctx.render_to_pixmap(&mut resources, &mut pixmap);
        assert_eq!(pixmap.data()[8 * 96 + 48].a, 255, "band {band_y} rendered");
    }
}

#[test]
fn fallback_level_banded_scene_u8() {
    render_banded(RenderMode::OptimizeSpeed);
}

#[test]
fn fallback_level_banded_scene_f32() {
    render_banded(RenderMode::OptimizeQuality);
}
