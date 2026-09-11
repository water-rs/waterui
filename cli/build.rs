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
    waterui_core: String,
    waterui_ffi: String,
    hydrolysis: String,
    hydrolysis_m3: String,
    waterui_dew: String,
    waterui_gtk: String,
    waterui_browser_cef: String,
    waterui_preview: String,
    waterui_preview_protocol: String,
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
    check_scaffold_pins(
        &cli_manifest_dir,
        workspace_root.as_deref(),
        &scaffold_metadata,
    );

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
        "cargo:rustc-env=WATERUI_CLI_WATERUI_CORE_VERSION={}",
        scaffold_metadata.versions.waterui_core
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
        "cargo:rustc-env=WATERUI_CLI_HYDROLYSIS_M3_VERSION={}",
        scaffold_metadata.versions.hydrolysis_m3
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
        "cargo:rustc-env=WATERUI_CLI_WATERUI_BROWSER_CEF_VERSION={}",
        scaffold_metadata.versions.waterui_browser_cef
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_PREVIEW_VERSION={}",
        scaffold_metadata.versions.waterui_preview
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_PREVIEW_PROTOCOL_VERSION={}",
        scaffold_metadata.versions.waterui_preview_protocol
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

/// Hold the pins in `cli/Cargo.toml` to what this workspace actually is.
///
/// A workspace build reads the real versions and the real submodule commits, so the
/// literals in the manifest only ever reach a *published* CLI — which has no workspace
/// to read and therefore has to carry them. They cannot be derived away, only kept
/// honest, and when they go stale `cargo install waterui-cli` scaffolds projects against
/// a framework one or two releases behind the tree they were scaffolded from (#548).
///
/// The two kinds of pin fail differently on purpose. A version changes only when a crate
/// is released, and a wrong one is shipped straight to users, so a mismatch stops the
/// build with the line to write. A submodule commit moves all the time during ordinary
/// backend work, where a hard failure would be in the way of the change itself, so that
/// one warns.
fn check_scaffold_pins(
    cli_manifest_dir: &Path,
    workspace_root: Option<&Path>,
    resolved: &ScaffoldMetadata,
) {
    let Some(workspace_root) = workspace_root else {
        return;
    };
    let cli_manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));
    let scaffold_metadata = &cli_manifest["package"]["metadata"]["waterui-scaffold"];

    // `waterui-testing` inherits the workspace version rather than carrying one
    // of its own, and a channel update looks its entry up by name like any
    // other, so it is held to `[workspace.package]`.
    let workspace_version =
        manifest_value(&workspace_root.join("Cargo.toml"))["workspace"]["package"]["version"]
            .as_str()
            .expect("missing workspace.package.version")
            .to_string();
    let versions = &resolved.versions;
    let stale: Vec<String> = [
        ("waterui-version", versions.waterui.as_str()),
        ("waterui-core-version", versions.waterui_core.as_str()),
        ("waterui-testing-version", workspace_version.as_str()),
        ("waterui-ffi-version", versions.waterui_ffi.as_str()),
        ("hydrolysis-version", versions.hydrolysis.as_str()),
        ("hydrolysis-m3-version", versions.hydrolysis_m3.as_str()),
        ("waterui-dew-version", versions.waterui_dew.as_str()),
        ("waterui-gtk-version", versions.waterui_gtk.as_str()),
        (
            "waterui-browser-cef-version",
            versions.waterui_browser_cef.as_str(),
        ),
        ("waterui-preview-version", versions.waterui_preview.as_str()),
        (
            "waterui-preview-protocol-version",
            versions.waterui_preview_protocol.as_str(),
        ),
    ]
    .into_iter()
    .filter_map(|(field, resolved_version)| {
        let pinned = manifest_scaffold_string(scaffold_metadata, field);
        (pinned != resolved_version)
            .then(|| format!("  {field} = \"{resolved_version}\"   (pinned {pinned})"))
    })
    .collect();
    assert!(
        stale.is_empty(),
        "cli/Cargo.toml [package.metadata.waterui-scaffold] is behind this workspace. \
         A released CLI scaffolds projects against these literals, so they travel with \
         the version bump. Set:\n{}",
        stale.join("\n")
    );

    for (field, resolved_commit) in [
        (
            "apple-backend-commit",
            resolved.apple_backend.commit.as_str(),
        ),
        (
            "android-backend-commit",
            resolved.android_backend.commit.as_str(),
        ),
    ] {
        let pinned = manifest_scaffold_string(scaffold_metadata, field);
        if pinned != resolved_commit {
            println!(
                "cargo::warning=cli/Cargo.toml `{field}` is stale: pinned {pinned}, submodule is at {resolved_commit}. A released CLI would scaffold against the pinned commit. Set `{field} = \"{resolved_commit}\"`."
            );
        }
    }
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
            workspace_root.join("core").join("Cargo.toml").display()
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
                .join("platform")
                .join("browser-cef")
                .join("Cargo.toml")
                .display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace_root
                .join("components")
                .join("devtools")
                .join("preview")
                .join("protocol")
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
        && root.join("testing").join("Cargo.toml").is_file()
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
                waterui_core: manifest_package_version(
                    &workspace_root.join("core").join("Cargo.toml"),
                ),
                waterui_ffi: manifest_package_version(
                    &workspace_root.join("ffi").join("Cargo.toml"),
                ),
                waterui_gtk: manifest_package_version(
                    &workspace_root
                        .join("backends")
                        .join("gtk")
                        .join("Cargo.toml"),
                ),
                waterui_browser_cef: manifest_package_version(
                    &workspace_root
                        .join("components")
                        .join("platform")
                        .join("browser-cef")
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
                waterui_preview_protocol: manifest_package_version(
                    &workspace_root
                        .join("components")
                        .join("devtools")
                        .join("preview")
                        .join("protocol")
                        .join("Cargo.toml"),
                ),
                android_kotlin: manifest_scaffold_string(
                    scaffold_metadata,
                    "android-kotlin-version",
                ),
                hydrolysis: workspace_dependency_requirement(
                    &workspace_root.join("Cargo.toml"),
                    "hydrolysis",
                ),
                hydrolysis_m3: workspace_dependency_requirement(
                    &workspace_root.join("Cargo.toml"),
                    "hydrolysis-m3",
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
            waterui_core: manifest_scaffold_string(scaffold_metadata, "waterui-core-version"),
            waterui_ffi: manifest_scaffold_string(scaffold_metadata, "waterui-ffi-version"),
            waterui_gtk: manifest_scaffold_string(scaffold_metadata, "waterui-gtk-version"),
            waterui_browser_cef: manifest_scaffold_string(
                scaffold_metadata,
                "waterui-browser-cef-version",
            ),
            waterui_preview: manifest_scaffold_string(scaffold_metadata, "waterui-preview-version"),
            waterui_preview_protocol: manifest_scaffold_string(
                scaffold_metadata,
                "waterui-preview-protocol-version",
            ),
            android_kotlin: manifest_scaffold_string(scaffold_metadata, "android-kotlin-version"),
            hydrolysis: manifest_scaffold_string(scaffold_metadata, "hydrolysis-version"),
            hydrolysis_m3: manifest_scaffold_string(scaffold_metadata, "hydrolysis-m3-version"),
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

/// The version requirement the workspace consumes an extracted package at.
///
/// The Hydrolysis renderer and its Material 3 theme have their own
/// repositories (#480, #481), so there is no in-tree manifest to read a
/// `package.version` from; what a scaffolded project has to agree with is the
/// requirement this workspace resolves against.
fn workspace_dependency_requirement(workspace_manifest: &Path, name: &str) -> String {
    let manifest = manifest_value(workspace_manifest);
    let dependency = &manifest["workspace"]["dependencies"][name];
    dependency
        .as_str()
        .or_else(|| dependency["version"].as_str())
        .unwrap_or_else(|| {
            panic!(
                "missing workspace.dependencies.{name} version in {}",
                workspace_manifest.display()
            )
        })
        .to_string()
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
