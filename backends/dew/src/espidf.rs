//! ESP-IDF runtime entry: boots a `WaterUI` [`App`] on ESP32-class chips.
//!
//! This is the firmware counterpart of the desktop runners: the generated
//! harness's `main` calls [`run`] and nothing else — ESP-IDF runtime
//! patching, logging, executors, and the frame loop all live here, so user
//! apps stay pure `WaterUI` apps that work unchanged across platforms.
//!
//! The flush sink is currently an in-memory framebuffer with serial frame
//! diagnostics; panel drivers (`DisplayFlush` implementations streaming
//! RGB565 over the display bus) plug into the same loop as they land.

use core::time::Duration;

use waterui::App;

use crate::display::BufferDisplay;
use crate::runtime::DewRuntime;

/// Geometry of the target panel.
#[derive(Debug, Clone, Copy)]
pub struct PanelConfig {
    /// Panel width in pixels.
    pub width: u32,
    /// Panel height in pixels.
    pub height: u32,
    /// Maximum rows per rasterization band (bounds scratch memory).
    pub band_height: u32,
}

impl PanelConfig {
    /// Creates a panel configuration.
    #[must_use]
    pub const fn new(width: u32, height: u32, band_height: u32) -> Self {
        Self {
            width,
            height,
            band_height,
        }
    }
}

/// Boots `app` and drives its main window on this chip forever.
///
/// # Panics
///
/// Panics when the app defines no main window — embedded targets render
/// exactly one window.
pub fn run(app: App, panel: PanelConfig) -> ! {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(native_executor::NativeExecutor::new());

    let (mut windows, _menu_bar, env) = app.into_parts();
    assert!(
        !windows.is_empty(),
        "embedded WaterUI apps must define a main window"
    );
    let content = windows.remove(0).content;

    log::info!(
        "dew[espidf]: starting {}x{} panel, {}px bands",
        panel.width,
        panel.height,
        panel.band_height
    );
    let display = BufferDisplay::new(panel.width, panel.height);
    let mut runtime = DewRuntime::new(display, env, panel.band_height, move || content.build());

    loop {
        if let Some(dirty) = runtime.pump() {
            log::info!("dew[espidf]: frame with {} dirty regions", dirty.len());
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}
