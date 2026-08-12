//! Deterministic work accounting — Dew's machine-independent performance
//! contract.
//!
//! Wall-clock measured on a development host says nothing about a target
//! microcontroller. The host runs a different instruction set, at roughly
//! twenty times the clock, with several times the instructions-per-cycle and
//! a cache hierarchy the target does not have; a host millisecond and a
//! target millisecond are not the same currency, and no constant converts
//! between them. The divergence is not even uniform across the engine:
//! [`kurbo`] geometry is `f64` throughout, which every ESP32-class FPU
//! executes in software, so the host is *disproportionately* fast at exactly
//! the arithmetic Dew does most.
//!
//! What does transfer is the amount of work the engine asks for. How many
//! text runs it shapes, how many glyph outlines it measures, how many measure
//! calls the layout pass makes, how many draw commands the painter revisits
//! per band, how many pixels it rasterizes, how many bytes reach the panel —
//! those counts are identical on host and target, because they are properties
//! of the algorithm rather than of the machine running it. Dew therefore
//! budgets against [`FrameWork`], and the host simulation is a *work*
//! simulation, not a speed simulation.
//!
//! Two consequences are worth stating out loud:
//!
//! - A regression that doubles the number of shaped text runs is caught on
//!   the host with full confidence, because the target will shape exactly
//!   twice as many too.
//! - An absolute "this holds 60 FPS on chip X" claim is **not** established
//!   here. Turning work into time needs a per-operation cost measured on the
//!   chip itself; [`ChipBudget`] carries that conversion and records, via
//!   [`Provenance`], whether its numbers were measured or merely estimated.

use core::ops::{Add, AddAssign};

/// Work performed while producing one frame.
///
/// Every field counts an operation whose cost is roughly constant on a given
/// target, so the vector as a whole is the input to any cost model. Counts
/// are exact and reproducible: rendering the same view tree with the same
/// signal values twice yields an identical `FrameWork`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct FrameWork {
    /// Text layouts built through `parley` — itemization, font matching,
    /// shaping and line breaking. The most expensive single operation Dew
    /// performs, and the one most worth caching.
    pub text_layouts_shaped: u64,
    /// Text layouts answered from a retained layout cache instead of shaped.
    pub text_layouts_reused: u64,
    /// Glyph outline bounds read through `skrifa` while emitting glyph runs.
    /// Each one parses `glyf`/`loca` from font data that lives in flash on an
    /// embedded target, so these are also potential flash-cache misses.
    pub glyph_bounds_measured: u64,
    /// Glyph runs emitted from a retained cache without re-measuring
    /// outlines.
    pub glyph_runs_reused: u64,
    /// Node measure calls that reached a node's own measure implementation.
    /// Container layouts probe their children repeatedly, so this grows
    /// super-linearly with tree depth unless measurements are cached.
    pub measures_computed: u64,
    /// Node measure calls answered from a per-frame measurement cache.
    pub measures_reused: u64,
    /// Draw commands appended to the display list this frame.
    pub commands_emitted: u64,
    /// Path elements produced by flattening shapes into Bézier paths.
    pub path_elements_flattened: u64,
    /// `(command, band)` pairs the painter examined for intersection.
    pub command_band_visits: u64,
    /// `(command, band)` pairs the painter actually rasterized.
    pub command_band_draws: u64,
    /// Device pixels rasterized into scratch bands.
    pub pixels_rasterized: u64,
    /// Device pixels handed to the panel.
    pub pixels_transferred: u64,
    /// Separate panel window transactions. Each costs an address-window
    /// command sequence and a DMA setup on real hardware, so many small
    /// regions can be worse than one larger one.
    pub regions_transferred: u64,
}

impl FrameWork {
    /// Zero work — a clean frame.
    pub const ZERO: Self = Self {
        text_layouts_shaped: 0,
        text_layouts_reused: 0,
        glyph_bounds_measured: 0,
        glyph_runs_reused: 0,
        measures_computed: 0,
        measures_reused: 0,
        commands_emitted: 0,
        path_elements_flattened: 0,
        command_band_visits: 0,
        command_band_draws: 0,
        pixels_rasterized: 0,
        pixels_transferred: 0,
        regions_transferred: 0,
    };

    /// Fraction of measure calls served from cache, or [`None`] when the
    /// frame performed no measurement.
    #[must_use]
    pub fn measure_hit_rate(&self) -> Option<f64> {
        ratio(self.measures_reused, self.measures_computed)
    }

