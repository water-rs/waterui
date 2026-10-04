//! `cherenkov-bench creation` — the issue-#170 B1 creation ledger.
//!
//! One [`MemorySnapshot`] and a wall-clock `elapsed_ms` timestamp per
//! engine-creation boundary: the empty
//! process, then every [`CreationPhase`] the engine reports (instance,
//! adapter, device, layouts, modules, each pipeline family, buffers,
//! atlas, bind groups, timestamp resources), then the bench-side phases —
//! engine constructed, first surface, first upload and first completed
//! submission, and teardown. `--stop-after PHASE` aborts creation just
//! past a boundary, the ledger's one-factor ablation handle.
//!
//! A `--cycles N` run repeats create → first frame → drop inside one
//! process, the create/drop retention row of the selection loop.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cherenkov::kurbo::Rect;
use cherenkov::{Draw as _, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::interop::{SharedDevice, wgpu};
use cherenkov_gpu::{CreationPhase, Gpu, GpuConfig};
use serde::Serialize;

use crate::memory::{
    AdapterMemory, EngineBytes, MemorySnapshot, Reading, SampleDetail, wgpu_allocator,
    wgpu_vk_memory_budget,
};
use crate::{BenchError, conditions, report::Conditions};

/// One ledger row: the phase boundary and what was measured there.
#[derive(Serialize)]
struct Row {
    /// Which create/drop cycle of the run produced it.
    cycle: u32,
    /// The boundary's name: a [`CreationPhase`] or a bench-side marker
    /// (`process`, `engine`, `surface`, `first_submission`,
    /// `first_complete`, `teardown_device`, `teardown`).
    phase: String,
    /// Wall-clock milliseconds from the cycle's `process` baseline to
    /// this boundary; a phase's duration is the difference between its
    /// row and the previous row's.
    elapsed_ms: f64,
    /// Engine, allocator and process memory at the boundary.
    #[serde(flatten)]
    snapshot: MemorySnapshot,
}

/// `duration` in fractional milliseconds for [`Row::elapsed_ms`].
fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// One engine-side boundary the [`cherenkov_gpu::CreationProbe`] collects
/// before the `Engine` exists, folded into [`Row`]s once creation returns.
struct ProbeRow {
    /// The phase that completed.
    phase: CreationPhase,
    /// Wall-clock milliseconds from the cycle's `process` baseline.
    elapsed_ms: f64,
    /// Memory snapshot at the boundary.
    snapshot: MemorySnapshot,
}

/// The `creation` report.
#[derive(Serialize)]
struct CreationReport {
    /// Adapter provenance: name, backend, driver.
    device: crate::DeviceInfo,
    /// Phases in order; `stopped` marks where a `--stop-after` ablation
    /// ended creation.
    rows: Vec<Row>,
    /// The `--stop-after` phase creation aborted at, if any.
    stopped_after: Option<String>,
    /// Host thermal state at the run's start.
    conditions: Conditions,
}

/// A process/allocator snapshot with whatever device-level readings are
/// available; before the device exists every device-sourced reading is
/// honestly unavailable.
fn snapshot(
    adapter: Option<&wgpu::Adapter>,
    device: Option<&wgpu::Device>,
    engine: Reading<EngineBytes>,
) -> MemorySnapshot {
    let (wgpu_allocator, vk_memory_budget) = match (adapter, device) {
        (Some(adapter), Some(device)) => {
            let info = adapter.get_info();
            (
                wgpu_allocator(device, info.backend),
                wgpu_vk_memory_budget(device, info.backend, &info.name),
            )
        }
        _ => (
            Reading::unavailable("device not yet created"),
            Reading::unavailable("device not yet created"),
        ),
    };
    MemorySnapshot::capture(
        AdapterMemory {
            engine,
            wgpu_allocator,
            skia_budgeted: Reading::unavailable("not a Skia adapter"),
            vk_memory_budget,
        },
        SampleDetail::Full,
    )
}

/// The engine's own accounting once it exists.
fn engine_bytes(engine: &Engine<Gpu>) -> Reading<EngineBytes> {
    let usage = engine.memory();
    Reading::Measured(EngineBytes {
        cpu_bytes: usage.cpu.0,
        gpu_bytes: usage.gpu.0,
        backdrop_capture_bytes: usage.backdrop_captures.0,
    })
}

/// The CLI `--stop-after` spelling of a [`CreationPhase`].
const fn phase_name(phase: CreationPhase) -> &'static str {
    match phase {
        CreationPhase::Instance => "instance",
        CreationPhase::Adapter => "adapter",
        CreationPhase::Device => "device",
        CreationPhase::Layouts => "layouts",
        CreationPhase::ShaderModules => "shader-modules",
        CreationPhase::CorePipelines => "core-pipelines",
        CreationPhase::Buffers => "buffers",
        CreationPhase::Atlas => "atlas",
        CreationPhase::BindGroups => "bind-groups",
        CreationPhase::Timestamps => "timestamps",
        CreationPhase::ShadowBlur => "shadow-blur",
        CreationPhase::ExternalNative => "external-native",
        CreationPhase::Complete => "complete",
    }
}

