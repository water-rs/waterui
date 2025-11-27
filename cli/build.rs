#![allow(missing_docs)]

use std::{path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../Cargo.toml");
    println!("cargo:rerun-if-changed=../backends/apple/Package.swift");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads");

    emit_waterui_version();
    emit_swift_backend_version();
    emit_android_backend_version();
    emit_git_commit_hash();
}

fn emit_git_commit_hash() {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !hash.is_empty() {
                println!("cargo:rustc-env=GIT_COMMIT_HASH={hash}");
            } else {
                println!("cargo:rustc-env=GIT_COMMIT_HASH=unknown");
            }
        }
        _ => {
            println!("cargo:rustc-env=GIT_COMMIT_HASH=unknown");
        }
    }
}

fn emit_waterui_version() {
    let repo_path = Path::new("..");
    let version = git_tag_version(repo_path, "waterui", "waterui-v*");

    if let Some(version) = version {
        println!("cargo:rustc-env=WATERUI_VERSION={version}");
    } else {
        eprintln!(
            "cargo:warning=No stable version tag found for 'waterui'. Use `water create --dev` until a release is cut."
        );
        println!("cargo:rustc-env=WATERUI_VERSION=");
    }
}

fn emit_swift_backend_version() {
    let repo_path = Path::new("../backends/apple");
    let version = git_tag_version(
        Path::new(repo_path),
        "waterui-backend-swift",
        "apple-backend-v*",
    );

    if let Some(version) = version {
        println!("cargo:rustc-env=WATERUI_BACKEND_SWIFT_VERSION={version}");
    } else {
        eprintln!(
            "cargo:warning=No stable version tag found for 'waterui-backend-swift'. Use `water create --dev` until a release is cut."
        );
        println!("cargo:rustc-env=WATERUI_BACKEND_SWIFT_VERSION=");
    }
}

fn emit_android_backend_version() {
    let repo_path = Path::new("../backends/android");
    let version = git_tag_version(
        Path::new(repo_path),
        "waterui-backend-android",
        "android-backend-v*",
    );

    if let Some(version) = version {
        println!("cargo:rustc-env=WATERUI_BACKEND_ANDROID_VERSION={version}");
    } else {
        eprintln!(
            "cargo:warning=No stable version tag found for 'waterui-backend-android'. Use `water create --dev` until a release is cut."
        );
        println!("cargo:rustc-env=WATERUI_BACKEND_ANDROID_VERSION=");
    }
}

fn git_tag_version(repo_path: &Path, name: &str, pattern: &str) -> Option<String> {
    let mut command = Command::new("git");
    command.args(["describe", "--tags", "--abbrev=0", "--match", pattern]);
    command.current_dir(repo_path);

    let output = match command.output() {
        Ok(output) => output,
        Err(err) => {
            eprintln!("cargo:warning=Failed to query git tags for '{name}': {err}");
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No names found") {
            eprintln!("cargo:warning=No matching tags found for '{name}'.");
        } else {
            eprintln!(
                "cargo:warning=git describe for '{}' failed: {}",
                name,
                stderr.trim()
            );
        }
        return None;
    }

    let version_tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let prefix = format!("{name}-v");

    version_tag.strip_prefix(&prefix).map_or_else(
        || {
            eprintln!("cargo:warning=Unexpected tag format '{version_tag}' for '{name}'.");
            None
        },
        |version| Some(version.to_string()),
    )
}
