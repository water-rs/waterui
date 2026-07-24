//! Performance-constrained commercial UI simulation for Dew.
//!
//! The workload models a 480×320 vending-machine panel with twelve product
//! tiles, a live order summary, and a continuously updating payment-progress
//! bar. The renderer stays single-threaded and the test is intended to run in
//! Cargo's unoptimized test profile with the scalar fallback rasterizer. Display
//! transfer time is accounted for as an `RGB565` panel on a 40 MHz `SPI` bus.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::{Duration, Instant as StdInstant};

use nami::{Binding, binding};
use vello_cpu::{Level, RenderMode, RenderSettings};
use waterui::prelude::*;
use waterui_backend_core::input::TouchPhase;
use waterui_backend_core::time::Instant;
use waterui_core::AnyView;
use waterui_dew::{
    Board, DeviceRegion, DewRuntime, PointerSample, Rgb565Display, Rgb565Sink, render_view_png,
};
use waterui_layout::frame::Frame;

mod support;

use core::prelude::v1::test;

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const DEFAULT_BAND_HEIGHT: u32 = 16;
const FRAME_BUDGET: Duration = Duration::from_nanos(16_666_667);
const SPI_BITS_PER_SECOND: u64 = 40_000_000;
const RGB565_BITS_PER_PIXEL: u64 = 16;
const DMA_SETUP: Duration = Duration::from_micros(8);
const WARMUP_FRAMES: usize = 120;
const DEFAULT_SAMPLE_FRAMES: usize = 3_600;
const INTERACTION_PERIOD: usize = 120;

#[derive(Debug)]
struct StreamingPanel {
    width: u32,
    height: u32,
    frame_pixels: u64,
    frame_regions: u64,
    last_transfer: Option<TransferWork>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TransferWork {
    pixels: u64,
    regions: u64,
}

impl StreamingPanel {
    const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            frame_pixels: 0,
            frame_regions: 0,
            last_transfer: None,
        }
    }

    const fn last_transfer(&self) -> TransferWork {
        self.last_transfer
            .expect("a pumped Dew frame must present its transfer work")
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
        std::hint::black_box(pixels);
        self.frame_pixels += region.area();
        self.frame_regions += 1;
    }

    fn present(&mut self) {
        self.last_transfer = Some(TransferWork {
            pixels: self.frame_pixels,
            regions: self.frame_regions,
        });
        self.frame_pixels = 0;
        self.frame_regions = 0;
    }
}

#[derive(Debug)]
struct SimulatedEmbeddedBoard {
    display: Rgb565Display<StreamingPanel>,
    pointer_samples: VecDeque<PointerSample>,
}