/// Parses a `--stop-after` phase name.
#[must_use]
pub fn parse_phase(name: &str) -> Option<CreationPhase> {
    [
        CreationPhase::Instance,
        CreationPhase::Adapter,
        CreationPhase::Device,
        CreationPhase::Layouts,
        CreationPhase::ShaderModules,
        CreationPhase::CorePipelines,
        CreationPhase::Buffers,
        CreationPhase::Atlas,
        CreationPhase::BindGroups,
        CreationPhase::Timestamps,
        CreationPhase::ShadowBlur,
        CreationPhase::ExternalNative,
        CreationPhase::Complete,
    ]
    .into_iter()
    .find(|phase| phase_name(*phase) == name)
}

/// One engine create → first frame → drop segment of the ledger.
/// `shared` and `engine` are owned by the caller; the surface is local so
/// it releases before them. `start` is the cycle's `process` baseline.
fn measure_cycle(
    rows: &mut Vec<Row>,
    cycle: u32,
    start: Instant,
    shared: &SharedDevice,
    engine: &Engine<Gpu>,
) -> Result<crate::DeviceInfo, BenchError> {
    let info = engine.info();
    let device_info = crate::DeviceInfo {
        adapter: Some(info.name.clone()),
        backend: Some(info.backend.clone()),
        driver: Some(info.driver.clone()),
        driver_info: Some(info.driver_info.clone()),
        vendor: Some(info.vendor),
        device: Some(info.device),
        ..crate::DeviceInfo::default()
    };
    let mut push = |phase: &str| {
        rows.push(Row {
            cycle,
            phase: phase.to_string(),
            elapsed_ms: millis(start.elapsed()),
            snapshot: snapshot(
                Some(&shared.adapter),
                Some(&shared.device),
                engine_bytes(engine),
            ),
        });
    };
    push("engine");
    // First surface and first drawn frame: the upload, staging and
    // deferred-initialization delta.
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16))
        .map_err(|error| BenchError::Gpu(format!("cherenkov surface: {error}")))?;
    push("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0., 0., 64., 64.), WorkingColor::WHITE);
        }));
    });
    engine
        .render(FrameTime::now())
        .map_err(|error| BenchError::Engine(format!("first frame: {error}")))?;
    push("first_submission");
    let _ = shared.device.poll(wgpu::PollType::wait_indefinitely());
    push("first_complete");
    drop(surface);
    Ok(device_info)
}

