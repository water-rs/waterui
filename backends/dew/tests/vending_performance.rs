//! Work-budgeted embedded simulation for Dew.
//!
//! The workload models a 480×320 vending-machine panel with twelve product
//! tiles, a live order summary, and a continuously updating payment-progress
//! bar, driven single-threaded through the scalar fallback rasterizer.
//!
//! What makes it a *simulation* rather than a benchmark is what it asserts on.
//! Host wall-clock is recorded but is never a pass criterion: a development
//! machine runs a different instruction set at twenty times the clock, and it
//! executes `kurbo`'s `f64` geometry in hardware where every ESP32-class FPU
//! is single-precision and emulates it in software. No constant converts one
//! to the other. What does transfer exactly is the *amount of work* the engine
//! asks for, the heap it holds, and the bytes it pushes down the panel bus, so
//! those are the budgets. See [`waterui_dew::stats`] and `support::simulation`.
//!
//! The projected on-chip frame time in the report is exactly that — a
//! projection from an estimated cost table, labelled as such. Calibrate
//! [`ChipBudget`] against real hardware to turn it into a measurement.

use std::collections::VecDeque;
use std::time::{Duration, Instant as StdInstant};

use nami::{Binding, binding};
use peniko::Blob;
use vello_cpu::{Level, RenderMode, RenderSettings};
use waterui::prelude::*;
use waterui_backend_core::input::TouchPhase;
use waterui_backend_core::time::Instant;
use waterui_core::AnyView;
use waterui_dew::{
    Board, ChipBudget, DeviceRegion, DewRuntime, FontSources, FrameWork, PointerSample, Provenance,
    Rgb565Display, Rgb565Sink,
};
use waterui_layout::frame::Frame;

mod support;

use support::simulation::{MemoryBudget, PanelBus, PeakTrackingAllocator, Violation, WorkBudget};

use core::prelude::v1::test;

/// Heap accounting for the simulation. Rust requires a `static` here.
#[global_allocator]
static ALLOCATOR: PeakTrackingAllocator = PeakTrackingAllocator::new();

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const DEFAULT_BAND_HEIGHT: u32 = 16;
const WARMUP_FRAMES: usize = 120;
const DEFAULT_SAMPLE_FRAMES: usize = 3_600;
const INTERACTION_PERIOD: usize = 120;

/// The flash-bundled face the simulated board ships, bundled for real.
///
/// This used to probe the host for Arial or `DejaVu`, so the shaping work — and
/// therefore the work budgets below — depended on which OS ran the test:
/// numbers calibrated against one host's font tripped the gate on another's
/// (issue #153). The repository's deterministic test font is compiled in
/// instead, exactly the way firmware bundles its face: system enumeration
/// stays off, and every platform shapes the same glyphs from the same tables.
fn load_simulation_font() -> Vec<u8> {
    include_bytes!("../../../testing/fonts/Roboto-Regular.ttf").to_vec()
}

/// Steady-state heap retention tolerated, in bytes per sampled frame.
///
/// Expressed per frame rather than as a lump sum so the gate does not loosen
/// as the sample lengthens: genuine per-frame retention keeps the same
/// per-frame rate at any frame count, while cache saturation — which is what
/// this workload actually exhibits — sees its rate fall as frames increase.
///
/// Measured behaviour on this screen, for reference: 81 B/frame over 200
/// frames, 50 over 600, 34 over 1800. Decreasing, i.e. bounded. It has not
/// been soaked long enough to observe the plateau outright, so this ceiling
/// is set to catch a *linear* leak rather than to certify the bound.
const MAX_HEAP_RETENTION_BYTES_PER_FRAME: u64 = 128;

/// Name of the rasterizer pipeline the simulated target runs.
const TARGET_RENDER_MODE_NAME: &str = "OptimizeQuality (f32 pipeline)";

