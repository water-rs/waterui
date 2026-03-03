use hydrolysis::HydrolysisExt;
use waterui::Color;
use waterui::View;
use waterui::env::Environment;
use waterui_graphics::{OffscreenRenderConfig, OffscreenSize};

#[derive(Clone)]
struct CloneableRect;

impl View for CloneableRect {
    fn body(self, _env: &Environment) -> impl View {
        Color::srgb_hex("#2563EB")
    }
}

#[test]
fn hydrolysis_ext_renders_offscreen() {
    let mut env = Environment::new();
    let view = CloneableRect.hydrolysis();

    let output = view
        .render_offscreen(
            OffscreenRenderConfig::new(
                OffscreenSize::try_from_pixels(400, 300).expect("static size must be valid"),
            ),
            &mut env,
        )
        .expect("hydrolysis extension offscreen render failed");

    assert_eq!(output.width, 400);
    assert_eq!(output.height, 300);
    assert_eq!(output.rgba8.len(), 400 * 300 * 4);

    let opaque_pixels = output.rgba8.chunks_exact(4).filter(|px| px[3] > 0).count();
    assert!(opaque_pixels > 4096, "rendered surface appears empty");
}
