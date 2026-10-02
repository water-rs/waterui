//! Headless frame-pipeline profiler for Hydrolysis.
//!
//! Renders representative WaterUI scenes through the real retained-tree →
//! layout → `Recording` → compositor pipeline offscreen — no window, no
//! display — and writes one JSON document per scene with per-frame stage
//! records plus p50/p90/p99 per stage. GPU stages come from wgpu timestamp
//! queries and stay `null` where the adapter lacks `TIMESTAMP_QUERY`.
//!
//! The point of the numbers is the stage split: `encode` is the CPU time
//! Hydrolysis's own pipeline spends recording the frame, while `content_gpu`
//! and `compositor_gpu` bracket the engine's layer-content submits and the
//! presentation pass.
//!
//! ```sh
//! cargo run --example frame_profile --features frame-profile --release -- \
//!     --out-dir frame-profile-out --warmup 20 --frames 120
//! ```

use core::time::Duration;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use hydrolysis::{HeadlessRuntime, OffscreenGpuContext, TARGET_FRAME_INTERVAL};
use hydrolysis_m3::{
    Material3, OutlinedSegmentedButton, OutlinedSegmentedButtonSet, assist_chip, extended_fab, fab,
    filter_chip, icon_button, input_chip, material_badge, material_card, material_divider,
    material_list_item, suggestion_chip,
};
use serde::Serialize;
use waterui::Binding;
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::component::text;
use waterui_chart::{BarChart, DataPoint};
use waterui_core::SignalExt as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_core::layout::Point;
use waterui_core::{AnyView, Environment};
use waterui_graphics::color::Srgb;
use waterui_icons_lucide as lucide;
use waterui_layout::scroll::{ScrollController, scroll};
use waterui_layout::stack::{VStack, hstack, vstack};

/// Phone-class logical viewport, matching the Pixel target the Android build
/// runs on.
const WINDOW_WIDTH: u32 = 390;
const WINDOW_HEIGHT: u32 = 844;
const SCALE_FACTOR: f64 = 2.625;
/// Frame instants step at the 120 Hz target interval, so animation sampling
/// sees the timeline a high-refresh display drives.
const FRAME_STEP: Duration = TARGET_FRAME_INTERVAL;

fn ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).expect("frame stage exceeds u64 nanoseconds")
}

/// Nearest-rank percentile summary over one stage's per-frame nanosecond
/// samples.
#[derive(Serialize)]
struct StageSummary {
    samples: usize,
    mean_ns: f64,
    p50_ns: u64,
    p90_ns: u64,
    p99_ns: u64,
    max_ns: u64,
}

fn summarize(mut samples: Vec<u64>) -> StageSummary {
    assert!(!samples.is_empty(), "a measured stage always has samples");
    samples.sort_unstable();
    let n = samples.len();
    let percentile = |p: usize| samples[(p * n).div_ceil(100).max(1) - 1];
    StageSummary {
        samples: n,
        mean_ns: samples.iter().sum::<u64>() as f64 / n as f64,
        p50_ns: percentile(50),
        p90_ns: percentile(90),
        p99_ns: percentile(99),
        max_ns: samples[n - 1],
    }
}

/// One pumped frame: the CPU stage split, the resolved GPU spans (`null` when
/// the adapter cannot timestamp), and the pipeline's own phase/counter detail.
#[derive(Serialize)]
struct FrameRecord {
    frame: u32,
    rebuilt: bool,
    update_ns: u64,
    layout_ns: u64,
    encode_ns: u64,
    content_gpu_ns: Option<u64>,
    compositor_gpu_ns: Option<u64>,
    gpu_wait_ns: u64,
    readback_ns: u64,
    total_ns: u64,
    /// Digest of the frame's placed-bounds tree — identical across runs iff
    /// layout produced identical geometry for every node.
    layout_signature: Option<u64>,
    phases: BTreeMap<&'static str, u64>,
    counters: BTreeMap<&'static str, u64>,
}

#[derive(Serialize)]
struct GpuRecord {
    adapter: String,
    device_type: String,
    backend: String,
    driver: String,
    driver_info: String,
    vendor: u32,
    device: u32,
    adapter_features: String,
    device_features: String,
    timestamp_queries: bool,
}