/// Builds the shared device and engine under a [`CreationProbe`]
/// collecting one snapshot per phase into `collected`; `aborted` marks
/// the `--stop-after` request so the caller can tell the ablation abort
/// from a real failure.
fn create(
    stop_after: Option<CreationPhase>,
    start: Instant,
    collected: &Arc<Mutex<Vec<ProbeRow>>>,
    aborted: &Arc<AtomicBool>,
) -> Result<(SharedDevice, Engine<Gpu>), cherenkov::EngineError> {
    let probe = cherenkov_gpu::CreationProbe::new({
        let collected = Arc::clone(collected);
        let aborted = Arc::clone(aborted);
        move |point| {
            collected.lock().expect("creation rows").push(ProbeRow {
                phase: point.phase,
                elapsed_ms: millis(start.elapsed()),
                snapshot: snapshot(
                    point.adapter,
                    point.device,
                    Reading::unavailable("engine not yet constructed"),
                ),
            });
            if Some(point.phase) == stop_after {
                aborted.store(true, Ordering::Release);
                return true;
            }
            false
        }
    });
    let config = GpuConfig {
        // The `measure` runs this ledger explains enable timestamps;
        // the ledger records the instrumentation's own cost too.
        timestamps: true,
        creation_probe: Some(probe),
        ..GpuConfig::default()
    };
    let shared = SharedDevice::create(&config)?;
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        ..config
    })?;
    Ok((shared, engine))
}

/// The creation ledger. Builds the engine through the same
/// `SharedDevice` + `Engine::new` path the `cherenkov` adapter uses —
/// with `timestamps` on, as in `measure` — and snapshots every boundary.
///
/// # Errors
/// [`BenchError`] on engine creation, first-frame render or I/O failure.
/// A `--stop-after` abort is reported in the ledger, not as an error.
///
/// # Panics
/// Only if the probe's row collection is poisoned — it is not shared
/// outside this function.
pub fn run(cycles: u32, stop_after: Option<CreationPhase>, out: &Path) -> Result<(), BenchError> {
    let conditions = conditions::collect(None);
    let mut rows = Vec::new();
    let mut stopped_after = None;
    let mut device_info = crate::DeviceInfo::default();
    let mut cycle = 0;
    while cycle < cycles {
        // The empty-harness baseline: no GPU state exists in the process.
        // `start` is the instant every row's `elapsed_ms` counts from.
        let start = Instant::now();
        rows.push(Row {
            cycle,
            phase: "process".to_string(),
            elapsed_ms: millis(start.elapsed()),
            snapshot: snapshot(
                None,
                None,
                Reading::unavailable("engine not yet constructed"),
            ),
        });
        let collected = Arc::new(Mutex::new(Vec::<ProbeRow>::new()));
        let aborted = Arc::new(AtomicBool::new(false));
        let made = create(stop_after, start, &collected, &aborted);
        rows.extend(
            collected
                .lock()
                .expect("creation rows")
                .drain(..)
                .map(|probe| Row {
                    cycle,
                    phase: phase_name(probe.phase).to_string(),
                    elapsed_ms: probe.elapsed_ms,
                    snapshot: probe.snapshot,
                }),
        );
        let (shared, engine) = match made {
            Ok(pair) => pair,
            Err(error) => {
                if aborted.load(Ordering::Acquire) {
                    stopped_after = stop_after.map(|phase| phase_name(phase).to_string());
                    break;
                }
                return Err(BenchError::Gpu(format!("cherenkov engine: {error}")));
            }
        };
        device_info = measure_cycle(&mut rows, cycle, start, &shared, &engine)?;
        drop(engine);
        // The renderer is gone; the shared device still lets the
        // allocator report what survived the engine's drop.
        let _ = shared.device.poll(wgpu::PollType::wait_indefinitely());
        rows.push(Row {
            cycle,
            phase: "teardown_device".to_string(),
            elapsed_ms: millis(start.elapsed()),
            snapshot: snapshot(
                Some(&shared.adapter),
                Some(&shared.device),
                Reading::unavailable("engine dropped"),
            ),
        });
        drop(shared);
        rows.push(Row {
            cycle,
            phase: "teardown".to_string(),
            elapsed_ms: millis(start.elapsed()),
            snapshot: snapshot(None, None, Reading::unavailable("engine dropped")),
        });
        cycle += 1;
    }
    let report = CreationReport {
        device: device_info,
        rows,
        stopped_after,
        conditions,
    };
    std::fs::write(
        out,
        serde_json::to_string_pretty(&report)
            .map_err(|error| BenchError::Engine(format!("serialize creation report: {error}")))?,
    )?;
    Ok(())
}
