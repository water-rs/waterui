//! Build metadata injection for the `WaterUI` inspector protocol crate.

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=WATERUI_INSPECTOR_PROTOCOL_COMMIT");
    println!("cargo:rerun-if-env-changed=WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE");

    let crate_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo did not provide CARGO_MANIFEST_DIR"),
    );
    let published_metadata = crate_dir.join(".cargo_vcs_info.json");
    let published = match published_metadata.metadata() {
        Ok(metadata) => {
            assert!(
                metadata.is_file(),
                "Cargo package provenance must be a file"
            );
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => panic!("Failed to inspect Cargo package provenance: {error}"),
    };
    if published {
        println!("cargo:rerun-if-changed={}", published_metadata.display());
    }
    let release = match env::var("WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE") {
        Ok(mode) if mode == "release" => true,
        Ok(mode) if mode == "checkout" => false,
        Ok(mode) => panic!(
            "WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE must be release or checkout, received {mode}"
        ),
        Err(env::VarError::NotPresent) => published,
        Err(error) => panic!("Invalid WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE: {error}"),
    };

    let explicit_commit = match env::var("WATERUI_INSPECTOR_PROTOCOL_COMMIT") {
        Ok(commit) => Some(commit),
        Err(env::VarError::NotPresent) => None,
        Err(error) => panic!("Invalid WATERUI_INSPECTOR_PROTOCOL_COMMIT: {error}"),
    };
    if release {
        assert!(
            explicit_commit.is_none(),
            "A release protocol build must not override its checkout commit"
        );
        return;
    }

    let commit = explicit_commit.or_else(resolve_git_commit).expect(
        "An unreleased Inspector protocol requires a Git checkout; \
         release artifacts must explicitly select the release build mode",
    );
    assert!(
        commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Inspector protocol checkout identity must be a full Git commit"
    );

    println!(
        "cargo:rustc-env=WATERUI_INSPECTOR_PROTOCOL_COMMIT={}",
        commit.to_ascii_lowercase()
    );
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