impl SimulatedEmbeddedBoard {
    const fn new() -> Self {
        Self {
            display: Rgb565Display::new(StreamingPanel::new(WIDTH, HEIGHT)),
            pointer_samples: VecDeque::new(),
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
            level: Level::fallback(),
            num_threads: 0,
            render_mode: RenderMode::OptimizeSpeed,
        }
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

fn panel_transfer_time(work: TransferWork) -> Duration {
    let wire_bits = work.pixels * RGB565_BITS_PER_PIXEL;
    let wire_nanoseconds = wire_bits
        .checked_mul(1_000_000_000)
        .expect("panel transfer duration overflow")
        / SPI_BITS_PER_SECOND;
    Duration::from_nanos(wire_nanoseconds)
        + DMA_SETUP * u32::try_from(work.regions).expect("region count must fit u32")
}

const fn percentile(samples: &[Duration], numerator: usize, denominator: usize) -> Duration {
    let index = (samples.len() - 1) * numerator / denominator;
    samples[index]
}

fn sample_frames() -> usize {
    std::env::var("DEW_PERF_FRAMES").map_or(DEFAULT_SAMPLE_FRAMES, |value| {
        value
            .parse()
            .expect("DEW_PERF_FRAMES must be a positive integer")
    })
}

fn warmup_frames() -> usize {
    std::env::var("DEW_PERF_WARMUP").map_or(WARMUP_FRAMES, |value| {
        value
            .parse()
            .expect("DEW_PERF_WARMUP must be a non-negative integer")
    })
}

fn band_height() -> u32 {
    std::env::var("DEW_PERF_BAND_HEIGHT").map_or(DEFAULT_BAND_HEIGHT, |value| {
        value
            .parse()
            .expect("DEW_PERF_BAND_HEIGHT must be a positive integer")
    })
}

struct PerformanceSample {
    cpu_times: Vec<Duration>,
    effective_times: Vec<Duration>,
    transferred_pixels: u64,
    transferred_regions: u64,
    peak_rgba_flush_bytes: usize,
    peak_rgb565_bytes: usize,
    missed_frames: Vec<MissedFrame>,
}

struct MissedFrame {
    frame: usize,
    cpu_time: Duration,
    transfer_time: Duration,
    transfer: TransferWork,
    dirty: Vec<kurbo::Rect>,
}

impl PerformanceSample {
    const fn missed_frames(&self) -> usize {
        self.missed_frames.len()
    }
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

fn warm_up(runtime: &mut DewRuntime<SimulatedEmbeddedBoard>, state: &VendingState, frames: usize) {
    for frame in 0..frames {
        state.update_animation(frame);
        enqueue_product_selection(runtime, frame);
        runtime.pump().expect("warmup state change must render");
    }
}

fn enqueue_product_selection(runtime: &mut DewRuntime<SimulatedEmbeddedBoard>, frame: usize) {
    if !frame.is_multiple_of(INTERACTION_PERIOD) {
        return;
    }
    let selection = (frame / INTERACTION_PERIOD) % 12;
    let column = selection % 4;
    let row = selection / 4;
    let x = 108.0f64.mul_add(
        f64::from(u32::try_from(column).expect("column must fit u32")),
        78.0,
    );
    let y = 62.0f64.mul_add(
        f64::from(u32::try_from(row).expect("row must fit u32")),
        77.0,
    );
    runtime.board_mut().click(x, y);
}

fn collect_sample(
    runtime: &mut DewRuntime<SimulatedEmbeddedBoard>,
    state: &VendingState,
    start_frame: usize,
    frames: usize,
) -> PerformanceSample {
    let mut sample = PerformanceSample {
        cpu_times: Vec::with_capacity(frames),
        effective_times: Vec::with_capacity(frames),
        transferred_pixels: 0,
        transferred_regions: 0,
        peak_rgba_flush_bytes: 0,
        peak_rgb565_bytes: 0,
        missed_frames: Vec::new(),
    };
    for frame in 0..frames {
        let animation_frame = frame + start_frame;
        state.update_animation(animation_frame);
        enqueue_product_selection(runtime, animation_frame);
        let started = StdInstant::now();
        let dirty = runtime
            .pump()
            .expect("commercial animation state change must render");
        let cpu_time = started.elapsed();
        let transfer = runtime.board().display.sink().last_transfer();
        let transfer_time = panel_transfer_time(transfer);
        let effective_time = cpu_time + transfer_time;
        if effective_time > FRAME_BUDGET {
            sample.missed_frames.push(MissedFrame {
                frame: animation_frame,
                cpu_time,
                transfer_time,
                transfer,
                dirty,
            });
        }
        sample.cpu_times.push(cpu_time);
        sample.effective_times.push(effective_time);
        sample.transferred_pixels += transfer.pixels;
        sample.transferred_regions += transfer.regions;
    }
    sample.peak_rgba_flush_bytes = runtime.board().display.peak_rgba_bytes();
    sample.peak_rgb565_bytes = runtime.board().display.peak_dma_bytes();
    sample.cpu_times.sort_unstable();
    sample.effective_times.sort_unstable();
    sample
}

fn duration_millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn format_report(sample: &PerformanceSample, frames: usize, band_height: u32) -> String {
    let frame_count = u32::try_from(frames).expect("sample frame count must fit u32");
    let scratch_rgba_bytes = usize::try_from(WIDTH).expect("panel width must fit usize")
        * usize::try_from(band_height).expect("band height must fit usize")
        * 4;
    let pixel_count =
        u32::try_from(sample.transferred_pixels).expect("sample pixel count must fit u32");
    let region_count =
        u32::try_from(sample.transferred_regions).expect("sample region count must fit u32");
    let elapsed: Duration = sample.effective_times.iter().sum();
    let paced_elapsed: Duration = sample
        .effective_times
        .iter()
        .map(|duration| (*duration).max(FRAME_BUDGET))
        .sum();
    let work_capacity_fps = f64::from(frame_count) / elapsed.as_secs_f64();
    let sustained_fps = f64::from(frame_count) / paced_elapsed.as_secs_f64();
    let mut report = String::new();
    writeln!(report, "workload=vending-machine-480x320-12-products").unwrap();
    writeln!(
        report,
        "interaction=retained-product-button-every-120-frames"
    )
    .unwrap();
    writeln!(report, "cpu_profile=scalar-fallback-single-thread").unwrap();
    writeln!(report, "sample_frames={frames}").unwrap();
    writeln!(report, "band_height={band_height}").unwrap();
    write_memory_report(&mut report, sample, scratch_rgba_bytes);
    writeln!(
        report,
        "frame_budget_ms={:.3}",
        duration_millis(FRAME_BUDGET)
    )
    .unwrap();
    writeln!(report, "target_fps=60").unwrap();
    writeln!(report, "missed_frames={}", sample.missed_frames()).unwrap();
    writeln!(report, "sustained_fps={sustained_fps:.2}").unwrap();
    writeln!(report, "work_capacity_fps={work_capacity_fps:.2}").unwrap();
    writeln!(
        report,
        "cpu_p50_ms={:.3}",
        duration_millis(percentile(&sample.cpu_times, 50, 100))
    )
    .unwrap();
    writeln!(
        report,
        "cpu_p99_ms={:.3}",
        duration_millis(percentile(&sample.cpu_times, 99, 100))
    )
    .unwrap();
    writeln!(
        report,
        "effective_p50_ms={:.3}",
        duration_millis(percentile(&sample.effective_times, 50, 100))
    )
    .unwrap();
    writeln!(
        report,
        "effective_p95_ms={:.3}",
        duration_millis(percentile(&sample.effective_times, 95, 100))
    )
    .unwrap();
    writeln!(
        report,
        "effective_p99_ms={:.3}",
        duration_millis(percentile(&sample.effective_times, 99, 100))
    )
    .unwrap();
    writeln!(
        report,
        "effective_max_ms={:.3}",
        duration_millis(
            *sample
                .effective_times
                .last()
                .expect("performance sample must not be empty")
        )
    )
    .unwrap();
    writeln!(
        report,
        "average_transfer_pixels={:.1}",
        f64::from(pixel_count) / f64::from(frame_count)
    )
    .unwrap();
    writeln!(
        report,
        "average_transfer_regions={:.2}",
        f64::from(region_count) / f64::from(frame_count)
    )
    .unwrap();
    write_missed_frames(&mut report, sample);
    report
}

fn write_memory_report(report: &mut String, sample: &PerformanceSample, scratch_rgba_bytes: usize) {
    writeln!(report, "panel_framebuffer_bytes=0").unwrap();
    writeln!(report, "scratch_rgba_bytes={scratch_rgba_bytes}").unwrap();
    writeln!(
        report,
        "peak_rgba_flush_bytes={}",
        sample.peak_rgba_flush_bytes
    )
    .unwrap();
    writeln!(report, "peak_rgb565_dma_bytes={}", sample.peak_rgb565_bytes).unwrap();
    writeln!(
        report,
        "peak_pixel_working_set_bytes={}",
        scratch_rgba_bytes + sample.peak_rgb565_bytes
    )
    .unwrap();
}

fn write_missed_frames(report: &mut String, sample: &PerformanceSample) {
    for missed in &sample.missed_frames {
        writeln!(
            report,
            "missed_frame={} cpu_ms={:.3} transfer_ms={:.3} pixels={} regions={}",
            missed.frame,
            duration_millis(missed.cpu_time),
            duration_millis(missed.transfer_time),
            missed.transfer.pixels,
            missed.transfer.regions
        )
        .unwrap();
        writeln!(report, "missed_dirty={:?}", missed.dirty).unwrap();
    }
}

fn export_visual(state: &VendingState) {
    let visual_state = state.clone();
    let png = render_view_png(
        move || vending_screen(&visual_state),
        support::test_environment(),
        WIDTH,
        HEIGHT,
    );
    std::fs::write("/tmp/waterui_dew_vending_machine.png", png)
        .expect("vending-machine visual artifact must be writable");
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

#[test]
#[ignore = "explicit scalar single-thread 60 FPS performance simulation"]
fn vending_machine_holds_stable_sixty_fps() {
    let sample_frames = sample_frames();
    let warmup_frames = warmup_frames();
    let band_height = band_height();
    assert!(sample_frames > 0, "performance sample must contain frames");
    assert!(band_height > 0, "performance band height must be positive");
    let state = VendingState::new();
    if std::env::var_os("DEW_PERF_SKIP_IMAGE").is_none() {
        export_visual(&state);
    }
    let mut runtime = create_runtime(&state, band_height);
    runtime
        .pump()
        .expect("initial vending-machine frame must render");

    warm_up(&mut runtime, &state, warmup_frames);
    let sample = collect_sample(&mut runtime, &state, warmup_frames, sample_frames);
    let maximum_rgba_band_bytes = usize::try_from(WIDTH).expect("panel width must fit usize")
        * usize::try_from(band_height).expect("band height must fit usize")
        * 4;
    let maximum_rgb565_band_bytes = maximum_rgba_band_bytes / 2;
    assert!(
        sample.peak_rgba_flush_bytes <= maximum_rgba_band_bytes,
        "simulated board received a region larger than one RGBA band"
    );
    assert!(
        sample.peak_rgb565_bytes <= maximum_rgb565_band_bytes,
        "simulated board allocated more than one RGB565 DMA band"
    );
    let report = format_report(&sample, sample_frames, band_height);
    std::fs::write("/tmp/waterui_dew_vending_performance.txt", &report)
        .expect("performance report must be writable");

    assert_eq!(
        sample.missed_frames(),
        0,
        "vending-machine simulation missed frames; {report}"
    );
}
