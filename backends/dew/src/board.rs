//! The board abstraction: the hardware substrate the dew engine runs on.
//!
//! The dew engine (renderer, painter, compositor, runtime) is decoupled from
//! any specific chip by the [`Board`] trait, which bundles the platform
//! concerns the engine needs: a display sink, a monotonic clock, and an
//! optional pointer input device. [`DewRuntime`](crate::DewRuntime) is
//! generic over a `Board`, so the identical engine runs on a development
//! machine ([`HostBoard`]) and on a real chip (e.g. an ESP32 board with an
//! RGB565 panel and a touch controller) with no change to the engine or the
//! `WaterUI` app.
//!
//! [`HostBoard`] is the reference implementation and the primary way to test
//! the engine: it renders into an in-memory framebuffer driven by the system
//! clock, requiring no cross-compilation or hardware.

use waterui_backend_core::input::TouchPhase;
use waterui_backend_core::time::Instant;

use crate::display::{BufferDisplay, DisplayFlush};

/// A pointer/touch sample from the board's input device, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerSample {
    /// Horizontal position in device pixels from the left edge.
    pub x: f64,
    /// Vertical position in device pixels from the top edge.
    pub y: f64,
    /// Lifecycle phase of this pointer sample.
    pub phase: TouchPhase,
}

/// The hardware substrate the dew engine runs on.
///
/// Bundles the display sink, the monotonic clock that drives animations, and
/// an optional pointer device. Implement this once per target; the engine is
/// generic over it. The default [`Board::poll_pointer`] reports no input, so
/// display-only boards need only provide [`Board::display`] and
/// [`Board::now`].
pub trait Board {
    /// The display sink that rasterized regions are flushed to.
    type Display: DisplayFlush;

    /// Mutable access to the display sink.
    fn display(&mut self) -> &mut Self::Display;

    /// The current monotonic time, used to advance animations.
    fn now(&self) -> Instant;

    /// Returns the next pending pointer event, or [`None`] when the board has
    /// no input device or no event is queued.
    fn poll_pointer(&mut self) -> Option<PointerSample> {
        None
    }
}

/// Host board: an in-memory framebuffer driven by the system clock.
///
/// The reference [`Board`] for running and testing the dew engine on a
/// development machine without cross-compilation. The framebuffer it renders
/// into is inspectable via [`HostBoard::framebuffer`] for snapshot tests and
/// for presentation by a desktop window (see the `embedded-simulator`
/// feature).
#[derive(Debug, Clone)]
pub struct HostBoard {
    display: BufferDisplay,
}

impl HostBoard {
    /// Creates a host board with a `width` × `height` framebuffer.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            display: BufferDisplay::new(width, height),
        }
    }

    /// The framebuffer the engine renders into.
    #[must_use]
    pub const fn framebuffer(&self) -> &BufferDisplay {
        &self.display
    }
}

impl Board for HostBoard {
    type Display = BufferDisplay;

    fn display(&mut self) -> &mut BufferDisplay {
        &mut self.display
    }

    fn now(&self) -> Instant {
        Instant::now()
    }
}
