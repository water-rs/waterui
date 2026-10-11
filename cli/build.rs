//! Build script for waterui-cli.
//!
//! Embeds the git commit and the CLI-owned scaffold metadata the binary reads
//! at runtime. Framework-owned scaffold facts are not embedded in the CLI at
//! all: they live in the framework root manifest's `[package.metadata.waterui]`
//! and reach a scaffolded project through the published `framework.json`.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use toml::Value;

fn main() {
    let cli_manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should always be set"),
    );
    // `cargo package` normalizes the manifest inside the tarball — path
    // dependencies are rewritten to their registry `version`, which is the
    // only shape crates.io accepts — and keeps the authored file verbatim at
    // `Cargo.toml.orig`. The checks below (workspace source, waterui-scaffold
    // keys) are about what the author wrote, so the packaged build must read
    // the original manifest or it panics on the normalized one.
    let authored_manifest = cli_manifest_dir.join("Cargo.toml.orig");
    let manifest_path = if authored_manifest.exists() {
        authored_manifest
    } else {
        cli_manifest_dir.join("Cargo.toml")
    };
    let manifest = manifest_value(&manifest_path);
    let scaffold_metadata = &manifest["package"]["metadata"]["waterui-scaffold"];

    // `android-kotlin-version` and `android-jdk-version` are the table's only
    // legitimate keys. A framework-owned key landing here would shadow the
    // framework manifest's value for every reader that does not know the
    // difference, so the build stops with the place it belongs instead.
    if let Some(table) = scaffold_metadata.as_table() {
        for key in table.keys() {
            assert!(
                key == "android-kotlin-version" || key == "android-jdk-version",
                "Cargo.toml [package.metadata.waterui-scaffold].{key} is \
                 framework-owned: declare it in the water-rs/waterui root \
                 manifest's [package.metadata.waterui] table instead"
            );
        }
    }
    let kotlin_version = manifest_scaffold_string(scaffold_metadata, "android-kotlin-version");
    println!("cargo:rustc-env=WATERUI_CLI_ANDROID_KOTLIN_VERSION={kotlin_version}");
    let jdk_version = manifest_scaffold_string(scaffold_metadata, "android-jdk-version");
    println!("cargo:rustc-env=WATERUI_CLI_ANDROID_JDK_VERSION={jdk_version}");

    println!(
        "cargo:rustc-env=WATERUI_FRAMEWORK_REPOSITORY={}",
        framework_repository(&manifest)
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_DIST_TARGETS={}",
        dist_targets(&manifest).join(" ")
    );

    let cli_commit =
        git(&cli_manifest_dir, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=WATERUI_CLI_COMMIT={cli_commit}");

    println!("cargo:rerun-if-changed={}", manifest_path.display());
    register_git_head_rerun(&cli_manifest_dir);
}

fn git(repo_root: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .filter(|output| !output.is_empty())
}

fn register_git_head_rerun(repo_root: &Path) {
    // A packaged build has no `.git` to watch.
    let Some(git_dir) = git(repo_root, &["rev-parse", "--git-dir"]) else {
        return;
    };
    let git_dir_path = PathBuf::from(git_dir);
    let git_dir_path = if git_dir_path.is_absolute() {
        git_dir_path
    } else {
        repo_root.join(git_dir_path)
    };
    println!(
        "cargo:rerun-if-changed={}",
        git_dir_path.join("HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        git_dir_path.join("refs").display()
    );
}

/// The repository this crate lives in — where certified manifests, releases,
/// and `dev` revisions live. Every `waterui-*` dependency must be
/// workspace-inherited (`workspace = true`): they resolve by path to this
/// workspace's own members, so a registry or git copy of one would exist
/// twice beside the shared types its siblings link.
fn framework_repository(manifest: &Value) -> String {
    let mut seen = false;
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(dependencies) = manifest[table].as_table() else {
            continue;
        };
        for (name, dependency) in dependencies {
            if !name.starts_with("waterui-") {
                continue;
            }
            seen = true;
            assert!(
                dependency.get("workspace").and_then(Value::as_bool) == Some(true),
                "Cargo.toml [{table}] {name} must be a workspace dependency — the CLI lives in \
                 the framework repository, and only the workspace's path resolution keeps every \
                 waterui-* crate on one source"
            );
        }
    }
    assert!(seen, "Cargo.toml declares no waterui-* dependency");
    "https://github.com/water-rs/waterui".to_string()
}

/// The host triples `dist` ships the CLI for — `[package.metadata.dist]
/// targets`, the list `dist` itself builds from.
fn dist_targets(manifest: &Value) -> Vec<&str> {
    let targets = manifest["package"]["metadata"]["dist"]["targets"]
        .as_array()
        .expect("Cargo.toml [package.metadata.dist] must declare `targets`");
    assert!(
        !targets.is_empty(),
        "Cargo.toml [package.metadata.dist] targets must not be empty"
    );
    targets
        .iter()
        .map(|target| {
            target.as_str().unwrap_or_else(|| {
                panic!("Cargo.toml [package.metadata.dist] target {target} is not a string")
            })
        })
        .collect()
}

fn manifest_scaffold_string(scaffold_metadata: &Value, key: &str) -> String {
    scaffold_metadata[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing package.metadata.waterui-scaffold.{key}"))
        .to_string()
}

fn manifest_value(path: &Path) -> Value {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    toml::from_str::<Value>(&contents)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}