/// Cost model for the chip this simulation projects onto.
///
/// The numbers are ESTIMATES derived from the chip's clock and the rough
/// instruction counts of each operation — good enough to rank two designs,
/// not to promise a frame rate. [`Provenance::Estimated`] says so in every
/// report. Replace them with a fit against on-device cycle counters and flip
/// the provenance to [`Provenance::Measured`]; nothing else has to change.
const fn target_chip() -> ChipBudget {
    ChipBudget {
        chip: "esp32s3",
        provenance: Provenance::Estimated,
        core_hz: 240_000_000,
        // Internal SRAM an application can realistically allocate once
        // ESP-IDF has taken its share of the 512 KiB.
        usable_sram_bytes: 320 * 1024,
        // The devkit norm for this chip (N16R8: 8 MiB octal PSRAM), mapped
        // into the malloc heap by ESP-IDF's CONFIG_SPIRAM_USE_MALLOC.
        usable_psram_bytes: 8 * 1024 * 1024,
        // CONFIG_ESP_MAIN_TASK_STACK_SIZE the water CLI configures for this
        // chip (cli/src/esp32/chip.rs); the simulation pumps frames on a
        // thread with exactly this stack.
        main_task_stack_bytes: 163_840,
        // parley shaping: itemization, font matching, glyph positioning.
        cycles_per_text_layout: 120_000,
        // A skrifa outline-bounds read, including a likely flash-cache miss.
        cycles_per_glyph_bounds: 2_500,
        cycles_per_measure: 1_200,
        cycles_per_command: 600,
        // Soft-float f64 dominates flattening on a single-precision FPU.
        cycles_per_path_element: 400,
        cycles_per_band_visit: 120,
        cycles_per_pixel: 60,
    }
}

/// Per-frame work ceilings for this screen.
///
/// Each is set a little above the measured steady state, so an algorithmic
/// regression trips it while ordinary variation does not.
const fn work_budget() -> WorkBudget {
    WorkBudget {
        // Only genuinely changing text reshapes: the order total and the
        // authorization percentage, at the handful of widths layout proposes.
        text_layouts_shaped: 8.0,
        // Retained glyph runs mean only changed text re-reads outlines.
        glyph_bounds_measured: 40.0,
        measures_computed: 200.0,
        commands_emitted: 60.0,
        // The band index must keep this near the number of commands that can
        // actually appear in the dirty bands, not commands x bands.
        command_band_visits: 60.0,
        // Dirty-region rasterization: a tiny fraction of the 153,600-pixel
        // panel. Anything approaching full-screen means the diff has broken.
        pixels_rasterized: 4_000.0,
        pixels_transferred: 4_000.0,
    }
}

/// Heap ceilings for this screen, set a little above the measured steady
/// state like the work budgets — regression tripwires, not fit certificates.
///
/// Where the workload actually stands (host, 64-bit, so target-conservative):
/// steady-state peak live heap after subtracting the font blob measured
/// ~740 KiB. Broken down by experiment: ~330 KiB is the retained node tree,
/// text caches, and display list; ~100 KiB is the rasterizer's retained
/// resources; the rest is baseline plus transients. On a 32-bit target that
/// projects to roughly half — which does NOT fit an ESP32-S3's ~320 KiB of
/// usable internal SRAM, and is why the chip model carries the devkit's
/// PSRAM: boards for this screen class need PSRAM in the malloc heap (the
/// S3 N16R8 norm), or the retained tree needs another round of slimming.
/// Per-band scratch (30 KiB) and hot caches still fit SRAM comfortably.
const fn memory_budget() -> MemoryBudget {
    MemoryBudget {
        steady_peak_heap_bytes: 792 * 1024,
        allocations_per_frame: 1_050.0,
        allocated_bytes_per_frame: 256.0 * 1024.0,
    }
}

#[derive(Debug)]
struct StreamingPanel {
    width: u32,
    height: u32,
}