#[derive(Serialize)]
struct SceneReport {
    scene: &'static str,
    window_width: u32,
    window_height: u32,
    scale_factor: f64,
    warmup_frames: u32,
    gpu: GpuRecord,
    /// Per-stage percentiles; a stage present only when the adapter supports
    /// it serializes as `null`.
    stages: BTreeMap<&'static str, Option<StageSummary>>,
    frames: Vec<FrameRecord>,
}

/// A scene's per-frame driver: every measured frame must carry real reactive
/// change (a scroll step, a state toggle, a data shift) or the pump would
/// stay idle and produce no frame at all — the profiler never pumps
/// `thread::sleep`, it pumps frames.
type FrameDriver = Box<dyn FnMut(u32)>;

struct SceneSpec {
    name: &'static str,
    build: fn(&OffscreenGpuContext) -> (HeadlessRuntime, FrameDriver),
}

fn runtime_on(gpu: &OffscreenGpuContext, view: impl Fn() -> AnyView + 'static) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests_on_context(
        gpu.clone(),
        Environment::new(),
        AnyViewBuilder::<AnyView>::new(view),
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        Material3::defaults(),
    )
    .with_scale_factor(SCALE_FACTOR)
}

/// A 200-row list of icon + text rows scrolled by a scripted offset each
/// frame — the scrolling-list workload every feed UI is.
fn scrolling_list(gpu: &OffscreenGpuContext) -> (HeadlessRuntime, FrameDriver) {
    const ROWS: usize = 200;
    const ROW_HEIGHT: f32 = 52.0;
    let controller = ScrollController::new(Point::zero());
    let runtime = {
        let controller = controller.clone();
        runtime_on(gpu, move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            AnyView::new(
                scroll(VStack::for_each(rows, |row| {
                    let index = row.into_inner();
                    hstack((
                        lucide::file_text(),
                        text(format!("Row {index} — agenda item")),
                        text(format!("{index:02}:00")),
                    ))
                    .size(360.0, ROW_HEIGHT)
                }))
                .scroll_controller(&controller),
            )
        })
    };
    let max_scroll = ROWS as f32 * ROW_HEIGHT - WINDOW_HEIGHT as f32;
    let driver = move |frame: u32| {
        // Sawtooth sweep through the full row range.
        controller.scroll_to(Point::new(0.0, (frame as f32 * 47.0) % max_scroll));
    };
    (runtime, Box::new(driver))
}

/// An md3 component gallery: chips, list items, badges, icon buttons,
/// segmented buttons and FABs, driven by toggled selection bindings.
fn md3_gallery(gpu: &OffscreenGpuContext) -> (HeadlessRuntime, FrameDriver) {
    let selected = Binding::container(false);
    let segment = Binding::container(false);
    let badge_count = Binding::container(3i32);
    let runtime = {
        let selected = selected.clone();
        let segment = segment.clone();
        let badge_count = badge_count.clone();
        runtime_on(gpu, move || {
            AnyView::new(scroll(vstack((
                text("Material 3 gallery"),
                material_card(vstack((
                    text("Card title"),
                    text("Supporting text for the card body."),
                ))),
                material_list_item("Inbox"),
                material_list_item("Sent items"),
                material_list_item("Drafts"),
                hstack((
                    assist_chip("Assist"),
                    filter_chip("Filter", &selected),
                    input_chip("Input"),
                    suggestion_chip("Suggest"),
                )),
                hstack((
                    icon_button("Home", lucide::house()),
                    icon_button("Settings", lucide::settings()),
                    icon_button("Profile", lucide::user()),
                    icon_button("Star", lucide::star()),
                )),
                hstack((
                    material_badge(badge_count.clone(), text("Mail")),
                    material_badge(24i32, text("Alerts")),
                )),
                OutlinedSegmentedButtonSet::new((
                    OutlinedSegmentedButton::new("Day", &segment),
                    OutlinedSegmentedButton::new("Week", &Binding::container(false)),
                    OutlinedSegmentedButton::new("Month", &Binding::container(true)),
                )),
                material_divider(),
                hstack((fab("Add item", lucide::heart()), extended_fab("Compose"))),
            ))))
        })
    };
    let driver = move |frame: u32| {
        if frame.is_multiple_of(30) {
            selected.set(frame.is_multiple_of(60));
            segment.set(!frame.is_multiple_of(60));
        }
        badge_count.set((frame % 99) as i32);
    };
    (runtime, Box::new(driver))
}

