//! Dew on ESP32-S3: first-light firmware for the QEMU-emulated chip.
//!
//! Renders a `WaterUI` view tree through the full dew pipeline (dispatch →
//! layout → vello_cpu rasterization → banded flush) into an in-memory
//! framebuffer on the Xtensa core, logging frame timings over the serial
//! console. The `DEW_QEMU_OK` sentinel line lets the host assert success.
//!
//! Text is intentionally absent: there is no system font collection on
//! ESP-IDF yet (bundled-font registration is the follow-up).

use std::time::Instant;

use waterui_core::{AnyView, Environment};
use waterui_dew::{BufferDisplay, DewRuntime};
use waterui_graphics::Color;
use waterui_layout::stack::vstack;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;
const BAND_HEIGHT: u32 = 16;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("dew: starting on ESP32-S3 ({WIDTH}x{HEIGHT}, {BAND_HEIGHT}px bands)");

    // Minimal vello_cpu probes, narrowing the Xtensa miscompile trigger.
    {
        use kurbo::Shape;
        let mut resources = vello_cpu::Resources::new();
        let mut pixmap = vello_cpu::Pixmap::new(96, 16);

        let ctx = vello_cpu::RenderContext::new(96, 16);
        ctx.render_to_pixmap(&mut resources, &mut pixmap);
        log::info!("PROBE_EMPTY_OK");

        let mut ctx = vello_cpu::RenderContext::new(96, 16);
        ctx.set_paint(peniko::Color::from_rgb8(200, 30, 30));
        ctx.fill_path(&kurbo::Rect::new(0.0, 0.0, 96.0, 16.0).to_path(0.05));
        ctx.flush();
        ctx.render_to_pixmap(&mut resources, &mut pixmap);
        log::info!("PROBE_ONE_FILL_OK ({:?})", pixmap.data()[0]);
    }

    let display = BufferDisplay::new(WIDTH, HEIGHT);
    let mut runtime = DewRuntime::new(display, Environment::new(), BAND_HEIGHT, || {
        AnyView::new(vstack((Color::red(), Color::blue(), Color::cyan())))
    });

    let start = Instant::now();
    let dirty = runtime.pump().expect("first frame must render");
    let first_frame_us = start.elapsed().as_micros();
    log::info!(
        "DEW_FIRST_FRAME_US: {first_frame_us} ({} dirty rects)",
        dirty.len()
    );

    let signals = runtime.signals();
    for index in 0..3 {
        signals.request_rebuild();
        let start = Instant::now();
        runtime.pump().expect("requested rebuild must render");
        log::info!("DEW_REBUILD_{index}_US: {}", start.elapsed().as_micros());
    }

    let top = runtime.display().pixel(WIDTH / 2, 5);
    let bottom = runtime.display().pixel(WIDTH / 2, HEIGHT - 5);
    log::info!("dew: top pixel {top:?}, bottom pixel {bottom:?}");
    assert!(top[0] > 120, "top of the stack must be red, got {top:?}");
    assert!(
        bottom[2] > 120 || bottom[1] > 120,
        "bottom of the stack must be cyan, got {bottom:?}"
    );

    log::info!("DEW_QEMU_OK");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
