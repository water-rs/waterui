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
        .arg(crate_dir)
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

    // A linked worktree has its own HEAD but shares refs with the main
    // repository. Watch the ref directory as well as an existing packed
    // file: packing deletes loose refs, and a later commit recreates them.
    for path in [
        head,
        common_dir.join("refs"),
        common_dir.join("packed-refs"),
        common_dir.join("reftable"),
    ] {
        if path.try_exists().unwrap_or_else(|error| {
            panic!("Failed to inspect Git metadata {}: {error}", path.display())
        }) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    (!commit.is_empty()).then_some(commit)
}
