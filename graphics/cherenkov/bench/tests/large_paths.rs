//! The large-path corpus scenes (water-rs/waterui#2546) against the oracle,
//! under thresholds 1–3 of the coverage specification's section 4.3
//! (`docs/design/2536-coverage-spec.md`).
//!
//! Threshold 2 is the ceiling — `flip_max <= 0.15` and
//! `max_local_error <= 0.09` absolute on every path scene, and `flip_mean`
//! per scene `max(0.006, the CPU sweep's own flip_mean)`. The `flip_mean`
//! ceiling is calibrated against the sweep: a scene made almost entirely
//! of antialiased edges — the dense dash, zero-length-dash and curve
//! scenes — exceeds 0.006 on the sweep with no perceptible difference, so
//! there the sweep's value is the scene's ceiling and threshold 1
//! governs it, while `flip_max` and `max_local_error` keep their
//! ceilings on every scene. The test therefore renders the scratch
//! corpus a second time with `--engine cherenkov-cpu` to read each
//! scene's sweep `flip_mean`.
//!
//! Threshold 3 says the corpus's worst scene per metric must keep its
//! identity, so a large-path scene has to stay at or under the value of
//! the corpus scene holding that metric's section-4.2 maximum:
//! `glyph-stroke-dashed` for `flip_mean`,
//! `stroke-join-continuous-sharp-round` for `flip_max` and
//! `stroke-dash-hdr` for `max_local_error`. Those maxima were measured
//! once on one machine and would be stale here, so the calibration
//! scenes are copied into the scratch corpus beside the large-path
//! scenes and rendered in the same `cherenkov` render — the bound is
//! whatever the calibration scene measures on this adapter and build.
//! Threshold 3 is calibrated against the sweep like threshold 2's
//! `flip_mean`: a scene the sweep already renders worse than the
//! calibration scene is held to the sweep's own value instead, since the
//! route has not created that worst scene. Both bounds are asserted on
//! every scene.
//!
//! Threshold 1 — no regression against dev per scene — cannot apply to a
//! scene dev does not have; it starts applying to these scenes once this
//! commit is dev, and it is the Ir-gate-style comparison the GPU route of
//! water-rs/waterui#2543 runs, not something this test can anticipate.
//!
//! These scenes are large and self-intersecting so that the GPU coverage
//! route selects them by itself (decision T2) when #2543 lands: the same
//! gate then measures that route instead of the CPU sweep, with no change
//! here. The route-to-route comparison of threshold 4 is
//! `cherenkov-gpu`'s `render::path::route_diff`, which needs the test-only
//! route hook and so lives inside that crate.
//!
//! The test builds and runs only with both the `cherenkov` and
//! `cherenkov-cpu` features, which put both engines into the bench
//! binary.

#![cfg(all(feature = "cherenkov", feature = "cherenkov-cpu"))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// The scenes `generate-corpus` writes for #2546, and the stroke scenes
/// of #2544.
const SCENES: &[&str] = &[
    "stroke-road-network-p3",
    "stroke-road-network-hdr",
    "path-dense-overlap-nonzero-p3",
    "path-dense-overlap-evenodd-p3",
    "path-dense-overlap-nonzero-hdr",
    "path-dense-overlap-evenodd-hdr",
    "path-sparse-bounds-nonzero-p3",
    "path-sparse-bounds-evenodd-hdr",
    "stroke-gpu-joins-miter-p3",
    "stroke-gpu-joins-miter-hdr",
    "stroke-gpu-joins-bevel-p3",
    "stroke-gpu-joins-bevel-hdr",
    "stroke-gpu-joins-round-p3",
    "stroke-gpu-joins-round-hdr",
    "stroke-gpu-joins-miter-lim15-p3",
    "stroke-gpu-joins-miter-lim15-hdr",
    "stroke-gpu-caps-square-round-p3",
    "stroke-gpu-caps-square-round-hdr",
    "stroke-gpu-caps-round-butt-p3",
    "stroke-gpu-caps-round-butt-hdr",
    "stroke-gpu-caps-butt-square-p3",
    "stroke-gpu-caps-butt-square-hdr",
    "stroke-gpu-dash-p3",
    "stroke-gpu-dash-hdr",
    "stroke-gpu-dash-zero-p3",
    "stroke-gpu-dash-zero-hdr",
    "stroke-gpu-dash-ties-p3",
    "stroke-gpu-dash-ties-hdr",
    "stroke-gpu-dash-dense-p3",
    "stroke-gpu-dash-dense-hdr",
    "stroke-gpu-curves-p3",
    "stroke-gpu-curves-hdr",
    "stroke-gpu-hairline-w025-p3",
    "stroke-gpu-hairline-w005-hdr",
    "stroke-gpu-transformed-p3",
    "stroke-gpu-transformed-hdr",
];

/// Section 4.3 threshold 2's absolute ceilings.
const FLIP_MEAN_CEILING: f64 = 0.006;
const FLIP_MAX_CEILING: f64 = 0.15;
const MAX_LOCAL_ERROR_CEILING: f64 = 0.09;

