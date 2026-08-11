//! Visual acceptance for the MD3 switch: mdui-faithful thumb geometry, track
//! outline shrink, 200ms standard-easing value motion, and the staged
//! checked-icon transition.
//!
//! The PNG-producing tests are ignored by default and reviewed by eye.

use core::time::Duration;
use hydrolysis_m3::install;
use waterui::reactive::binding;
use waterui_controls::toggle;
use waterui_testing::{OffscreenApp, Role, ui};

fn save(app: &mut OffscreenApp, stage: &str) {
    let _ = app.capture_snapshot("material3-preview", "switch", stage);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn switch_toggle_slides_and_reveals_checkmark() {
    let value_for_view = binding(false);
    let mut app = ui()
        .viewport(200, 120)
        .theme(install)
        .mount_offscreen(move || toggle("Wi-Fi", &value_for_view));
    assert!(
        app.query()
            .role(Role::SWITCH)
            .label("Wi-Fi")
            .wait_for_existence(Duration::from_secs(3)),
        "toggle must mount"
    );
    save(&mut app, "off");
    let bounds = app
        .query()
        .role(Role::SWITCH)
        .label("Wi-Fi")
        .single()
        .bounds();
    let (cx, cy) = (
        bounds.x() + bounds.width() / 2.0,
        bounds.y() + bounds.height() / 2.0,
    );
    app.queue_pointer_down(cx, cy);
    let _ = app.snapshot();
    app.queue_pointer_up(cx, cy);
    let _ = app.snapshot();
    // Mid-transition (200ms value motion): thumb sliding, icon not yet in.
    // The pointer still hovers the switch, so the thumb shows the
    // on-surface-variant hover color per mdui.
    app.pump_for(Duration::from_millis(60));
    save(&mut app, "toggling-60ms");
    // Park the pointer away from the switch: the settled state must be free
    // of hover chrome (no dark thumb, no hover halo).
    app.queue_pointer_move(5.0, 5.0);
    app.pump_for(Duration::from_millis(500));
    save(&mut app, "on-idle");
    // Toggle back off: icon leaves within the first 50ms.
    app.queue_pointer_down(cx, cy);
    let _ = app.snapshot();
    app.queue_pointer_up(cx, cy);
    let _ = app.snapshot();
    app.pump_for(Duration::from_millis(60));
    save(&mut app, "untoggling-60ms");
    app.queue_pointer_move(5.0, 5.0);
    app.pump_for(Duration::from_millis(500));
    save(&mut app, "off-idle");
}
