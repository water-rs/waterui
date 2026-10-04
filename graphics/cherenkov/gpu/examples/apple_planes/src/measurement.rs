//! A device-counter window with a real completion signal for the host runner.

use std::fs::File;
use std::time::{Duration, Instant};

use cherenkov_bench::energy::{Meter, ios};
use serde::Serialize;

use crate::recorded::Spec;

pub struct Measurement {
    run_id: String,
    ready_at: Instant,
    window: Option<Window>,
    pub finished: bool,
}

struct Window {
    meter: Meter,
    start: Instant,
    frame_times: Vec<f64>,
    cadences: Vec<f64>,
    previous: Option<Instant>,
    before: ios::Snapshot,
}

#[derive(Serialize)]
struct Result {
    run_id: String,
    scenario: String,
    side: u32,
    count: u32,
    in_engine: bool,
    decision: String,
    frames: u32,
    energy: cherenkov_bench::report::EnergyReport,
    frame_ms_p50: f64,
    frame_ms_p99: f64,
    cadence_ms_p50: f64,
    cadence_ms_p99: f64,
    engine_gpu_bytes: u64,
    engine_cpu_bytes: u64,
    before: ios::Snapshot,
    after: ios::Snapshot,
}

fn percentile(values: &mut [f64], percent: usize) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    values[(values.len() * percent).div_ceil(100).saturating_sub(1)]
}

impl Measurement {
    pub fn new(run_id: String) -> Self {
        Meter::probe().expect("device energy counter");
        Self {
            run_id,
            ready_at: Instant::now() + Duration::from_secs(5),
            window: None,
            finished: false,
        }
    }

    pub fn begin_frame(&mut self) {
        if self.window.is_none() && Instant::now() >= self.ready_at && !self.finished {
            let before = ios::snapshot().expect("initial process footprint");
            let meter = Meter::begin(Duration::from_secs(20)).expect("begin energy window");
            self.window = Some(Window {
                meter,
                start: Instant::now(),
                frame_times: Vec::with_capacity(2500),
                cadences: Vec::with_capacity(2500),
                previous: None,
                before,
            });
            crate::log::line("measurement window opened");
        }
    }

    pub fn end_frame(
        &mut self,
        start: Instant,
        end: Instant,
        spec: Spec,
        decision: String,
        memory: impl FnOnce() -> cherenkov::MemoryUsage,
    ) {
        let Some(window) = &mut self.window else {
            return;
        };
        window
            .frame_times
            .push((end - start).as_secs_f64() * 1000.0);
        if let Some(previous) = window.previous.replace(start) {
            window
                .cadences
                .push((start - previous).as_secs_f64() * 1000.0);
        }
        if end - window.start < Duration::from_secs(20) {
            return;
        }
        assert!(
            if spec.engine {
                decision.starts_with("TranslucentAbove")
            } else {
                decision == "promoted"
            },
            "wrong realization: {decision}"
        );
        let mut window = self.window.take().expect("open measurement");
        let frames = u32::try_from(window.frame_times.len()).expect("window frame count");
        let energy = window
            .meter
            .finish(window.start, end, frames)
            .expect("finish device energy window")
            .report;
        let after = ios::snapshot().expect("final process footprint");
        let memory = memory();
        let result = Result {
            run_id: self.run_id.clone(),
            scenario: if spec.animated { "animated" } else { "static" }.into(),
            side: spec.side,
            count: spec.count,
            in_engine: spec.engine,
            decision,
            frames,
            energy,
            frame_ms_p50: percentile(&mut window.frame_times, 50),
            frame_ms_p99: percentile(&mut window.frame_times, 99),
            cadence_ms_p50: percentile(&mut window.cadences, 50),
            cadence_ms_p99: percentile(&mut window.cadences, 99),
            engine_gpu_bytes: memory.gpu.0,
            engine_cpu_bytes: memory.cpu.0,
            before: window.before,
            after,
        };
        let home = std::env::var_os("HOME").expect("iOS sandbox home");
        let path = std::path::PathBuf::from(home).join("Documents/planes-result.json");
        let file = File::create(path).expect("measurement report");
        serde_json::to_writer_pretty(&file, &result).expect("serialize measurement report");
        file.sync_all().expect("persist measurement report");
        self.finished = true;
        crate::log::line("measurement complete");
    }
}
