//! Visual acceptance for MD3 disabled controls: mdui-faithful disabled
//! switch/checkbox (track surface-container-highest/on-surface at 12%, thumb
//! on-surface at 38% or opaque surface, content at 38%), button containers at
//! on-surface 12% with 38% labels, and slider tracks/handle at 12%/38%.
//!
//! The PNG-producing tests are ignored by default and reviewed by eye.

use core::time::Duration;
use hydrolysis_m3::install;
use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::reactive::binding;
use waterui_controls::{button, slider::slider, toggle};
use waterui_core::View;
use waterui_testing::{OffscreenApp, Role, ui};

fn save(app: &mut OffscreenApp, stage: &str) {
    let _ = app.capture_snapshot("material3-preview", "disabled-controls", stage);
}

fn controls(checked: bool) -> impl View {
    let switch_on = binding(checked);
    let switch_off = binding(false);
    let checkbox_on = binding(checked);
    let volume = binding(0.6);
    vstack((
        toggle("Enabled switch", &switch_off),
        toggle("Disabled switch (off)", &switch_off).disabled(true),
        toggle("Disabled switch (on)", &switch_on).disabled(true),
        toggle("Disabled checkbox (on)", &checkbox_on)
            .checkbox()
            .disabled(true),
        button("Enabled button"),
        button("Disabled button").disabled(true),
        slider("Disabled volume", &volume).disabled(true),
    ))
    .padding()
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn disabled_controls_render_md3_disabled_palette() {
    let mut app = ui()
        .viewport(360, 420)
        .theme(install)
        .mount(move || controls(true));
    assert!(
        app.query()
            .role(Role::SWITCH)
            .label("Enabled switch")
            .wait_for_existence(Duration::from_secs(3)),
        "controls must mount"
    );
    save(&mut app, "idle");

    // Hover and press the disabled switch: no hover halo, no ripple, no
    // thumb growth may appear.
    let bounds = app
        .query()
        .role(Role::SWITCH)
        .label("Disabled switch (off)")
        .single()
        .bounds();
    let (cx, cy) = (
        bounds.x() + bounds.width() / 2.0,
        bounds.y() + bounds.height() / 2.0,
    );
    app.queue_pointer_move(cx, cy);
    let _ = app.snapshot();
    app.queue_pointer_down(cx, cy);
    let _ = app.snapshot();
    std::thread::sleep(Duration::from_millis(60));
    save(&mut app, "pressed-on-disabled-switch");
    app.queue_pointer_up(cx, cy);
    let _ = app.snapshot();
}
