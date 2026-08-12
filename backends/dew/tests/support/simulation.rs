//! The embedded-target model the Dew simulation is measured against.
//!
//! Three things have to be modelled for a host run to say anything about a
//! microcontroller, and they are modelled here rather than assumed:
//!
//! 1. **Work**, not time. See [`waterui_dew::stats`]: the host and the target
//!    perform identical counts of every operation, so budgets are written
//!    against counts. A wall-clock budget measured on a development machine
//!    would be measuring the development machine.
//! 2. **Memory**, exactly. Heap behaviour transfers verbatim — the same
//!    allocations happen in the same order — so a growth check on the host is
//!    a growth check on the target, and it is the failure mode most likely to
//!    kill a real port.
//! 3. **The panel bus**, arithmetically. Pixels, address-window commands, and
//!    DMA setup all cost wire time at a documented clock.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use waterui_dew::FrameWork;

/// Tracks live and peak heap bytes.
///
/// Rust requires the global allocator to be a `static`; there is no way to
/// pass one in. The counters are therefore process-global by language
/// mandate rather than by design, and only this test binary installs it.
pub struct PeakTrackingAllocator {
    live: AtomicUsize,
    peak: AtomicUsize,
}

impl PeakTrackingAllocator {
    pub const fn new() -> Self {
        Self {
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// Bytes currently allocated and not yet freed.
    pub fn live_bytes(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }

    /// Largest live total observed since the last [`Self::reset_peak`].
    pub fn peak_bytes(&self) -> usize {
        self.peak.load(Ordering::Relaxed)
    }

    /// Restarts peak tracking from the current live total.
    pub fn reset_peak(&self) {
        self.peak.store(self.live_bytes(), Ordering::Relaxed);
    }

    fn record_allocation(&self, size: usize) {
        let live = self.live.fetch_add(size, Ordering::Relaxed) + size;
        self.peak.fetch_max(live, Ordering::Relaxed);
    }

    fn record_deallocation(&self, size: usize) {
        self.live.fetch_sub(size, Ordering::Relaxed);
    }
}

// SAFETY: every method forwards to the system allocator unchanged and only
// adds relaxed atomic bookkeeping, which cannot affect allocation validity.
unsafe impl GlobalAlloc for PeakTrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` came from the caller and is forwarded unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            self.record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        self.record_deallocation(layout.size());
        // SAFETY: `pointer` and `layout` came from the caller and are
        // forwarded unchanged to the allocator that produced the pointer.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: every argument came from the caller and is forwarded
        // unchanged to the allocator that produced the pointer.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            self.record_deallocation(layout.size());
            self.record_allocation(new_size);
        }
        new_pointer
    }
}

/// A panel's electrical interface, for turning transferred pixels into time.
///
/// Modelled explicitly because the display bus, not the CPU, is the binding
/// constraint on most embedded panels: 480×320 at 16 bpp over 40 MHz SPI is
/// 61 ms, so a full-screen repaint cannot exceed ~16 FPS no matter how fast
/// the chip is. Dew's whole dirty-region design exists to stay off that
/// ceiling, and this is what measures whether it does.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct PanelBus {
    /// Human-readable interface name, e.g. `"spi-40mhz"`.
    pub interface: &'static str,
    /// Wire clock in bits per second.
    pub bits_per_second: u64,
    /// Bits transferred per pixel.
    pub bits_per_pixel: u64,
    /// Bytes of address-window commands per region (`CASET` + `RASET` +
    /// `RAMWR` and their parameters on a typical controller).
    pub command_bytes_per_region: u64,
    /// Fixed driver cost to set up one DMA transfer.
    pub dma_setup: Duration,
}

impl PanelBus {
    /// A 40 MHz SPI RGB565 panel, the common hobby/industrial baseline.
    pub const SPI_40MHZ_RGB565: Self = Self {
        interface: "spi-40mhz-rgb565",
        bits_per_second: 40_000_000,
        bits_per_pixel: 16,
        // 5 bytes each for CASET and RASET (command + 4 coordinate bytes),
        // 1 for RAMWR.
        command_bytes_per_region: 11,
        dma_setup: Duration::from_micros(8),
    };

    /// Wire time to push one frame's worth of regions.
    ///
    /// # Panics
    ///
    /// Panics if the bit total overflows its nanosecond conversion.
    pub fn transfer_time(&self, work: &FrameWork) -> Duration {
        let pixel_bits = work.pixels_transferred * self.bits_per_pixel;
        let command_bits = work.regions_transferred * self.command_bytes_per_region * 8;
        let nanoseconds = (pixel_bits + command_bits)
            .checked_mul(1_000_000_000)
            .expect("panel transfer duration overflowed")
            / self.bits_per_second;
        Duration::from_nanos(nanoseconds)
            + self.dma_setup
                * u32::try_from(work.regions_transferred).expect("region count must fit u32")
    }

    /// Seconds a full-screen repaint would take on this bus.
    pub fn full_frame_time(&self, width: u32, height: u32) -> Duration {
        self.transfer_time(&FrameWork {
            pixels_transferred: u64::from(width) * u64::from(height),
            regions_transferred: u64::from(height),
            ..FrameWork::ZERO
        })
    }
}

/// Per-frame ceilings on [`FrameWork`], averaged over the sample.
///
/// These are the simulation's actual pass/fail criteria. Each one is a
/// deterministic count that the target reproduces exactly, so a violation on
/// the host is a violation on the chip — unlike a wall-clock threshold, which
/// only describes the machine that ran it.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct WorkBudget {
    pub text_layouts_shaped: f64,
    pub glyph_bounds_measured: f64,
    pub measures_computed: f64,
    pub commands_emitted: f64,
    pub command_band_visits: f64,
    pub pixels_rasterized: f64,
    pub pixels_transferred: f64,
}

/// One budget violation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Violation {
    pub counter: &'static str,
    pub budget: f64,
    pub measured: f64,
}

impl WorkBudget {
    /// Every per-frame counter that exceeds its ceiling.
    #[expect(
        clippy::cast_precision_loss,
        reason = "counter magnitudes stay far below f64's exact-integer range"
    )]
    pub fn violations(&self, total: &FrameWork, frames: usize) -> Vec<Violation> {
        let frames = frames as f64;
        let checks: [(&'static str, f64, u64); 7] = [
            (
                "text_layouts_shaped",
                self.text_layouts_shaped,
                total.text_layouts_shaped,
            ),
            (
                "glyph_bounds_measured",
                self.glyph_bounds_measured,
                total.glyph_bounds_measured,
            ),
            (
                "measures_computed",
                self.measures_computed,
                total.measures_computed,
            ),
            (
                "commands_emitted",
                self.commands_emitted,
                total.commands_emitted,
            ),
            (
                "command_band_visits",
                self.command_band_visits,
                total.command_band_visits,
            ),
            (
                "pixels_rasterized",
                self.pixels_rasterized,
                total.pixels_rasterized,
            ),
            (
                "pixels_transferred",
                self.pixels_transferred,
                total.pixels_transferred,
            ),
        ];
        checks
            .into_iter()
            .filter_map(|(counter, budget, measured)| {
                let measured = measured as f64 / frames;
                (measured > budget).then_some(Violation {
                    counter,
                    budget,
                    measured,
                })
            })
            .collect()
    }
}