    /// Fraction of text layouts served from cache, or [`None`] when the frame
    /// laid out no text.
    #[must_use]
    pub fn text_hit_rate(&self) -> Option<f64> {
        ratio(self.text_layouts_reused, self.text_layouts_shaped)
    }

    /// Fraction of glyph runs emitted from cache, or [`None`] when the frame
    /// emitted none.
    #[must_use]
    pub fn glyph_run_hit_rate(&self) -> Option<f64> {
        ratio(self.glyph_runs_reused, self.glyph_bounds_measured)
    }

    /// Fraction of per-band command visits that produced an actual draw.
    ///
    /// A low rate means the painter is walking commands that cannot touch the
    /// band — the cost per-band command indexing removes.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "counter magnitudes stay far below f64's exact-integer range"
    )]
    pub fn band_visit_efficiency(&self) -> Option<f64> {
        debug_assert!(
            self.command_band_draws <= self.command_band_visits,
            "a band draw must have been preceded by a band visit"
        );
        (self.command_band_visits > 0)
            .then(|| self.command_band_draws as f64 / self.command_band_visits as f64)
    }
}

/// `hits / (hits + misses)`, or [`None`] when nothing happened.
#[expect(
    clippy::cast_precision_loss,
    reason = "counter magnitudes stay far below f64's exact-integer range"
)]
fn ratio(hits: u64, misses: u64) -> Option<f64> {
    let total = hits + misses;
    (total > 0).then(|| hits as f64 / total as f64)
}

impl Add for FrameWork {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            text_layouts_shaped: self.text_layouts_shaped + other.text_layouts_shaped,
            text_layouts_reused: self.text_layouts_reused + other.text_layouts_reused,
            glyph_bounds_measured: self.glyph_bounds_measured + other.glyph_bounds_measured,
            glyph_runs_reused: self.glyph_runs_reused + other.glyph_runs_reused,
            measures_computed: self.measures_computed + other.measures_computed,
            measures_reused: self.measures_reused + other.measures_reused,
            commands_emitted: self.commands_emitted + other.commands_emitted,
            path_elements_flattened: self.path_elements_flattened + other.path_elements_flattened,
            command_band_visits: self.command_band_visits + other.command_band_visits,
            command_band_draws: self.command_band_draws + other.command_band_draws,
            pixels_rasterized: self.pixels_rasterized + other.pixels_rasterized,
            pixels_transferred: self.pixels_transferred + other.pixels_transferred,
            regions_transferred: self.regions_transferred + other.regions_transferred,
        }
    }
}

impl AddAssign for FrameWork {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

/// How a [`ChipBudget`]'s per-operation costs were obtained.
///
/// Recorded in every report so a projection is never mistaken for a
/// measurement. A budget stays [`Provenance::Estimated`] until someone runs
/// the calibration workload on the physical chip and writes the resulting
/// costs back into the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Costs derived from published clock rates and rough instruction counts,
    /// not from running Dew on the chip. Order-of-magnitude guidance only —
    /// useful for ranking two designs, not for promising a frame rate.
    Estimated,
    /// Costs fitted to [`FrameWork`] and cycle-counter samples taken on the
    /// physical chip.
    Measured,
}

/// Cost of one unit of each kind of Dew work on a specific target, in CPU
/// cycles.
///
/// This is the only place where machine-independent work becomes a time
/// estimate, and the estimate is only as good as [`ChipBudget::provenance`]
/// says it is. Keeping the conversion in one small reviewable table is the
/// point: when a physical board exists, one calibration run replaces these
/// numbers and every existing simulation immediately reports real time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ChipBudget {
    /// Chip identifier, e.g. `"esp32s3"`.
    pub chip: &'static str,
    /// Whether the costs below were measured on hardware or estimated.
    pub provenance: Provenance,
    /// Core clock in hertz.
    pub core_hz: u64,
    /// Internal SRAM an application can realistically allocate from once
    /// the RTOS has taken its share. The fastest memory; per-band scratch
    /// and hot caches should fit here.
    pub usable_sram_bytes: u64,
    /// External PSRAM the target board maps into the malloc heap, or zero
    /// for chips/boards without it. Slower than SRAM but still ordinary
    /// heap; retained caches may spill here on real boards.
    pub usable_psram_bytes: u64,
    /// Stack the firmware render task runs on
    /// (`CONFIG_ESP_MAIN_TASK_STACK_SIZE` on ESP-IDF targets). Host
    /// simulations pump their frames on a thread with exactly this stack, so
    /// layout/shaping recursion that would overflow the chip's task overflows
    /// on the host first.
    pub main_task_stack_bytes: u64,
    /// Cycles to shape one text layout through `parley`.
    pub cycles_per_text_layout: u64,
    /// Cycles to read one glyph's outline bounds through `skrifa`.
    pub cycles_per_glyph_bounds: u64,
    /// Cycles for one node measure that reached its implementation.
    pub cycles_per_measure: u64,
    /// Cycles to emit one draw command into the display list.
    pub cycles_per_command: u64,
    /// Cycles to flatten one path element.
    pub cycles_per_path_element: u64,
    /// Cycles to examine one command against one band.
    pub cycles_per_band_visit: u64,
    /// Cycles to rasterize one device pixel.
    pub cycles_per_pixel: u64,
}

