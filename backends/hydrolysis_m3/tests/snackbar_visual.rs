//! Visual acceptance for the Material 3 Snackbar entrance motion.
//!
//! The PNG-producing test is ignored by default and reviewed by eye.

use core::time::Duration;
use hydrolysis_m3::install;
use waterui::snackbar::{Snackbar, SnackbarManager};
use waterui_testing::{OffscreenApp, ui};

fn save(app: &mut OffscreenApp, stage: &str) {
    let _ = app.capture_snapshot("material3-preview", "snackbar", stage);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn snackbar_fades_and_slides_into_place() {
    let (manager, overlay) = SnackbarManager::new();
    let mut app = ui()
        .viewport(640, 360)
        .theme(install)
        .mount_offscreen(move || overlay.clone());

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
