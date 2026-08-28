//! ESP-IDF runtime entry: boots a `WaterUI` [`App`] on ESP32-class chips.
//!
//! This is the firmware counterpart of the desktop runners: the generated
//! harness's `main` calls [`run`] and nothing else — ESP-IDF runtime
//! patching, logging, executors, and the frame loop all live here, so user
//! apps stay pure `WaterUI` apps that work unchanged across platforms.
//!
//! Without a selected physical board, this entry runs a headless streaming
//! panel simulation: Dew still rasterizes bands and converts them to RGB565,
//! but the simulated sink consumes each band instead of driving GPIO. It never
//! allocates a full framebuffer.

use std::time::Instant;

use waterui::app::App;

use crate::board::{Board, FontSources};
use crate::compositor::DeviceRegion;
use crate::display::{Rgb565Display, Rgb565Sink};
use crate::frame_cadence::FrameCadence;
use crate::runtime::DewRuntime;

#[derive(Debug)]
struct SimulatedPanel {
    width: u32,
    height: u32,
}

impl SimulatedPanel {
    const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl Rgb565Sink for SimulatedPanel {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn write_region(&mut self, region: DeviceRegion, pixels: &[u16]) {
        assert_eq!(
            u64::try_from(pixels.len()).expect("RGB565 band length must fit u64"),
            region.area(),
            "ESP-IDF simulation RGB565 payload must match its region"
        );
        std::hint::black_box(pixels);
    }
}

#[derive(Debug)]
struct SimulationBoard {
    display: Rgb565Display<SimulatedPanel>,
    fonts: &'static [&'static [u8]],
}

impl SimulationBoard {
    fn new(width: u32, height: u32, fonts: &'static [&'static [u8]]) -> Self {
        Self {
            display: Rgb565Display::new(SimulatedPanel::new(width, height)),
            fonts,
        }
    }
}

impl Board for SimulationBoard {
    type Display = Rgb565Display<SimulatedPanel>;

    fn display(&mut self) -> &mut Self::Display {
        &mut self.display
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn fonts(&self) -> FontSources {
        FontSources::bundled(self.fonts)
    }
}

/// Geometry of the simulated target panel.
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
/// Installs the main-thread executors that reactive `WaterUI` tasks
/// (`text!`, `Binding`/`Computed` watchers) require.
///
/// [`run`] calls this; firmware that drives a [`DewRuntime`] directly must
/// call it once before the first render.
pub fn init_executors() {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    // The reactive runtime spawns local watcher tasks from the render thread;
    // install the cooperative executor (driven by [`crate::embedded_executor::tick`])
    // rather than the polyfill's dedicated-thread backend.
    crate::embedded_executor::install();
}

/// # Panics
///
/// Panics when the app defines no main window — embedded targets render
/// exactly one window.
///
/// `fonts` are the TTF/OTF binaries text shapes with (typically
/// `include_bytes!` data, which stays in flash). Firmware has no font
/// directory to enumerate, so an app that renders text must bundle at least
/// one face; shaping with an empty bundle fails fast with an actionable
/// message.
pub fn run(app: App, panel: PanelConfig, fonts: &'static [&'static [u8]]) -> ! {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    init_executors();

    let (mut windows, _menu_bar, env) = app.into_parts();
    assert!(
        !windows.is_empty(),
        "embedded WaterUI apps must define a main window"
    );
    let content = windows.remove(0).content;

    log::info!(
        "dew[espidf-sim]: starting {}x{} RGB565 stream, {}px bands, {} bundled fonts",
        panel.width,
        panel.height,
        panel.band_height,
        fonts.len()
    );
    let board = SimulationBoard::new(panel.width, panel.height, fonts);
    let mut runtime = DewRuntime::new(board, env, panel.band_height, move || content.build());
    let mut cadence = FrameCadence::sixty_hz(Instant::now());

    loop {
        if let Some(frame) = runtime.pump() {
            log::debug!(
                "dew[espidf-sim]: frame with {} logical dirty regions, {} pixels transferred",
                frame.dirty.len(),
                frame.work.pixels_transferred
            );
        }
        // Drive reactive watcher tasks so signal changes propagate.
        crate::embedded_executor::tick();
        let now = Instant::now();
        std::thread::sleep(cadence.deadline_after_work(now).duration_since(now));
    }
}