impl StreamingPanel {
    const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl Rgb565Sink for StreamingPanel {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn write_region(&mut self, region: DeviceRegion, pixels: &[u16]) {
        assert!(
            region.x + region.width <= self.width && region.y + region.height <= self.height,
            "simulated panel region {region:?} exceeds {}x{}",
            self.width,
            self.height
        );
        let pixel_count = usize::try_from(region.area()).expect("panel region area must fit usize");
        assert_eq!(
            pixels.len(),
            pixel_count,
            "simulated panel RGB565 payload must match the flushed region"
        );
        // The panel consumes the band and keeps no framebuffer; `black_box`
        // stops the optimizer from eliding the RGB565 conversion that a real
        // device would pay for.
        std::hint::black_box(pixels);
    }
}

#[derive(Debug)]
struct SimulatedEmbeddedBoard {
    display: Rgb565Display<StreamingPanel>,
    pointer_samples: VecDeque<PointerSample>,
    /// The stand-in flash-bundled font. On the host this blob lives in heap,
    /// so its size is subtracted from the peak-heap gate.
    font: Blob<u8>,
}

impl SimulatedEmbeddedBoard {
    fn new() -> Self {
        Self {
            display: Rgb565Display::new(StreamingPanel::new(WIDTH, HEIGHT)),
            pointer_samples: VecDeque::new(),
            font: Blob::from(load_simulation_font()),
        }
    }

    fn click(&mut self, x: f64, y: f64) {
        self.pointer_samples.push_back(PointerSample {
            x,
            y,
            phase: TouchPhase::Started,
        });
        self.pointer_samples.push_back(PointerSample {
            x,
            y,
            phase: TouchPhase::Ended,
        });
    }
}

#[derive(Clone)]
struct VendingState {
    progress_value: Binding<f64>,
    progress_percent: Binding<i32>,
    total_cents: Binding<i32>,
    selected_product: Binding<&'static str>,
}

impl VendingState {
    fn new() -> Self {
        Self {
            progress_value: binding(0.0),
            progress_percent: binding(0),
            total_cents: binding(125),
            selected_product: binding("Spring Water"),
        }
    }

    fn update_animation(&self, frame: usize) {
        let percent = i32::try_from(frame % 101).expect("payment percentage must fit i32");
        self.progress_value.set(f64::from(percent) / 100.0);
        self.progress_percent.set(percent);
    }
}

impl Board for SimulatedEmbeddedBoard {
    type Display = Rgb565Display<StreamingPanel>;

