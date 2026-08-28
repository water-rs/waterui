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

#[cfg(feature = "host")]
use std::collections::VecDeque;

use crate::accessibility::{AccessibilityActionRequest, AccessibilityTreeUpdate};
use peniko::Blob;
use vello_cpu::RenderSettings;
use waterui_backend_core::input::TouchPhase;
use waterui_backend_core::time::Instant;

#[cfg(feature = "host")]
use crate::display::BufferDisplay;
use crate::display::DisplayFlush;
use crate::painter::target_render_settings;

/// Where a board's text faces come from.
///
/// Text shaping needs at least one font, and a microcontroller has no font
/// directory to enumerate — its faces are TTF/OTF binaries linked into
/// flash. The board owns that decision because the board is the hardware
/// model: desktop hosts enumerate the system collection, firmware boards
/// hand over their flash-resident font data.
#[derive(Debug, Clone)]
pub enum FontSources {
    /// Enumerate the host's installed fonts.
    ///
    /// Only exists when the `system-fonts` feature is enabled; firmware
    /// builds disable that feature and must bundle fonts instead.
    #[cfg(feature = "system-fonts")]
    System,
    /// Register these font binaries (TTF/OTF/TTC).
    ///
    /// On embedded targets each blob typically wraps `include_bytes!` data,
    /// which stays in flash — registering it costs a handful of heap bytes
    /// of face metadata, not a copy of the font.
    Bundled(Vec<Blob<u8>>),
}

impl FontSources {
    /// Wraps flash-resident font binaries without copying them.
    #[must_use]
    pub fn bundled(fonts: &[&'static [u8]]) -> Self {
        Self::Bundled(
            fonts
                .iter()
                .map(|data| Blob::new(std::sync::Arc::new(*data)))
                .collect(),
        )
    }
}

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

    /// CPU rasterization profile for this board.
    fn render_settings(&self) -> RenderSettings {
        target_render_settings()
    }

    /// The fonts text on this board shapes with.
    ///
    /// Called once when the runtime is created. On hosts with the
    /// `system-fonts` feature this defaults to the system collection;
    /// firmware builds disable that feature, so every firmware board must
    /// say which flash-resident fonts it ships — there is nothing to
    /// enumerate on a microcontroller, and rendering text with no font is
    /// an error dew refuses to hide.
    #[cfg(feature = "system-fonts")]
    fn fonts(&self) -> FontSources {
        FontSources::System
    }

    /// The fonts text on this board shapes with.
    ///
    /// This build has no `system-fonts` feature, so there is no default:
    /// the board must provide its bundled font binaries (usually
    /// `include_bytes!` data via [`FontSources::bundled`]).
    #[cfg(not(feature = "system-fonts"))]
    fn fonts(&self) -> FontSources;

    /// Returns the next pending pointer event, or [`None`] when the board has
    /// no input device or no event is queued.
    fn poll_pointer(&mut self) -> Option<PointerSample> {
        None
    }

    /// Whether this board has an assistive interface that consumes semantic
    /// tree updates and can return actions.
    fn supports_accessibility(&self) -> bool {
        false
    }

    /// Publishes the complete semantic tree for the rendered frame.
    fn update_accessibility(&mut self, _update: &AccessibilityTreeUpdate) {}

    /// Returns the next action requested by the board's assistive interface.
    fn poll_accessibility_action(&mut self) -> Option<AccessibilityActionRequest> {
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
#[cfg(feature = "host")]
#[derive(Debug, Clone)]
pub struct HostBoard {
    display: BufferDisplay,
    pointer_samples: VecDeque<PointerSample>,
    accessibility_actions: VecDeque<AccessibilityActionRequest>,
    accessibility_tree: Option<AccessibilityTreeUpdate>,
    fonts: Vec<Blob<u8>>,
}

#[cfg(feature = "host")]
impl HostBoard {
    /// Creates a host board with a `width` × `height` framebuffer.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            display: BufferDisplay::new(width, height),
            pointer_samples: VecDeque::new(),
            accessibility_actions: VecDeque::new(),
            accessibility_tree: None,
            fonts: Vec::new(),
        }
    }

    /// Registers a bundled font, replacing system enumeration.
    ///
    /// With at least one registered font the board shapes text exactly like
    /// a firmware board: the same binaries, no system collection. This is
    /// what makes a host simulation's font path — and therefore its heap
    /// profile — match the embedded target it stands in for.
    #[must_use]
    pub fn with_font(mut self, data: impl Into<Blob<u8>>) -> Self {
        self.fonts.push(data.into());
        self
    }

    /// The framebuffer the engine renders into.
    #[must_use]
    pub const fn framebuffer(&self) -> &BufferDisplay {
        &self.display
    }

    /// Enqueues one host pointer sample for the next runtime pump.
    pub fn push_pointer(&mut self, sample: PointerSample) {
        self.pointer_samples.push_back(sample);
    }

    /// The most recently published semantic tree.
    #[must_use]
    pub const fn accessibility_tree(&self) -> Option<&AccessibilityTreeUpdate> {
        self.accessibility_tree.as_ref()
    }

    /// Enqueues an assistive-technology action for the next pump.
    pub fn push_accessibility_action(&mut self, request: AccessibilityActionRequest) {
        self.accessibility_actions.push_back(request);
    }
}

#[cfg(feature = "host")]
impl Board for HostBoard {
    type Display = BufferDisplay;

    fn display(&mut self) -> &mut BufferDisplay {
        &mut self.display
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn fonts(&self) -> FontSources {
        #[cfg(feature = "system-fonts")]
        if self.fonts.is_empty() {
            return FontSources::System;
        }
        FontSources::Bundled(self.fonts.clone())
    }

    fn poll_pointer(&mut self) -> Option<PointerSample> {
        self.pointer_samples.pop_front()
    }

    fn supports_accessibility(&self) -> bool {
        true
    }

    fn update_accessibility(&mut self, update: &AccessibilityTreeUpdate) {
        self.accessibility_tree = Some(update.clone());
    }

    fn poll_accessibility_action(&mut self) -> Option<AccessibilityActionRequest> {
        self.accessibility_actions.pop_front()
    }
}
