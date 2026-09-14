//! Build script for waterui-cli.
//!
//! Embeds the git commit and the CLI-owned scaffold metadata the binary reads
//! at runtime. Framework-owned scaffold facts are not embedded in the CLI at
//! all: they live in the root manifest's `[package.metadata.waterui]` and reach
//! a scaffolded project through the published `framework.json`.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use toml::Value;

#[path = "src/android/ndk_version.rs"]
mod ndk_version;

fn main() {
    let cli_manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should always be set"),
    );
    let manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));
    let scaffold_metadata = &manifest["package"]["metadata"]["waterui-scaffold"];

    // `android-kotlin-version` is the table's only legitimate key. A
    // framework-owned key landing here would shadow the framework manifest's
    // value for every reader that does not know the difference, so the build
    // stops with the place it belongs instead.
    if let Some(table) = scaffold_metadata.as_table() {
        for key in table.keys() {
            assert!(
                key == "android-kotlin-version",
                "cli/Cargo.toml [package.metadata.waterui-scaffold].{key} is \
                 framework-owned: declare it in the root manifest's \
                 [package.metadata.waterui] table instead"
            );
        }
    }
    let kotlin_version = manifest_scaffold_string(scaffold_metadata, "android-kotlin-version");
    println!("cargo:rustc-env=WATERUI_CLI_ANDROID_KOTLIN_VERSION={kotlin_version}");

    // `ANDROID_NDK_VERSION` is a source literal in `android::ndk_version` so it
    // survives `cargo publish` — a packaged CLI still knows which NDK
    // `sdkmanager` package to install. When this build runs inside a WaterUI
    // checkout, pin the literal to the runtime Gradle declaration so the two
    // can never drift.
    let runtime_gradle = cli_manifest_dir
        .join("..")
        .join(ndk_version::RUNTIME_BUILD_GRADLE_RELATIVE_PATH);
    println!("cargo:rerun-if-changed={}", runtime_gradle.display());
    if let Ok(contents) = fs::read_to_string(&runtime_gradle) {
        let declared = ndk_version::parse_android_ndk_version_from_runtime_build_gradle(&contents)
            .unwrap_or_else(|| {
                panic!(
                    "no `ndkVersion` declaration in {}",
                    runtime_gradle.display()
                )
            });
        assert_eq!(
            declared,
            ndk_version::ANDROID_NDK_VERSION,
            "cli/src/android/ndk_version.rs ANDROID_NDK_VERSION drifted from \
             {} — update the literal with the runtime Gradle `ndkVersion`",
            runtime_gradle.display()
        );
    }

    let cli_commit =
        git(&cli_manifest_dir, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=WATERUI_CLI_COMMIT={cli_commit}");

    println!("cargo:rerun-if-changed=Cargo.toml");
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
