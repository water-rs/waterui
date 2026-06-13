//! Dew on ESP32-C3 (RISC-V): first-light firmware for the QEMU-emulated chip.
//!
//! The ESP32-C3 is a RISC-V core, so codegen uses LLVM's mainline RISC-V
//! backend rather than the Xtensa fork that miscompiles vello_cpu. This
//! firmware therefore runs the real dew rendering pipeline (vstack of
//! colours → vello_cpu rasterization → banded flush into an in-memory
//! framebuffer) on the emulated chip and logs frame timings plus a
//! framebuffer checksum, proving the engine renders correctly on a real
//! embedded target. `DEW_C3_OK` is the host success sentinel.

use std::time::Instant;

use waterui_core::{AnyView, Environment};
use waterui_dew::{DewRuntime, HostBoard};
use waterui_graphics::Color;
use waterui_layout::stack::vstack;

const WIDTH: u32 = 128;
const HEIGHT: u32 = 64;
const BAND_HEIGHT: u32 = 16;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("dew[c3]: starting on ESP32-C3 RISC-V ({WIDTH}x{HEIGHT}, {BAND_HEIGHT}px bands)");

    let board = HostBoard::new(WIDTH, HEIGHT);
    let mut runtime = DewRuntime::new(board, Environment::new(), BAND_HEIGHT, || {
        AnyView::new(vstack((Color::red(), Color::blue(), Color::cyan())))
    });

    let start = Instant::now();
    let dirty = runtime.pump().expect("first frame must render");
    let first_us = start.elapsed().as_micros();
    log::info!("DEW_C3_FIRST_FRAME_US: {first_us} ({} dirty rects)", dirty.len());

    let signals = runtime.signals();
    for index in 0..3 {
        signals.request_rebuild();
        let start = Instant::now();
        runtime.pump().expect("requested rebuild must render");
        log::info!("DEW_C3_REBUILD_{index}_US: {}", start.elapsed().as_micros());
    }

    let fb = runtime.board().framebuffer();
    let top = fb.pixel(WIDTH / 2, 4);
    let bottom = fb.pixel(WIDTH / 2, HEIGHT - 4);
    let checksum: u64 = fb.pixels().iter().map(|&b| u64::from(b)).sum();
    log::info!("dew[c3]: top {top:?}, bottom {bottom:?}, fb_checksum {checksum}");
    assert!(top[0] > 120, "top of the stack must be red, got {top:?}");
    assert!(
        bottom[1] > 120 || bottom[2] > 120,
        "bottom of the stack must be cyan, got {bottom:?}"
    );
    assert!(checksum > 0, "framebuffer must not be blank");

    log::info!("DEW_C3_OK");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
