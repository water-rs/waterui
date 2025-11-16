use hydrolysis::{HydrolysisRenderer, backend::TinySkiaBackend};
use tiny_skia::PixmapEncodeError;
use waterui_core::Environment;
use waterui_text::text;

fn main() -> Result<(), PixmapEncodeError> {
    let env = Environment::new();
    let backend = TinySkiaBackend::new(480, 160).expect("failed to allocate pixmap");
    let mut renderer = HydrolysisRenderer::new(backend);

    let greeting = text("Hello from Hydrolysis!").size(32.0f64);
    let result = renderer.render_view(&env, greeting);
    println!("Frame result: {result:?}");

    renderer.backend().pixmap().save_png("hydrolysis_text.png")
}