/// Animated opacity and transform: bindings eased through spring/ease
/// animations so every frame re-encodes moving geometry.
fn animated(gpu: &OffscreenGpuContext) -> (HeadlessRuntime, FrameDriver) {
    let opacity = Binding::container(1.0f32);
    let scale = Binding::container(1.0f32);
    let rotation = Binding::container(0.0f32);
    let offset_x = Binding::container(0.0f32);
    let runtime = {
        let opacity = opacity.clone();
        let scale = scale.clone();
        let rotation = rotation.clone();
        let offset_x = offset_x.clone();
        runtime_on(gpu, move || {
            let animated_opacity = opacity.with(Animation::ease_in_out(Duration::from_millis(400)));
            let animated_scale = scale.with(Animation::spring(250.0, 18.0));
            let animated_rotation =
                rotation.with(Animation::ease_in_out(Duration::from_millis(400)));
            let animated_x = offset_x.with(Animation::spring(200.0, 20.0));
            AnyView::new(vstack((
                text("Animated page"),
                material_card(vstack((
                    text("This card fades, scales and drifts."),
                    hstack((lucide::star(), text("Moving content"))),
                )))
                .opacity(animated_opacity)
                .scale(animated_scale.clone(), animated_scale)
                .rotation(animated_rotation)
                .offset(animated_x, 0.0f32),
                material_list_item("Static row below the motion"),
                material_list_item("Second static row"),
            )))
        })
    };
    let driver = move |frame: u32| {
        // The animation targets oscillate continuously, so every pumped frame
        // carries reactive change and every animation is mid-flight.
        let t = frame as f32;
        opacity.set(0.65 + 0.35 * (t * 0.11).sin());
        scale.set(0.92 + 0.08 * (t * 0.07).cos());
        rotation.set(4.0 * (t * 0.09).sin());
        offset_x.set(12.0 * (t * 0.05).sin());
    };
    (runtime, Box::new(driver))
}

/// A text-heavy page mixing Latin, CJK, Arabic, Devanagari and emoji, scrolled
/// by a scripted offset each frame.
fn text_heavy(gpu: &OffscreenGpuContext) -> (HeadlessRuntime, FrameDriver) {
    let controller = ScrollController::new(Point::zero());
    let paragraphs = [
        "The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs.",
        "，、。",
        "يقوم المحرك بإعادة ترميز الشجرة المحتفظ بها بالكامل في كل إطار ثم يرسمها.",
        "इंजन हर फ्रेम में पूरे रखे गए पेड़ को फिर से एन्कोड करता है और फिर उसे खींचता है।",
        "Mixed line: ASCII テキスト نص देवनागरी  emoji 😀🚀🎉 end.",
    ];
    let runtime = {
        let controller = controller.clone();
        runtime_on(gpu, move || {
            let blocks = (0..48u64).map(SelfId::new).collect::<Vec<_>>();
            AnyView::new(
                scroll(VStack::for_each(blocks, move |block| {
                    let index = block.into_inner();
                    let body = paragraphs[(index as usize) % paragraphs.len()];
                    vstack((text(format!("§{index}")), text(body)))
                }))
                .scroll_controller(&controller),
            )
        })
    };
    let driver = move |frame: u32| {
        controller.scroll_to(Point::new(0.0, (frame as f32 * 31.0) % 12_000.0));
    };
    (runtime, Box::new(driver))
}