    fn display(&mut self) -> &mut Self::Display {
        &mut self.display
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn render_settings(&self) -> RenderSettings {
        RenderSettings {
            // No SIMD and no worker threads: an ESP32-class core has neither.
            level: Level::fallback(),
            num_threads: 0,
            // The pipeline the target actually runs. `painter.rs` forces the
            // f32 path on Xtensa because the Xtensa LLVM backend miscompiles
            // vello_cpu's u8/u16 fine kernels, so simulating the faster u8
            // path would measure code the chip will never execute.
            render_mode: RenderMode::OptimizeQuality,
        }
    }

    fn fonts(&self) -> FontSources {
        FontSources::Bundled(vec![self.font.clone()])
    }

    fn poll_pointer(&mut self) -> Option<PointerSample> {
        self.pointer_samples.pop_front()
    }
}

fn product_tile(
    state: &VendingState,
    name: &'static str,
    price: &'static str,
    cents: i32,
    color: Color,
) -> impl View + use<> {
    let selected_product = state.selected_product.clone();
    let total_cents = state.total_cents.clone();
    Frame::new(zstack((
        color,
        Frame::new(button(name).hide_label().borderless().action(move || {
            selected_product.set(name);
            total_cents.set(cents);
        }))
        .width(104)
        .height(58),
        vstack((text(name).bold(), text(price).caption()))
            .spacing(0.0)
            .padding_with(4.0),
    )))
    .width(104)
    .height(58)
}

#[expect(
    clippy::too_many_lines,
    reason = "the commercial test screen declares all twelve distinct products explicitly"
)]
fn vending_screen(state: &VendingState) -> impl View + use<> {
    let progress_value = state.progress_value.clone();
    let progress_percent = state.progress_percent.clone();
    let total_cents = state.total_cents.clone();
    let selected_product = state.selected_product.clone();
    let header = Frame::new(hstack((
        text("FRESH MARKET").title(),
        spacer(),
        text!("Order A07 · {total_cents} cents"),
    )))
    .height(36);

    let products = vstack((
        hstack((
            product_tile(
                state,
                "Spring Water",
                "125 cents",
                125,
                Color::srgb(217, 241, 255),
            ),
            product_tile(
                state,
                "Sparkling Lime",
                "175 cents",
                175,
                Color::srgb(220, 255, 230),
            ),
            product_tile(
                state,
                "Cold Brew",
                "325 cents",
                325,
                Color::srgb(232, 220, 211),
            ),
            product_tile(
                state,
                "Green Tea",
                "225 cents",
                225,
                Color::srgb(225, 245, 214),
            ),
        ))
        .spacing(4.0),
        hstack((
            product_tile(
                state,
                "Sea Salt Chips",
                "200 cents",
                200,
                Color::srgb(255, 239, 194),
            ),
            product_tile(
                state,
                "Trail Mix",
                "275 cents",
                275,
                Color::srgb(242, 223, 194),
            ),
            product_tile(
                state,
                "Oat Bar",
                "190 cents",
                190,
                Color::srgb(255, 230, 208),
            ),
            product_tile(
                state,
                "Dark Chocolate",
                "250 cents",
                250,
                Color::srgb(225, 210, 205),
            ),
        ))
        .spacing(4.0),
        hstack((
            product_tile(
                state,
                "Apple Slices",
                "210 cents",
                210,
                Color::srgb(255, 224, 224),
            ),
            product_tile(
                state,
                "Berry Cup",
                "295 cents",
                295,
                Color::srgb(238, 218, 255),
            ),
            product_tile(
                state,
                "Greek Yogurt",
                "240 cents",
                240,
                Color::srgb(232, 242, 255),
            ),
            product_tile(
                state,
                "Protein Shake",
                "350 cents",
                350,
                Color::srgb(222, 232, 242),
            ),
        ))
        .spacing(4.0),
    ))
    .spacing(4.0);

    let payment = Frame::new(zstack((
        Color::srgb(235, 241, 247),
        hstack((
            vstack((
                text!("Selected: {selected_product}").bold(),
                text!("Authorization: {progress_percent}%"),
            )),
            spacer(),
            Frame::new(progress(progress_value)).width(150),
        ))
        .padding(),
    )))
    .height(64);

    zstack((
        Color::srgb(248, 250, 252),
        vstack((header, products, payment))
            .spacing(4.0)
            .padding_with(8.0),
    ))
}

/// Frames sampled after warm-up, overridable for longer soak runs.
fn sample_frames() -> usize {
    env_usize("DEW_PERF_FRAMES", DEFAULT_SAMPLE_FRAMES)
}

fn warmup_frames() -> usize {
    env_usize("DEW_PERF_WARMUP", WARMUP_FRAMES)
}

fn band_height() -> u32 {
    u32::try_from(env_usize(
        "DEW_PERF_BAND_HEIGHT",
        DEFAULT_BAND_HEIGHT as usize,
    ))
    .expect("band height must fit u32")
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| {
        value
            .parse()
            .unwrap_or_else(|error| panic!("{name} must be a non-negative integer: {error}"))
    })
}

/// Everything one sampled run establishes about the workload.
struct Sample {
    work: FrameWork,
    peak_heap_bytes: usize,
    heap_growth_bytes: i64,
    heap_allocations: u64,
    heap_allocated_bytes: u64,
    /// Bytes of the stand-in font blob living in host heap (flash on target).
    host_font_bytes: usize,
    peak_rgba_flush_bytes: usize,
    peak_rgb565_bytes: usize,
    host_cpu_p50: Duration,
    host_cpu_p99: Duration,
}

/// The simulation's verdict, serialized to TOML for review.
#[derive(serde::Serialize)]
struct Report {
    workload: Workload,
    target: Target,
    per_frame_work: PerFrameWork,
    memory: Memory,
    panel: Panel,
    /// Host wall-clock, recorded for context and explicitly not a criterion.
    host_diagnostics: HostDiagnostics,
    budget: WorkBudget,
    memory_budget: MemoryBudget,
    violations: Vec<Violation>,
}