impl ChipBudget {
    /// Estimated CPU time to perform `work`, excluding panel transfer.
    ///
    /// # Panics
    ///
    /// Panics when the cycle total overflows the nanosecond conversion, which
    /// would require a frame roughly eighteen billion times larger than any
    /// real one.
    #[must_use]
    pub const fn cpu_nanoseconds(&self, work: &FrameWork) -> u64 {
        let cycles = work.text_layouts_shaped * self.cycles_per_text_layout
            + work.glyph_bounds_measured * self.cycles_per_glyph_bounds
            + work.measures_computed * self.cycles_per_measure
            + work.commands_emitted * self.cycles_per_command
            + work.path_elements_flattened * self.cycles_per_path_element
            + work.command_band_visits * self.cycles_per_band_visit
            + work.pixels_rasterized * self.cycles_per_pixel;
        cycles
            .checked_mul(1_000_000_000)
            .expect("Dew chip-budget cycle count overflowed its nanosecond conversion")
            / self.core_hz
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_adds_field_wise() {
        let mut total = FrameWork::ZERO;
        total += FrameWork {
            text_layouts_shaped: 3,
            pixels_rasterized: 100,
            ..FrameWork::ZERO
        };
        total += FrameWork {
            text_layouts_shaped: 4,
            pixels_rasterized: 20,
            ..FrameWork::ZERO
        };
        assert_eq!(total.text_layouts_shaped, 7);
        assert_eq!(total.pixels_rasterized, 120);
    }

    #[test]
    fn hit_rates_report_none_when_nothing_happened() {
        assert!(FrameWork::ZERO.measure_hit_rate().is_none());
        assert!(FrameWork::ZERO.text_hit_rate().is_none());
        assert!(FrameWork::ZERO.glyph_run_hit_rate().is_none());
        assert!(FrameWork::ZERO.band_visit_efficiency().is_none());
    }

    #[test]
    fn hit_rates_measure_cache_effectiveness() {
        let work = FrameWork {
            measures_computed: 1,
            measures_reused: 3,
            text_layouts_shaped: 1,
            text_layouts_reused: 1,
            command_band_visits: 10,
            command_band_draws: 2,
            ..FrameWork::ZERO
        };
        assert!((work.measure_hit_rate().expect("measures ran") - 0.75).abs() < f64::EPSILON);
        assert!((work.text_hit_rate().expect("text ran") - 0.5).abs() < f64::EPSILON);
        assert!((work.band_visit_efficiency().expect("bands ran") - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn cycle_cost_scales_with_the_dominant_counter() {
        let budget = ChipBudget {
            chip: "test",
            provenance: Provenance::Estimated,
            core_hz: 1_000_000_000,
            usable_sram_bytes: 320 * 1024,
            usable_psram_bytes: 0,
            main_task_stack_bytes: 160 * 1024,
            cycles_per_text_layout: 1_000,
            cycles_per_glyph_bounds: 0,
            cycles_per_measure: 0,
            cycles_per_command: 0,
            cycles_per_path_element: 0,
            cycles_per_band_visit: 0,
            cycles_per_pixel: 0,
        };
        let work = FrameWork {
            text_layouts_shaped: 1_000,
            ..FrameWork::ZERO
        };
        assert_eq!(budget.cpu_nanoseconds(&work), 1_000_000);
    }
}