/// A live-updating bar chart through waterui-chart — canvas → scene view →
/// engine layer — with the dataset shifted every frame.
fn chart(gpu: &OffscreenGpuContext) -> (HeadlessRuntime, FrameDriver) {
    const BARS: usize = 24;
    let data = Binding::container(
        (0..BARS)
            .map(|x| DataPoint::new(x as f32, 40.0 + 30.0 * (x as f32).sin()))
            .collect::<Vec<_>>(),
    );
    let runtime = {
        let data = data.clone();
        runtime_on(gpu, move || {
            AnyView::new(vstack((
                text("Throughput"),
                BarChart::new(data.clone())
                    .color(Srgb::from_hex("#3B82F6"))
                    .size(360.0, 300.0),
                text("Bars shift every frame"),
            )))
        })
    };
    let driver = move |frame: u32| {
        let phase = frame as f32 * 0.2;
        data.set(
            (0..BARS)
                .map(|x| DataPoint::new(x as f32, 40.0 + 30.0 * (x as f32 + phase).sin()))
                .collect::<Vec<_>>(),
        );
    };
    (runtime, Box::new(driver))
}

fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec {
            name: "scrolling_list",
            build: scrolling_list,
        },
        SceneSpec {
            name: "md3_gallery",
            build: md3_gallery,
        },
        SceneSpec {
            name: "animated",
            build: animated,
        },
        SceneSpec {
            name: "text_heavy",
            build: text_heavy,
        },
        SceneSpec {
            name: "chart",
            build: chart,
        },
    ]
}

fn run_scene(spec: &SceneSpec, gpu: &OffscreenGpuContext, warmup: u32, frames: u32) -> SceneReport {
    let (mut runtime, mut drive) = (spec.build)(gpu);
    let gpu_identity = runtime.gpu_identity();
    let timestamp_queries = gpu_identity
        .device_features
        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    let gpu_record = GpuRecord {
        adapter: gpu_identity.adapter.name.clone(),
        device_type: format!("{:?}", gpu_identity.adapter.device_type),
        backend: format!("{:?}", gpu_identity.adapter.backend),
        driver: gpu_identity.adapter.driver.clone(),
        driver_info: gpu_identity.adapter.driver_info.clone(),
        vendor: gpu_identity.adapter.vendor,
        device: gpu_identity.adapter.device,
        adapter_features: format!("{:?}", gpu_identity.adapter_features),
        device_features: format!("{:?}", gpu_identity.device_features),
        timestamp_queries,
    };

    let start = Instant::now();
    let mut at = start;
    for frame in 0..warmup {
        drive(frame);
        let _ = runtime.pump_at(false, at);
        at += FRAME_STEP;
    }

    let mut records = Vec::with_capacity(frames as usize);
    for index in 0..frames {
        drive(warmup + index);
        let result = runtime.pump_at(false, at);
        at += FRAME_STEP;
        let stages = result.stages;
        let phases = &result.profile.phases;
        let counters = &result.profile.counters;
        records.push(FrameRecord {
            frame: index,
            rebuilt: result.rebuilt,
            update_ns: ns(stages.update),
            layout_ns: ns(stages.layout),
            encode_ns: ns(stages.encode),
            content_gpu_ns: stages.content_gpu.map(ns),
            compositor_gpu_ns: stages.compositor_gpu.map(ns),
            gpu_wait_ns: ns(stages.gpu_wait),
            readback_ns: ns(stages.readback),
            total_ns: ns(result.profile.total),
            layout_signature: runtime.layout_signature(),
            phases: BTreeMap::from([
                ("executor_before_ns", ns(phases.executor_before)),
                ("input_ns", ns(phases.input)),
                ("animation_ns", ns(phases.animation)),
                ("rebuild_ns", ns(phases.rebuild)),
                ("build_content_ns", ns(phases.build_content)),
                ("scene_dispatch_ns", ns(phases.scene_dispatch)),
                ("scene_finish_ns", ns(phases.scene_finish)),
                ("acquire_ns", ns(phases.acquire)),
                ("render_ns", ns(phases.render)),
                ("present_ns", ns(phases.present)),
                ("executor_after_ns", ns(phases.executor_after)),
            ]),
            counters: {
                let mut metrics = BTreeMap::from([
                    ("rebuild_iterations", u64::from(counters.rebuild_iterations)),
                    (
                        "measurement_cache_hits",
                        u64::from(counters.measurement_cache_hits),
                    ),
                    (
                        "measurement_cache_misses",
                        u64::from(counters.measurement_cache_misses),
                    ),
                    ("scene_layers", u64::from(counters.scene_layers)),
                    (
                        "legacy_scene_layers",
                        u64::from(counters.legacy_scene_layers),
                    ),
                    ("gpu_surface_layers", u64::from(counters.gpu_surface_layers)),
                    (
                        "direct_gpu_surfaces",
                        u64::from(counters.direct_gpu_surfaces),
                    ),
                    ("clip_layers", u64::from(counters.clip_layers)),
                    (
                        "applied_filter_count",
                        u64::from(counters.applied_filter_count),
                    ),
                    ("rendered", u64::from(counters.rendered)),
                ]);
                counters.frame_work.record_into(&mut metrics);
                metrics
            },
        });
    }

    let mut stage_summaries = BTreeMap::new();
    let collect = |pick: fn(&FrameRecord) -> u64| {
        Some(summarize(records.iter().map(pick).collect::<Vec<u64>>()))
    };
    stage_summaries.insert("update", collect(|f| f.update_ns));
    stage_summaries.insert("layout", collect(|f| f.layout_ns));
    stage_summaries.insert("encode", collect(|f| f.encode_ns));
    // GPU stages: `None` in the report means "the device cannot timestamp",
    // never "the GPU took zero time".
    stage_summaries.insert(
        "content_gpu",
        timestamp_queries.then(|| {
            summarize(
                records
                    .iter()
                    .filter_map(|f| f.content_gpu_ns)
                    .collect::<Vec<u64>>(),
            )
        }),
    );
    stage_summaries.insert(
        "compositor_gpu",
        timestamp_queries.then(|| {
            summarize(
                records
                    .iter()
                    .filter_map(|f| f.compositor_gpu_ns)
                    .collect::<Vec<u64>>(),
            )
        }),
    );
    stage_summaries.insert("gpu_wait", collect(|f| f.gpu_wait_ns));
    stage_summaries.insert("readback", collect(|f| f.readback_ns));
    stage_summaries.insert("total", collect(|f| f.total_ns));

    SceneReport {
        scene: spec.name,
        window_width: WINDOW_WIDTH,
        window_height: WINDOW_HEIGHT,
        scale_factor: SCALE_FACTOR,
        warmup_frames: warmup,
        gpu: gpu_record,
        stages: stage_summaries,
        frames: records,
    }
}

