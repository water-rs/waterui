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
//!   region-sized scratch pixmap, and implements `waterui-graphics`'
//!   engine-neutral `Scene2D` over it, which is what lets `Canvas` drawings
//!   and SVG documents render here with no engine of their own
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
//!
//! Self-drawn *scene* content is the deliberate exception, and not a GPU
//! primitive at all: a `SceneView` draws through the engine-neutral `Scene2D`
//! contract, so dew installs `SceneViewMergeToParent` and draws it on the CPU
//! rather than letting it fall back to a GPU surface.
//!
//! # Interaction beyond controls: the `gestures` feature
//!
//! Controls (buttons, toggles, sliders, tabs) hit-test through
//! [`pointer::PointerRouter`] and are always available. The richer pointer
//! semantics a view asks for with `.gesture(...)` / `.on_hover_*` —
//! `Metadata<GestureObserver>` and `Metadata<OnEvent>`, which is what makes an
//! interactive chart interactive — recognize through `waterui-backend-core`'s
//! shared state machines, and that pulls the `waterui` facade the gesture event
//! payloads live in. They are therefore behind the default-on `gestures`
//! feature, gated for the same reason `progress` is: a firmware graph built
//! with `default-features = false` stays free of the facade. A build without
//! the feature fails fast when such a view reaches dispatch, naming the
//! feature, rather than drawing a view that silently never responds.

pub mod accessibility;
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
#[cfg(feature = "gestures")]
mod interaction;
pub mod painter;
mod pointer;
pub mod runtime;
pub mod stats;
pub mod text;
pub mod theme;
mod views;

pub use accessibility::{AccessibilityActionRequest, AccessibilityTreeUpdate};
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

/// The faces this crate's unit tests shape with: the host collection, exactly
/// as `HostBoard` resolves it on a desktop build.
#[cfg(all(test, feature = "system-fonts"))]
pub(crate) const fn test_fonts() -> FontSources {
    FontSources::System
}

/// The faces this crate's unit tests shape with: the repository's own test
/// binaries, registered the way a firmware board registers flash-resident
/// fonts.
///
/// This build has no `system-fonts` feature and therefore no
/// `FontSources::System` — the same asymmetry [`Board::fonts`] is declared
/// twice for. Bundling here is what keeps the shaping tests *running* in the
/// configuration a device ships instead of being gated away with it. Regular
/// and bold are both registered because the styled-span tests assert that a
/// bold run separates from the surrounding body text.
///
/// [`Board::fonts`]: board::Board::fonts
#[cfg(all(test, not(feature = "system-fonts")))]
pub(crate) fn test_fonts() -> FontSources {
    FontSources::bundled(&[
        include_bytes!("../../../testing/fonts/Roboto-Regular.ttf"),
        include_bytes!("../../../testing/fonts/Roboto-Bold.ttf"),
    ])
}

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
