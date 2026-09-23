//! Build script for waterui-cli.
//!
//! Embeds git/build metadata used to choose scaffold defaults at runtime.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use toml::Value;

const DEV_BRANCH_BUILD_KIND: &str = "dev-branch";
const RELEASE_BUILD_KIND: &str = "release";

struct ScaffoldVersions {
    waterui: String,
    waterui_ffi: String,
    hydrolysis: String,
    waterui_dew: String,
    waterui_gtk: String,
    waterui_preview: String,
    android_kotlin: String,
}

struct BackendReference {
    repository_url: String,
    commit: String,
}

struct ScaffoldMetadata {
    versions: ScaffoldVersions,
    apple_backend: BackendReference,
    android_backend: BackendReference,
}

fn main() {
    let cli_manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should always be set"),
    );
    let workspace_root = resolve_workspace_root(&cli_manifest_dir);
    let repo_root = workspace_root.as_deref().unwrap_or(&cli_manifest_dir);
    let cli_commit =
        git_output(repo_root, ["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let release_tag = git_output(repo_root, ["describe", "--exact-match", "--tags", "HEAD"]);
    let build_kind = if workspace_root.is_some() && release_tag.is_none() {
        DEV_BRANCH_BUILD_KIND
    } else {
        RELEASE_BUILD_KIND
    };
    let scaffold_metadata = resolve_scaffold_metadata(&cli_manifest_dir, workspace_root.as_deref());

    emit_scaffold_metadata(&cli_commit, build_kind, &scaffold_metadata);
    register_rerun_inputs(workspace_root.as_deref());
}

fn emit_scaffold_metadata(
    cli_commit: &str,
    build_kind: &str,
    scaffold_metadata: &ScaffoldMetadata,
) {
    println!("cargo:rustc-env=WATERUI_CLI_COMMIT={cli_commit}");
    println!("cargo:rustc-env=WATERUI_CLI_BUILD_KIND={build_kind}");
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_VERSION={}",
        scaffold_metadata.versions.waterui
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_FFI_VERSION={}",
        scaffold_metadata.versions.waterui_ffi
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_HYDROLYSIS_VERSION={}",
        scaffold_metadata.versions.hydrolysis
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_DEW_VERSION={}",
        scaffold_metadata.versions.waterui_dew
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_GTK_VERSION={}",
        scaffold_metadata.versions.waterui_gtk
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_PREVIEW_VERSION={}",
        scaffold_metadata.versions.waterui_preview
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_ANDROID_KOTLIN_VERSION={}",
        scaffold_metadata.versions.android_kotlin
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_APPLE_BACKEND_URL={}",
        scaffold_metadata.apple_backend.repository_url
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_APPLE_BACKEND_COMMIT={}",
        scaffold_metadata.apple_backend.commit
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_ANDROID_BACKEND_URL={}",
        scaffold_metadata.android_backend.repository_url
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_ANDROID_BACKEND_COMMIT={}",
        scaffold_metadata.android_backend.commit
    );
}

fn register_rerun_inputs(workspace_root: Option<&Path>) {
    println!("cargo:rerun-if-changed=Cargo.toml");
    if let Some(workspace_root) = workspace_root {
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root.join("Cargo.toml").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root.join("ffi").join("Cargo.toml").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root
                .join("backends")
                .join("gtk")
                .join("Cargo.toml")
                .display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root
                .join("components")
                .join("devtools")
                .join("preview")
                .join("runtime")
                .join("Cargo.toml")
                .display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root
                .join("backends")
                .join("hydrolysis")
                .join("Cargo.toml")
                .display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root
                .join("backends")
                .join("dew")
                .join("Cargo.toml")
                .display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root.join(".gitmodules").display()
        );
        register_git_head_rerun(workspace_root);
        register_git_head_rerun(&workspace_root.join("backends").join("apple"));
        register_git_head_rerun(&workspace_root.join("backends").join("android"));
    }
}

fn resolve_workspace_root(cli_manifest_dir: &Path) -> Option<PathBuf> {
    let root = cli_manifest_dir.parent()?.canonicalize().ok()?;
    if root.join("Cargo.toml").is_file()
        && root.join("ffi").join("Cargo.toml").is_file()
        && root
            .join("backends")
            .join("hydrolysis")
            .join("Cargo.toml")
            .is_file()
    {
        Some(root)
    } else {
        None
    }
}