#[derive(serde::Serialize)]
struct Workload {
    name: &'static str,
    width: u32,
    height: u32,
    band_height: u32,
    sample_frames: usize,
    interaction: &'static str,
}

#[derive(serde::Serialize)]
struct Target {
    chip: ChipBudget,
    /// Rasterizer pipeline, which must match what the chip actually runs.
    render_mode: &'static str,
    projected_cpu_ms_per_frame: f64,
    projected_frame_ms: f64,
    /// True only when the chip budget was calibrated on real hardware.
    projection_is_measured: bool,
}

#[derive(serde::Serialize)]
struct PerFrameWork {
    text_layouts_shaped: f64,
    text_layouts_reused: f64,
    glyph_bounds_measured: f64,
    glyph_runs_reused: f64,
    measures_computed: f64,
    measures_reused: f64,
    commands_emitted: f64,
    path_elements_flattened: f64,
    command_band_visits: f64,
    command_band_draws: f64,
    pixels_rasterized: f64,
    pixels_transferred: f64,
    regions_transferred: f64,
    measure_cache_hit_rate: Option<f64>,
    text_cache_hit_rate: Option<f64>,
    band_visit_efficiency: Option<f64>,
}

#[derive(serde::Serialize)]
struct Memory {
    /// Zero by construction: Dew never allocates a full framebuffer.
    panel_framebuffer_bytes: u64,
    scratch_rgba_band_bytes: usize,
    peak_rgba_flush_bytes: usize,
    peak_rgb565_dma_bytes: usize,
    peak_heap_bytes: usize,
    /// Bytes of the stand-in font blob, heap on the host but flash on the
    /// target; subtracted from the peak before gating.
    host_font_bytes: usize,
    /// The gated number: steady-state peak live heap minus the font blob.
    steady_peak_heap_bytes: u64,
    /// Live-heap delta across the sample. Non-zero means per-frame retention,
    /// which is fatal on a device with a few hundred KiB of RAM.
    heap_growth_bytes: i64,
    /// Allocation events per frame — the fragmentation-pressure proxy; the
    /// count and order transfer to the target exactly.
    heap_allocations_per_frame: f64,
    /// Bytes requested from the allocator per frame.
    heap_allocated_bytes_per_frame: f64,
}

#[derive(serde::Serialize)]
struct Panel {
    bus: PanelBus,
    transfer_ms_per_frame: f64,
    /// What a full-screen repaint would cost on this bus — the ceiling Dew's
    /// dirty-region design exists to stay below.
    full_frame_ms: f64,
    full_frame_fps: f64,
}

#[derive(serde::Serialize)]
struct HostDiagnostics {
    note: &'static str,
    cpu_p50_ms: f64,
    cpu_p99_ms: f64,
}

fn duration_millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[expect(
    clippy::cast_precision_loss,
    reason = "counter and frame magnitudes stay far below f64's exact-integer range"
)]
fn per_frame(total: u64, frames: usize) -> f64 {
    total as f64 / frames as f64
}

