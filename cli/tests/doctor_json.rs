//! `water doctor --json` end-to-end coverage.
//!
//! Runs the built `water` binary and deserializes every stdout line. Item
//! records use the same `DoctorItemRecord` schema the command serializes, so
//! a schema drift fails to deserialize rather than string-matching.

use std::process::Command;

use waterui_cli::toolchain::doctor::{DoctorItemRecord, ids};

#[test]
fn doctor_json_emits_typed_item_records_for_every_check() {
    let home = tempfile::tempdir().expect("scratch home for the child process");
    let output = Command::new(env!("CARGO_BIN_EXE_water"))
        .args(["--json", "doctor"])
        // Redirect the child's home so `ensure_global_config` never writes
        // `~/.water/config.toml` on the machine running the tests. (dirs'
        // Windows backend consults the known-folder API, so this is only
        // effective on Unix; the write itself is a small config file the CLI
        // owns anyway.)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env_remove("RUST_LOG")
        .output()
        .expect("spawn `water doctor --json`");
    assert!(
        output.status.success(),
        "`water doctor --json` must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("doctor --json emits utf-8");
    assert!(!stdout.trim().is_empty(), "doctor --json emitted nothing");

    let mut items = Vec::new();
    for (line_number, line) in stdout.lines().enumerate() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("stdout line {line_number} is not a JSON record: {error}: {line:?}")
        });
        if record.get("type").and_then(serde_json::Value::as_str) == Some("doctor-item") {
            items.push(serde_json::from_value::<DoctorItemRecord>(record).unwrap_or_else(
                |error| {
                    panic!("stdout line {line_number} fails the DoctorItemRecord schema: {error}: {line:?}")
                },
            ));
        }
    }

    let emitted_ids: Vec<&str> = items.iter().map(|item| item.id.as_ref()).collect();
    assert_eq!(emitted_ids, ids::ALL, "doctor item set/order drifted");

    for item in &items {
        assert!(
            ["ok", "missing", "skipped"].contains(&item.status.as_ref()),
            "unexpected status {:?} on {}",
            item.status,
            item.id
        );
        if item.status == "missing" {
            assert!(
                item.message
                    .as_deref()
                    .is_some_and(|message| !message.is_empty()),
                "missing item {} must carry a diagnostic",
                item.id
            );
        }
    }

    // Skipped identity is a `cfg!` property of the platform, not machine
    // state — assert it exactly, in emission order.
    let skipped_ids: Vec<&str> = items
        .iter()
        .filter(|item| item.status == "skipped")
        .map(|item| item.id.as_ref())
        .collect();
    let expected_skipped: Vec<&str> = ids::ALL
        .iter()
        .copied()
        .filter(|id| {
            let apple_item = matches!(
                *id,
                ids::XCODE
                    | ids::IOS_SDK
                    | ids::IOS_SIMULATOR_SDK
                    | ids::IOS_SIMULATORS
                    | ids::MACOS_SDK
            );
            let linux_item = matches!(*id, ids::LINUX_SYSTEM_PACKAGES | ids::GTK4);
            (apple_item && !cfg!(target_os = "macos"))
                || (linux_item && !cfg!(target_os = "linux"))
                || (*id == ids::WINDOWS_ARM64_LLVM
                    && !cfg!(all(target_os = "windows", target_arch = "aarch64")))
        })
        .collect();
    assert_eq!(skipped_ids, expected_skipped);
}
