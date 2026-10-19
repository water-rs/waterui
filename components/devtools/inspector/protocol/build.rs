//! Build metadata injection for the `WaterUI` inspector protocol crate.

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=WATERUI_INSPECTOR_PROTOCOL_COMMIT");
    println!("cargo:rerun-if-env-changed=WATERUI_CLI_COMMIT");

    let commit = env::var("WATERUI_INSPECTOR_PROTOCOL_COMMIT")
        .ok()
        .or_else(|| env::var("WATERUI_CLI_COMMIT").ok())
        .or_else(resolve_git_commit)
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=WATERUI_INSPECTOR_PROTOCOL_COMMIT={commit}");
}

fn resolve_git_commit() -> Option<String> {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .arg("-C")
        .arg(&crate_dir)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "HEAD",
            "--git-common-dir",
            "--short=12",
            "HEAD",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let mut lines = stdout.lines();
    let head = PathBuf::from(lines.next().expect("git returned no HEAD path"));
    let common_dir = PathBuf::from(lines.next().expect("git returned no common directory"));
    let commit = lines
        .next()
        .expect("git returned no commit")
        .trim()
        .to_string();

    // The embedded commit can only move with what this worktree's HEAD
    // resolves to. A linked worktree has its own HEAD file but shares refs
    // with the main repository.
    println!("cargo:rerun-if-changed={}", head.display());

    let output = Command::new("git")
        .arg("-C")
        .arg(&crate_dir)
        .args(["rev-parse", "--symbolic-full-name", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let symbolic = String::from_utf8(output.stdout).ok()?;
    let symbolic = symbolic.trim();

    // A detached HEAD prints itself and the HEAD file already covers it.
    if symbolic != "HEAD" {
        // HEAD names a ref whose value lives in its loose file or in packed
        // storage. --git-path resolves the loose file: refs/heads/* lands in
        // the common dir, per-worktree refs in the worktree gitdir. Watch the
        // path even while absent so a ref materialized later still reruns.
        let output = Command::new("git")
            .arg("-C")
            .arg(&crate_dir)
            .args([
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                symbolic,
            ])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let loose = String::from_utf8(output.stdout).ok()?;
        println!("cargo:rerun-if-changed={}", loose.trim());

        // Packed storage can hold or later receive the branch ref: packing
        // deletes the loose file and writes packed-refs, and a reftable
        // repository stores refs under reftable instead.
        for path in [common_dir.join("packed-refs"), common_dir.join("reftable")] {
            if path.try_exists().unwrap_or_else(|error| {
                panic!("Failed to inspect Git metadata {}: {error}", path.display())
            }) {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    (!commit.is_empty()).then_some(commit)
}