#[expect(
    clippy::cast_precision_loss,
    reason = "counter and frame magnitudes stay far below f64's exact-integer range"
)]
fn build_report(sample: &Sample, frames: usize, band_height: u32) -> Report {
    let work = &sample.work;
    let bus = PanelBus::SPI_40MHZ_RGB565;
    let transfer = bus.transfer_time(work);
    let full_frame = bus.full_frame_time(WIDTH, HEIGHT);
    let chip = target_chip();
    let projected_cpu = chip.cpu_nanoseconds(work) / frames as u64;
    let projected_frame =
        Duration::from_nanos(projected_cpu) + transfer / u32::try_from(frames).expect("frames");
    let budget = work_budget();
    Report {
        workload: Workload {
            name: "vending-machine-480x320-12-products",
            width: WIDTH,
            height: HEIGHT,
            band_height,
            sample_frames: frames,
            interaction: "retained product button every 120 frames",
        },
        target: Target {
            chip,
            render_mode: TARGET_RENDER_MODE_NAME,
            projected_cpu_ms_per_frame: projected_cpu as f64 / 1_000_000.0,
            projected_frame_ms: duration_millis(projected_frame),
            projection_is_measured: matches!(chip.provenance, Provenance::Measured),
        },
        per_frame_work: PerFrameWork {
            text_layouts_shaped: per_frame(work.text_layouts_shaped, frames),
            text_layouts_reused: per_frame(work.text_layouts_reused, frames),
            glyph_bounds_measured: per_frame(work.glyph_bounds_measured, frames),
            glyph_runs_reused: per_frame(work.glyph_runs_reused, frames),
            measures_computed: per_frame(work.measures_computed, frames),
            measures_reused: per_frame(work.measures_reused, frames),
            commands_emitted: per_frame(work.commands_emitted, frames),
            path_elements_flattened: per_frame(work.path_elements_flattened, frames),
            command_band_visits: per_frame(work.command_band_visits, frames),
            command_band_draws: per_frame(work.command_band_draws, frames),
            pixels_rasterized: per_frame(work.pixels_rasterized, frames),
            pixels_transferred: per_frame(work.pixels_transferred, frames),
            regions_transferred: per_frame(work.regions_transferred, frames),
            measure_cache_hit_rate: work.measure_hit_rate(),
            text_cache_hit_rate: work.text_hit_rate(),
            band_visit_efficiency: work.band_visit_efficiency(),
        },
        memory: Memory {
            panel_framebuffer_bytes: 0,
            scratch_rgba_band_bytes: WIDTH as usize * band_height as usize * 4,
            peak_rgba_flush_bytes: sample.peak_rgba_flush_bytes,
            peak_rgb565_dma_bytes: sample.peak_rgb565_bytes,
            peak_heap_bytes: sample.peak_heap_bytes,
            host_font_bytes: sample.host_font_bytes,
            steady_peak_heap_bytes: steady_peak_heap_bytes(sample),
            heap_growth_bytes: sample.heap_growth_bytes,
            heap_allocations_per_frame: per_frame(sample.heap_allocations, frames),
            heap_allocated_bytes_per_frame: per_frame(sample.heap_allocated_bytes, frames),
        },
        panel: Panel {
            bus,
            transfer_ms_per_frame: duration_millis(transfer) / frames as f64,
            full_frame_ms: duration_millis(full_frame),
            full_frame_fps: 1.0 / full_frame.as_secs_f64(),
        },
        host_diagnostics: HostDiagnostics {
            note: "host wall-clock; recorded for context, never a pass criterion \
                   (see waterui_dew::stats)",
            cpu_p50_ms: duration_millis(sample.host_cpu_p50),
            cpu_p99_ms: duration_millis(sample.host_cpu_p99),
        },
        budget,
        memory_budget: memory_budget(),
        violations: {
            let mut violations = budget.violations(work, frames);
            violations.extend(memory_budget().violations(
                steady_peak_heap_bytes(sample),
                sample.heap_allocations,
                sample.heap_allocated_bytes,
                frames,
            ));
            violations
        },
    }
}

/// The gated peak: steady-state peak live heap minus the host-only font blob.
fn steady_peak_heap_bytes(sample: &Sample) -> u64 {
    u64::try_from(
        sample
            .peak_heap_bytes
            .saturating_sub(sample.host_font_bytes),
    )
    .expect("peak heap must fit u64")
}

fn create_runtime(state: &VendingState, band_height: u32) -> DewRuntime<SimulatedEmbeddedBoard> {
    let root_state = state.clone();
    DewRuntime::new(
        SimulatedEmbeddedBoard::new(),
        support::test_environment(),
        band_height,
        move || AnyView::new(vending_screen(&root_state)),
    )
}

fn enqueue_product_selection(runtime: &mut DewRuntime<SimulatedEmbeddedBoard>, frame: usize) {
    if !frame.is_multiple_of(INTERACTION_PERIOD) {
        return;
    }
    let selection = (frame / INTERACTION_PERIOD) % 12;
    let column = selection % 4;
    let row = selection / 4;
    let x = 108.0f64.mul_add(f64::from(u32::try_from(column).expect("column")), 78.0);
    let y = 62.0f64.mul_add(f64::from(u32::try_from(row).expect("row")), 77.0);
    runtime.board_mut().click(x, y);
}

