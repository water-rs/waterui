//! Dew — `WaterUI`'s embedded-first CPU rendering backend.
//!
//! Dew renders `WaterUI` views without a GPU, targeting microcontrollers
//! (e.g. ESP32-S3 driving an SPI/parallel LCD) while staying fully testable
//! on desktop. It pairs the anti-aliased vector quality of [`vello_cpu`]
//! (sparse-strip rasterization, the CPU sibling of the GPU renderer used by
//! hydrolysis) with a Slint-style memory discipline: the screen is never
//! required to exist as a full-resolution framebuffer. Instead, dirty
//! regions are rasterized band-by-band into a small scratch pixmap and
//! streamed to the display.
//!
//! The crate speaks the same geometry/paint vocabulary as hydrolysis
//! ([`kurbo`] paths, [`peniko`] brushes — identical versions), and shares the
//! interaction runtime (gestures, scrolling, frame economy) through
//! `waterui-backend-core`.
//!
//! Module map:
//!
//! - [`display_list`]: the retained scene — draw commands in kurbo/peniko
//!   vocabulary, replayable into any sub-region of the screen
//! - [`compositor`]: dirty-region tracking and band scheduling — decides
//!   which device-pixel regions must be re-rasterized this frame
//! - [`painter`]: the `vello_cpu` bridge — rasterizes a display list into a
//!   region-sized scratch pixmap
//! - [`display`]: the flush boundary — where rasterized regions leave the
//!   renderer toward a concrete screen (in-memory buffer on desktop,
//!   RGB565 LCD stream on embedded targets)
//! - [`theme`]: the built-in widget palette — named colors every handler
//!   draws with until environment-driven theming lands
//!
//! # Deliberately unsupported: the GPU stack
//!
//! Dew's dependency graph is wgpu-free by design, and the GPU-backed
//! primitives (`GpuSurface`, `ShaderSurface`, `ViewEffect`, GPU
//! `AppliedFilter`s) are explicitly unsupported rather than emulated —
//! Dew targets devices without a GPU, and surfacing that asymmetry is the
//! framework's documented policy. Build Dew apps without the `waterui/gpu`
//! feature; a GPU view reaching the dispatcher fails fast through
//! `Native::body`. The widget handlers' style panics
//! (`dew does not implement …`) are the same kind of authored marker: a
//! feature awaiting a real Dew implementation, never a silent degradation.

pub mod board;
pub mod compositor;
pub mod dispatch;
pub mod display;
pub mod display_list;
// Used by the espidf firmware loop and by the desktop simulator: both own a
// loop and drive this executor themselves.
#[cfg(any(
    all(feature = "espidf", target_os = "espidf"),
    feature = "embedded-simulator"
))]
pub mod embedded_executor;
#[cfg(feature = "embedded-simulator")]
pub mod embedded_simulator;
#[cfg(all(feature = "espidf", target_os = "espidf"))]
pub mod espidf;
#[cfg(any(
    test,
    feature = "embedded-simulator",
    all(feature = "espidf", target_os = "espidf")
))]
pub(crate) mod frame_cadence;
pub mod painter;
mod pointer;
pub mod runtime;
pub mod stats;
pub mod text;
pub mod theme;
mod views;

#[cfg(feature = "host")]
pub use board::HostBoard;
pub use board::{Board, FontSources, PointerSample};
pub use compositor::{BandIndex, BandScheduler, DeviceRegion};
pub use dispatch::{DewRenderer, RenderContext};
#[cfg(feature = "host")]
pub use display::BufferDisplay;
pub use display::{DisplayFlush, Rgb565Display, Rgb565Sink};
pub use display_list::{Clip, ClipRegion, DisplayList, DrawCommand, PlacedCommand};
pub use painter::Painter;
#[cfg(feature = "host")]
pub use runtime::render_view_png;
pub use runtime::{DewRuntime, Frame};
pub use stats::{ChipBudget, FrameWork, Provenance};

use kurbo::Rect;

/// Rasterizes the dirty parts of `list` band-by-band and flushes them to
/// `display`, then presents the frame, accumulating the work performed into
/// `work`.
///
/// This is the per-frame composition step: `WaterUI` reactivity collects
/// `dirty` logical-pixel rects, the scheduler slices them into bands, a
/// [`BandIndex`] narrows each band to the commands that can appear in it, the
/// painter rasterizes it into a scratch pixmap, and the display streams it
/// out. Peak pixel memory is one band, not one frame.
pub fn render_frame(
    painter: &mut Painter,
    list: &DisplayList,
    scheduler: &BandScheduler,
    dirty: &[Rect],
    display: &mut impl DisplayFlush,
    work: &mut FrameWork,
) {
    let regions = scheduler.schedule(dirty);
    if regions.is_empty() {
        display.present();
        return;
    }
    let (_, screen_height) = display.size();
    let index = BandIndex::build(list, screen_height, scheduler.band_height());
    for region in regions {
        let pixmap = painter.rasterize_region(list, region, index.candidates(region), work);
        work.pixels_transferred += region.area();
        work.regions_transferred += 1;
        display.flush_region(region, pixmap.data_as_u8_slice());
    }
    display.present();
}