fn git_output<const N: usize>(repo_root: &Path, args: [&str; N]) -> Option<String> {
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

fn resolve_scaffold_metadata(
    cli_manifest_dir: &Path,
    workspace_root: Option<&Path>,
) -> ScaffoldMetadata {
    let cli_manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));
    let scaffold_metadata = &cli_manifest["package"]["metadata"]["waterui-scaffold"];
    if let Some(workspace_root) = workspace_root {
        return ScaffoldMetadata {
            versions: ScaffoldVersions {
                waterui: manifest_package_version(&workspace_root.join("Cargo.toml")),
                waterui_ffi: manifest_package_version(
                    &workspace_root.join("ffi").join("Cargo.toml"),
                ),
                waterui_gtk: manifest_package_version(
                    &workspace_root
                        .join("backends")
                        .join("gtk")
                        .join("Cargo.toml"),
                ),
                waterui_preview: manifest_package_version(
                    &workspace_root
                        .join("components")
                        .join("devtools")
                        .join("preview")
                        .join("runtime")
                        .join("Cargo.toml"),
                ),
                android_kotlin: manifest_scaffold_string(
                    scaffold_metadata,
                    "android-kotlin-version",
                ),
                hydrolysis: manifest_package_version(
                    &workspace_root
                        .join("backends")
                        .join("hydrolysis")
                        .join("Cargo.toml"),
                ),
                waterui_dew: manifest_package_version(
                    &workspace_root
                        .join("backends")
                        .join("dew")
                        .join("Cargo.toml"),
                ),
            },
            apple_backend: workspace_backend_reference(workspace_root, "backends/apple"),
            android_backend: workspace_backend_reference(workspace_root, "backends/android"),
        };
    }

    ScaffoldMetadata {
        versions: ScaffoldVersions {
            waterui: manifest_scaffold_string(scaffold_metadata, "waterui-version"),
            waterui_ffi: manifest_scaffold_string(scaffold_metadata, "waterui-ffi-version"),
            waterui_gtk: manifest_scaffold_string(scaffold_metadata, "waterui-gtk-version"),
            waterui_preview: manifest_scaffold_string(scaffold_metadata, "waterui-preview-version"),
            android_kotlin: manifest_scaffold_string(scaffold_metadata, "android-kotlin-version"),
            hydrolysis: manifest_scaffold_string(scaffold_metadata, "hydrolysis-version"),
            waterui_dew: manifest_scaffold_string(scaffold_metadata, "waterui-dew-version"),
        },
        apple_backend: manifest_backend_reference(scaffold_metadata, "apple-backend"),
        android_backend: manifest_backend_reference(scaffold_metadata, "android-backend"),
    }
}

fn manifest_scaffold_string(scaffold_metadata: &Value, key: &str) -> String {
    scaffold_metadata[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing package.metadata.waterui-scaffold.{key}"))
        .to_string()
}

fn workspace_backend_reference(workspace_root: &Path, submodule_path: &str) -> BackendReference {
    let repository_url = git_config_value(
        workspace_root,
        &[
            "config",
            "-f",
            ".gitmodules",
            "--get",
            &format!("submodule.{submodule_path}.url"),
        ],
    )
    .unwrap_or_else(|| {
        panic!(
            "failed to resolve {submodule_path} URL from {}",
            workspace_root.display()
        )
    });
    let commit = git_output(&workspace_root.join(submodule_path), ["rev-parse", "HEAD"])
        .unwrap_or_else(|| {
            panic!(
                "failed to resolve {submodule_path} commit from {}",
                workspace_root.display()
            )
        });
    BackendReference {
        repository_url,
        commit,
    }
}

fn manifest_backend_reference(scaffold_metadata: &Value, key_prefix: &str) -> BackendReference {
    let url_key = format!("{key_prefix}-url");
    let commit_key = format!("{key_prefix}-commit");
    BackendReference {
        repository_url: scaffold_metadata
            .get(url_key.as_str())
            .unwrap_or_else(|| panic!("missing package.metadata.waterui-scaffold.{key_prefix}-url"))
            .as_str()
            .unwrap_or_else(|| panic!("invalid package.metadata.waterui-scaffold.{key_prefix}-url"))
            .to_string(),
        commit: scaffold_metadata
            .get(commit_key.as_str())
            .unwrap_or_else(|| {
                panic!("missing package.metadata.waterui-scaffold.{key_prefix}-commit")
            })
            .as_str()
            .unwrap_or_else(|| {
                panic!("invalid package.metadata.waterui-scaffold.{key_prefix}-commit")
            })
            .to_string(),
    }
}

fn manifest_package_version(path: &Path) -> String {
    manifest_value(path)["package"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("missing package.version in {}", path.display()))
        .to_string()
}

fn git_config_value(repo_root: &Path, args: &[&str]) -> Option<String> {
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
    let Some(git_dir) = git_config_value(repo_root, &["rev-parse", "--git-dir"]) else {
        panic!("failed to resolve git dir for {}", repo_root.display());
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

fn manifest_value(path: &Path) -> Value {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    toml::from_str::<Value>(&contents)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}
