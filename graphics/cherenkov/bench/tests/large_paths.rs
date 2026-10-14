//! The large-path corpus scenes (water-rs/waterui#2546) against the oracle,
//! under thresholds 1–3 of the coverage specification's section 4.3
//! (`docs/design/2536-coverage-spec.md`).
//!
//! Threshold 2 is the absolute ceiling — `flip_mean <= 0.006`,
//! `flip_max <= 0.15`, `max_local_error <= 0.09` on every path scene — and
//! threshold 3 says the corpus's worst scene per metric must keep its
//! identity, so a scene added to the corpus has to stay under the maxima
//! those ceilings were derived from: `glyph-stroke-dashed`'s `flip_mean`
//! 0.005326, `stroke-join-continuous-sharp-round`'s `flip_max` 0.135176 and
//! `stroke-dash-hdr`'s `max_local_error` 0.077068 (section 4.2, dev's 65
//! path scenes). Both are asserted here, on the stricter of the two.
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

#![cfg(feature = "cherenkov")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// The scenes `generate-corpus` writes for #2546.
const SCENES: &[&str] = &[
    "stroke-road-network-p3",
    "stroke-road-network-hdr",
    "path-dense-overlap-nonzero-p3",
    "path-dense-overlap-evenodd-p3",
    "path-dense-overlap-nonzero-hdr",
    "path-dense-overlap-evenodd-hdr",
    "path-sparse-bounds-nonzero-p3",
    "path-sparse-bounds-evenodd-hdr",
];

/// Section 4.3 threshold 2's ceilings, tightened to section 4.2's measured
/// maxima by threshold 3: a new scene over one of these would take the
/// corpus's worst-scene identity for that metric.
const FLIP_MEAN: f64 = 0.005_326;
const FLIP_MAX: f64 = 0.135_176;
const MAX_LOCAL_ERROR: f64 = 0.077_068;

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

/// Renders the large-path scenes with `engine` and holds every one of them
/// to thresholds 1–3.
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
    let out_dir = base.join("out");
    std::fs::create_dir_all(&corpus_dir).expect("create scratch corpus");
    std::fs::create_dir_all(&out_dir).expect("create report directory");
    for scene in SCENES {
        let from = corpus().join(scene);
        assert!(
            from.join("scene.json").is_file(),
            "{scene} is missing: regenerate the scene tree with `uv run --python 3.12 --with-requirements scenes/fonts/tools/requirements.txt python scenes/tools/generate.py`"
        );
        copy_dir(&from, &corpus_dir.join(scene));
    }

    let output = Command::new(env!("CARGO_BIN_EXE_cherenkov-bench"))
        .args([
            "render",
            "--engine",
            "cherenkov",
            "--corpus",
            corpus_dir.to_str().expect("corpus path is UTF-8"),
            "--out-dir",
            out_dir.to_str().expect("report path is UTF-8"),
        ])
        .output()
        .expect("run cherenkov-bench");
    assert!(
        output.status.success(),
        "bench failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mut failures = Vec::new();
    let mut measured = 0usize;
    for entry in std::fs::read_dir(&out_dir).expect("read report dir") {
        let path = entry.expect("report entry").path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let report: Value = serde_json::from_slice(&std::fs::read(&path).expect("read report"))
                .expect("parse report");
            let metrics = &report["metrics"];
            let scene = report["scene"].as_str().expect("scene name").to_owned();
            let read = |key: &str| {
                metrics[key]
                    .as_f64()
                    .unwrap_or_else(|| panic!("{scene}: {key}"))
            };
            let (mean, max, mle) = (read("flip_mean"), read("flip_max"), read("max_local_error"));
            measured += 1;
            if mean > FLIP_MEAN || max > FLIP_MAX || mle > MAX_LOCAL_ERROR {
                failures.push(format!(
                    "{scene}: flip_mean {mean:.6}, flip_max {max:.6}, max_local_error {mle:.6}"
                ));
            }
        }
    }
    assert_eq!(
        measured,
        SCENES.len(),
        "expected {} render reports, found {measured}",
        SCENES.len()
    );
    assert!(
        failures.is_empty(),
        "large-path scenes over thresholds 1-3 (flip_mean {FLIP_MEAN}, flip_max {FLIP_MAX}, \
         max_local_error {MAX_LOCAL_ERROR}): {}",
        failures.join(", ")
    );
    std::fs::remove_dir_all(&base).expect("remove the scratch corpus");
}
