//! `measure --energy` is mandatory: when the platform's meter can't be
//! read, the command fails naming the meter rather than producing a
//! report without energy.

use std::process::Command;

fn measure(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_cherenkov-bench"))
        .arg("measure")
        .args(args)
        .output()
        .expect("run cherenkov-bench");
    // tracing's fmt layer writes to stdout.
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), log)
}

#[test]
fn energy_fails_when_meter_unreadable() {
    let (ok, log) = measure(&[
        "--engine",
        "test-engine",
        "--scene",
        "/nonexistent-scene",
        "--frames",
        "4",
        "--energy",
        "--out",
        "/nonexistent-dir/report.json",
    ]);
    assert!(!ok, "command unexpectedly succeeded");
    // On Android the error names the unreadable ODPM rail path and the
    // missing permission; on other unixes the meter itself doesn't
    // exist. Both name energy as what failed.
    #[cfg(not(target_vendor = "apple"))]
    assert!(
        log.contains("energy:"),
        "expected an energy error, output was: {log}"
    );
    // On macOS with passwordless sudo the meter probes fine and the
    // run fails later on the unknown engine instead.
    #[cfg(target_vendor = "apple")]
    assert!(
        log.contains("energy:") || log.contains("engine"),
        "output was: {log}"
    );
}

#[test]
fn energy_requires_measured_frames() {
    let (ok, log) = measure(&[
        "--engine",
        "test-engine",
        "--scene",
        "/nonexistent-scene",
        "--frames",
        "0",
        "--energy",
        "--out",
        "/nonexistent-dir/report.json",
    ]);
    assert!(!ok, "command unexpectedly succeeded");
    assert!(
        log.contains("--frames"),
        "expected a frames error, output was: {log}"
    );
}
