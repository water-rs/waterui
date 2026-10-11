//! Destructive Porter-Duff scenes must render against the surface clear
//! colour through the bench adapters: the scene root maps onto the engine
//! surface root, as in the oracle (#151). Each test runs only with its
//! adapter feature enabled; a default-feature nextest run skips both.

#![cfg(any(feature = "cherenkov", feature = "cherenkov-cpu"))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// The corpus scenes whose result depends on the backdrop the scene root's
/// children composite against, including their `-clip` and `-solid`
/// variants: the six destructive operators plus `dest-out`, `xor` and
/// `plus-lighter`, which all read the canvas under the group.
const DESTRUCTIVE_PREFIXES: &[&str] = &[
    "blend-clear",
    "blend-src",
    "blend-src-in",
    "blend-src-out",
    "blend-dest-in",
    "blend-dest-atop",
    "blend-dest-out",
    "blend-xor",
    "blend-plus-lighter",
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

/// Copies the destructive scenes into a scratch corpus, renders them all
/// with `engine`, and asserts every scene's `flip_mean` stays under the
/// suite's 0.02 bound. Returns nothing; panics with the failing scenes.
fn assert_destructive_scenes(engine: &str) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "cherenkov-bench-destructive-{engine}-{}-{nonce}",
        std::process::id()
    ));
    let corpus_dir = base.join("corpus");
    let out_dir = base.join("out");
    std::fs::create_dir_all(&corpus_dir).expect("create scratch corpus");
    std::fs::create_dir_all(&out_dir).expect("create report directory");

    let mut scenes = Vec::new();
    for entry in std::fs::read_dir(corpus()).expect("read scenes/corpus") {
        let entry = entry.expect("corpus entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let destructive = DESTRUCTIVE_PREFIXES.iter().any(|prefix| {
            name.strip_prefix(prefix).is_some_and(|rest| {
                rest.is_empty()
                    || rest.starts_with("-clip")
                    || rest.starts_with("-solid")
                    || rest == "-p3"
                    || rest == "-hdr"
            })
        });
        if destructive {
            copy_dir(&entry.path(), &corpus_dir.join(&name));
            scenes.push(name);
        }
    }
    assert!(
        scenes.len() >= 18,
        "expected at least 18 destructive corpus scenes, found {}",
        scenes.len()
    );

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
            if let Some(mean) = report["metrics"]["flip_mean"].as_f64() {
                measured += 1;
                if mean >= 0.02 {
                    failures.push(format!("{}: {mean:.4}", report["scene"]));
                }
            }
        }
    }
    assert_eq!(
        measured,
        scenes.len(),
        "expected {} render reports, found {measured}",
        scenes.len()
    );
    assert!(
        failures.is_empty(),
        "destructive scenes over 0.02 flip_mean: {}",
        failures.join(", ")
    );
}

/// GPU adapter: destructive blends must not isolate the scene root.
#[cfg(feature = "cherenkov")]
#[test]
fn gpu_destructive_scenes_match_the_oracle() {
    assert_destructive_scenes("cherenkov");
}

/// CPU adapter: the same scene-root mapping.
#[cfg(feature = "cherenkov-cpu")]
#[test]
fn cpu_destructive_scenes_match_the_oracle() {
    assert_destructive_scenes("cherenkov-cpu");
}
