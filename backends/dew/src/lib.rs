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

pub mod compositor;
pub mod dispatch;
pub mod display;
pub mod display_list;
pub mod painter;
pub mod runtime;
#[cfg(feature = "embedded-simulator")]
pub mod embedded_simulator;
pub mod text;

pub use compositor::{BandScheduler, DeviceRegion};
pub use dispatch::{DewRenderer, RenderContext};
pub use display::{BufferDisplay, DisplayFlush};
pub use display_list::{DisplayList, DrawCommand};
pub use painter::Painter;
pub use runtime::{DewRuntime, render_view_png};

use kurbo::Rect;

/// Rasterizes the dirty parts of `list` band-by-band and flushes them to
/// `display`, then presents the frame.
///
/// This is the per-frame composition step: `WaterUI` reactivity collects
/// `dirty` logical-pixel rects, the scheduler slices them into bands, the
/// painter rasterizes each band into a scratch pixmap, and the display
/// streams it out. Peak pixel memory is one band, not one frame.
pub fn render_frame(
    painter: &mut Painter,
    list: &DisplayList,
    scheduler: &BandScheduler,
    dirty: &[Rect],
    display: &mut impl DisplayFlush,
) {
    for region in scheduler.schedule(dirty) {
        let pixmap = painter.rasterize_region(list, region);
        display.flush_region(region, pixmap.data_as_u8_slice());
    }
    display.present();
}