fn drive(
    runtime: &mut DewRuntime<SimulatedEmbeddedBoard>,
    state: &VendingState,
    start_frame: usize,
    frames: usize,
    collect: Option<&mut Sample>,
) {
    let mut host_times = Vec::with_capacity(frames);
    let mut work = FrameWork::ZERO;
    for offset in 0..frames {
        let frame = offset + start_frame;
        state.update_animation(frame);
        enqueue_product_selection(runtime, frame);
        let started = StdInstant::now();
        let rendered = runtime
            .pump()
            .expect("an animated frame must always re-render");
        host_times.push(started.elapsed());
        work += rendered.work;
    }
    let Some(sample) = collect else {
        return;
    };
    host_times.sort_unstable();
    sample.work = work;
    sample.host_cpu_p50 = percentile(&host_times, 50);
    sample.host_cpu_p99 = percentile(&host_times, 99);
    sample.peak_rgba_flush_bytes = runtime.board().display.peak_rgba_bytes();
    sample.peak_rgb565_bytes = runtime.board().display.peak_dma_bytes();
}

const fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    sorted[(sorted.len() - 1) * percent / 100]
}

#[test]
fn vending_product_selection_updates_order() {
    let state = VendingState::new();
    let mut runtime = create_runtime(&state, DEFAULT_BAND_HEIGHT);
    runtime
        .pump()
        .expect("initial vending-machine frame must render");

    enqueue_product_selection(&mut runtime, INTERACTION_PERIOD);
    runtime
        .pump()
        .expect("product selection must render the retained update");

    assert_eq!(state.selected_product.get(), "Sparkling Lime");
    assert_eq!(state.total_cents.get(), 175);
}

/// The panel bus alone caps full-screen repaints well below 60 FPS, which is
/// the entire reason Dew tracks dirty regions. If this ever stops holding,
/// the workload is no longer representative of a bandwidth-bound panel.
#[test]
fn full_screen_repaint_is_bus_bound_below_thirty_fps() {
    let full_frame = PanelBus::SPI_40MHZ_RGB565.full_frame_time(WIDTH, HEIGHT);
    assert!(
        full_frame > Duration::from_millis(33),
        "a {WIDTH}x{HEIGHT} RGB565 frame over 40 MHz SPI must exceed a 30 FPS budget, got {full_frame:?}"
    );
}

/// Debug stack frames are several times their optimized size, so the
/// firmware-stack gate scales the target's task stack by this factor in
/// unoptimized builds.
///
/// The factor is a modeling estimate in the same spirit as
/// [`ChipBudget`]'s cycle costs, and it is calibrated against evidence in
/// both directions: real `opt-level=2` firmware renders a full facade app in
/// a 48 KiB task on an emulated ESP32-C3, while this unoptimized host
/// pipeline overflows the S3's 160 KiB task stack and first fits between
/// 320 KiB and 384 KiB (the deepest path is the first frame's build → label
/// emit → parley shaping chain). 3× (480 KiB) passes with bounded margin, so
/// recursion-depth growth still trips the gate; optimized builds use the
/// chip's stack unscaled.
const DEBUG_STACK_FACTOR: u64 = 3;