/// Per metric, the calibration scene that holds its section-4.2 corpus
/// maximum (threshold 3's bound, measured in this run) and threshold 2's
/// absolute ceiling. `flip_mean`'s ceiling is a floor: a scene whose CPU
/// sweep `flip_mean` is higher is held to the sweep's own value instead.
const METRIC_BOUNDS: &[(&str, &str, f64)] = &[
    ("flip_mean", "glyph-stroke-dashed", FLIP_MEAN_CEILING),
    (
        "flip_max",
        "stroke-join-continuous-sharp-round",
        FLIP_MAX_CEILING,
    ),
    (
        "max_local_error",
        "stroke-dash-hdr",
        MAX_LOCAL_ERROR_CEILING,
    ),
];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scenes/corpus")
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create scene dir");
    for entry in std::fs::read_dir(src).expect("read scene dir") {
        let entry = entry.expect("scene dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy scene file");
        }
    }
}

/// Renders `corpus_dir` with `engine` into `out_dir` and returns each
/// report's metrics keyed by scene name.
fn render_reports(corpus_dir: &Path, out_dir: &Path, engine: &str) -> HashMap<String, Value> {
    std::fs::create_dir_all(out_dir).expect("create report directory");
    let output = Command::new(env!("CARGO_BIN_EXE_cherenkov-bench"))
        .args([
            "render",
            "--engine",
            engine,
            "--corpus",
            corpus_dir.to_str().expect("corpus path is UTF-8"),
            "--out-dir",
            out_dir.to_str().expect("report path is UTF-8"),
        ])
        .output()
        .expect("run cherenkov-bench");
    assert!(
        output.status.success(),
        "bench render --engine {engine} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reports = HashMap::new();
    for entry in std::fs::read_dir(out_dir).expect("read report dir") {
        let path = entry.expect("report entry").path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let report: Value = serde_json::from_slice(&std::fs::read(&path).expect("read report"))
                .expect("parse report");
            let scene = report["scene"].as_str().expect("scene name").to_owned();
            reports.insert(scene, report["metrics"].clone());
        }
    }
    reports
}

/// `scene`'s `metric` value in `reports`.
fn read(reports: &HashMap<String, Value>, scene: &str, metric: &str) -> f64 {
    reports
        .get(scene)
        .unwrap_or_else(|| panic!("{scene} has no render report"))[metric]
        .as_f64()
        .unwrap_or_else(|| panic!("{scene}: {metric}"))
}

/// Renders the large-path scenes and the calibration scenes with
/// `cherenkov` and again with `cherenkov-cpu` (the CPU sweep the
/// threshold-2 `flip_mean` ceiling is calibrated against), and holds
/// every large-path scene to thresholds 2 and 3.
#[test]
fn large_paths_match_the_oracle() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "cherenkov-bench-large-paths-{}-{nonce}",
        std::process::id()
    ));
    let corpus_dir = base.join("corpus");
    std::fs::create_dir_all(&corpus_dir).expect("create scratch corpus");
    for scene in SCENES
        .iter()
        .copied()
        .chain(METRIC_BOUNDS.iter().map(|&(_, scene, _)| scene))
    {
        let from = corpus().join(scene);
        assert!(
            from.join("scene.json").is_file(),
            "{scene} is missing: regenerate the scene tree with `uv run --python 3.12 --with-requirements scenes/fonts/tools/requirements.txt python scenes/tools/generate.py`"
        );
        copy_dir(&from, &corpus_dir.join(scene));
    }

    let reports = render_reports(&corpus_dir, &base.join("out"), "cherenkov");
    let sweep = render_reports(&corpus_dir, &base.join("sweep"), "cherenkov-cpu");

    let expected = SCENES.len() + METRIC_BOUNDS.len();
    assert_eq!(
        reports.len(),
        expected,
        "expected {expected} render reports, found {}",
        reports.len()
    );
    assert_eq!(
        sweep.len(),
        expected,
        "expected {expected} CPU sweep reports, found {}",
        sweep.len()
    );

    let mut failures = Vec::new();
    for &(metric, cal_scene, ceiling) in METRIC_BOUNDS {
        let bound = read(&reports, cal_scene, metric);
        for scene in SCENES {
            let value = read(&reports, scene, metric);
            let sweep_value = read(&sweep, scene, metric);
            // Threshold 2's `flip_mean` ceiling is calibrated against the
            // CPU sweep: a scene whose sweep `flip_mean` already exceeds
            // the absolute ceiling is held to the sweep's own value.
            let (scene_ceiling, which) = if metric == "flip_mean" && sweep_value > ceiling {
                (sweep_value, "the CPU sweep's")
            } else {
                (ceiling, "the absolute")
            };
            if value > scene_ceiling {
                failures.push(format!(
                    "{scene}: threshold 2 {metric} {value:.6} > {which} ceiling \
                     {scene_ceiling:.6}"
                ));
            }
            // Threshold 3 is calibrated the same way: a scene the sweep
            // already renders worse than the calibration scene is no new
            // worst scene as long as the route does not exceed the sweep.
            let (scene_bound, whose) = if sweep_value > bound {
                (sweep_value, "the CPU sweep's".to_owned())
            } else {
                (bound, format!("calibration scene {cal_scene}'s"))
            };
            if value > scene_bound {
                failures.push(format!(
                    "{scene}: threshold 3 {metric} {value:.6} > {whose} {scene_bound:.6}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "large-path scenes over thresholds 2-3: {}",
        failures.join(", ")
    );
    std::fs::remove_dir_all(&base).expect("remove the scratch corpus");
}
