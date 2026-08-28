//! Visual acceptance for the Material 3 Snackbar entrance motion.
//!
//! The PNG-producing test is ignored by default and reviewed by eye.

use core::time::Duration;
use hydrolysis_m3::install;
use waterui::snackbar::{Snackbar, SnackbarManager};
use waterui_testing::{OffscreenApp, UiBuilder};

fn save(app: &mut OffscreenApp, stage: &str) {
    let _ = app.capture_snapshot("material3-preview", "snackbar", stage);
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = install, viewport = (640, 360))]
fn snackbar_fades_and_slides_into_place(ui: UiBuilder) {
    let (manager, overlay) = SnackbarManager::new();
    let mut app = ui.mount_offscreen(move || overlay.clone());

    save(&mut app, "before");
    manager.show(
        Snackbar::new("Saved successfully")
            .duration(Duration::ZERO)
            .closeable(),
    );
    save(&mut app, "enter-0ms");
    app.pump_for(Duration::from_millis(50));
    save(&mut app, "enter-50ms");
    app.pump_for(Duration::from_millis(70));
    save(&mut app, "enter-120ms");
    app.pump_for(Duration::from_millis(180));
    save(&mut app, "settled-300ms");
}
