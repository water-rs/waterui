//! Visual acceptance for the Material 3 Expressive slider.
//!
//! Expressive replaced the circular thumb on a hairline track with a narrow
//! vertical bar on a thick track, split by a gap around the handle and marked
//! with a stop indicator at the far end. The PNG is reviewed by eye.

use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_controls::slider::slider;
use waterui_testing::{OffscreenApp, UiBuilder};

fn sliders() -> impl View {
    let quiet = binding(0.0_f64);
    let low = binding(0.25_f64);
    let mid = binding(0.5_f64);
    let full = binding(1.0_f64);
    vstack((
        slider("Zero", &quiet),
        slider("Low", &low),
        slider("Mid", &mid),
        slider("Full", &full),
    ))
    .spacing(16.0)
    .padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (420, 320))]
fn slider_renders_the_expressive_bar_handle(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(sliders);
    let _ = app.capture_snapshot("material3-preview", "slider", "expressive");
}