fn main() {
    // `info!` progress lines by default; RUST_LOG overrides.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let mut out_dir = PathBuf::from("frame-profile-out");
    let mut warmup = 20u32;
    let mut frames = 120u32;
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out-dir" => {
                out_dir = PathBuf::from(args.next().expect("--out-dir needs a path"));
            }
            "--warmup" => {
                warmup = args
                    .next()
                    .expect("--warmup needs a count")
                    .parse()
                    .expect("--warmup needs a count");
            }
            "--frames" => {
                frames = args
                    .next()
                    .expect("--frames needs a count")
                    .parse()
                    .expect("--frames needs a count");
            }
            "--scene" => {
                only = Some(args.next().expect("--scene needs a name"));
            }
            other => panic!("unknown argument {other}"),
        }
    }

    // One device for every scene: adapter identity is identical across
    // reports, and a soft-GPU host does not pay a device request per scene.
    let gpu = OffscreenGpuContext::new_for_tests_blocking();
    fs::create_dir_all(&out_dir).expect("failed to create output directory");

    for spec in scenes() {
        if only.as_deref().is_some_and(|name| name != spec.name) {
            continue;
        }
        tracing::info!("[frame_profile] scene {} …", spec.name);
        let report = run_scene(&spec, &gpu, warmup, frames);
        let path = out_dir.join(format!("{}.json", spec.name));
        fs::write(
            &path,
            serde_json::to_string_pretty(&report).expect("report serialization failed"),
        )
        .expect("failed to write report");
        // Every measured frame must have rendered — an idle pump yields no
        // frame at all and silently drops a sample.
        let rendered = report
            .frames
            .iter()
            .all(|frame| frame.counters["rendered"] == 1);
        assert!(rendered, "scene {} had idle (unrendered) frames", spec.name);
        tracing::info!("[frame_profile] wrote {}", path.display());
        gpu.reclaim();
    }
}