/// The work-budgeted embedded simulation.
///
/// Fails on four things, none of which is host speed: exceeding a per-frame
/// work ceiling, exceeding a heap ceiling (steady-state peak or per-frame
/// churn), retaining heap across frames, or handing the board more than one
/// band of pixels at a time. The whole pipeline runs on a thread with the
/// target's main-task stack (debug-scaled, see [`DEBUG_STACK_FACTOR`]), so
/// recursion that would overflow the chip overflows here first.
#[test]
fn vending_machine_holds_its_embedded_work_budget() {
    let chip = target_chip();
    assert!(
        memory_budget().steady_peak_heap_bytes
            <= chip.usable_sram_bytes.midpoint(chip.usable_psram_bytes),
        "the heap budget must leave at least half of the {} + {} byte SRAM+PSRAM heap as \
         headroom for allocator overhead, fragmentation, and non-heap statics",
        chip.usable_sram_bytes,
        chip.usable_psram_bytes
    );
    let scale = if cfg!(debug_assertions) {
        DEBUG_STACK_FACTOR
    } else {
        1
    };
    let stack = usize::try_from(chip.main_task_stack_bytes * scale)
        .expect("task stack size must fit usize");
    let outcome = std::thread::Builder::new()
        .name("dew-main-task".into())
        .stack_size(stack)
        .spawn(run_budgeted_simulation)
        .expect("the firmware-stack simulation thread must spawn")
        .join();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

fn run_budgeted_simulation() {
    let frames = sample_frames();
    let warmup = warmup_frames();
    let band_height = band_height();
    assert!(frames > 0, "the sample must contain frames");
    assert!(band_height > 0, "the band height must be positive");

    let state = VendingState::new();
    let mut runtime = create_runtime(&state, band_height);
    let host_font_bytes = runtime.board().font.len();
    runtime
        .pump()
        .expect("initial vending-machine frame must render");
    drive(&mut runtime, &state, 0, warmup, None);

    // Warm-up has populated every cache and grown every buffer, so heap from
    // here on should be flat. Measure the steady state, not the ramp.
    ALLOCATOR.reset_peak();
    let heap_before = ALLOCATOR.live_bytes();
    let allocations_before = ALLOCATOR.allocation_count();
    let allocated_bytes_before = ALLOCATOR.allocated_bytes();
    let mut sample = Sample {
        work: FrameWork::ZERO,
        peak_heap_bytes: 0,
        heap_growth_bytes: 0,
        heap_allocations: 0,
        heap_allocated_bytes: 0,
        host_font_bytes,
        peak_rgba_flush_bytes: 0,
        peak_rgb565_bytes: 0,
        host_cpu_p50: Duration::ZERO,
        host_cpu_p99: Duration::ZERO,
    };
    drive(&mut runtime, &state, warmup, frames, Some(&mut sample));
    sample.peak_heap_bytes = ALLOCATOR.peak_bytes();
    sample.heap_growth_bytes = i64::try_from(ALLOCATOR.live_bytes()).expect("heap fits i64")
        - i64::try_from(heap_before).expect("heap fits i64");
    sample.heap_allocations =
        u64::try_from(ALLOCATOR.allocation_count() - allocations_before).expect("count fits u64");
    sample.heap_allocated_bytes =
        u64::try_from(ALLOCATOR.allocated_bytes() - allocated_bytes_before).expect("bytes fit u64");

    let report = build_report(&sample, frames, band_height);
    let rendered = toml::to_string_pretty(&report).expect("the report must serialize");
    std::fs::write("/tmp/waterui_dew_vending_performance.toml", &rendered)
        .expect("the performance report must be writable");

    let max_rgba_band = WIDTH as usize * band_height as usize * 4;
    assert!(
        sample.peak_rgba_flush_bytes <= max_rgba_band,
        "the board received a region larger than one RGBA band; {rendered}"
    );
    assert!(
        sample.peak_rgb565_bytes <= max_rgba_band / 2,
        "the board allocated more than one RGB565 DMA band; {rendered}"
    );
    let chip = target_chip();
    let retention_per_frame = sample.heap_growth_bytes.unsigned_abs() / frames as u64;
    assert!(
        retention_per_frame <= MAX_HEAP_RETENTION_BYTES_PER_FRAME,
        "steady-state frames retained {retention_per_frame} bytes of heap each ({} total over \
         {frames} frames). Sustained, that exhausts a {}-KiB device in {} frames — under half an \
         hour at 60 FPS. {rendered}",
        sample.heap_growth_bytes,
        chip.usable_sram_bytes / 1024,
        chip.usable_sram_bytes / retention_per_frame.max(1)
    );
    assert!(
        report.violations.is_empty(),
        "the vending-machine simulation exceeded its per-frame work or memory budget; {rendered}"
    );
}
