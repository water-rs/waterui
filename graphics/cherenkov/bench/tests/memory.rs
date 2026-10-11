//! Verifies that measure reports include CPU memory snapshots.

#![cfg(feature = "cherenkov-cpu")]

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

#[test]
fn measure_reports_memory_snapshots_for_cherenkov_cpu() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let out_dir = std::env::temp_dir().join(format!(
        "cherenkov-bench-memory-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&out_dir).expect("create report directory");
    let report_path = out_dir.join("measure.json");
    let scene_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scenes/perf/map");
    let output = Command::new(env!("CARGO_BIN_EXE_cherenkov-bench"))
        .args([
            "measure",
            "--engine",
            "cherenkov-cpu",
            "--scene",
            scene_path.to_str().expect("scene path is UTF-8"),
            "--frames",
            "1",
            "--warmup",
            "1",
            "--out",
            report_path.to_str().expect("report path is UTF-8"),
        ])
        .output()
        .expect("run cherenkov-bench");
    assert!(
        output.status.success(),
        "bench failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(&report_path).expect("read memory report"))
            .expect("parse memory report");
    for point in ["idle", "steady", "peak"] {
        assert!(
            report["memory"][point].is_object(),
            "missing memory.{point} snapshot: {report}"
        );
        assert!(
            report["memory"][point]["engine"]["measured"].is_object(),
            "memory.{point}.engine is not Measured: {report}"
        );
        assert!(
            report["memory"][point]["engine"]["measured"]["backdrop_capture_bytes"].is_number(),
            "memory.{point}.engine lacks backdrop_capture_bytes: {report}"
        );
    }
    std::fs::remove_dir_all(out_dir).expect("remove report directory");
}
