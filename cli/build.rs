//! Build script for waterui-cli.
//!
//! Embeds git/build metadata used to choose scaffold defaults at runtime.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use cargo_metadata::MetadataCommand;
use toml::Value;

const DEV_BRANCH_BUILD_KIND: &str = "dev-branch";
const RELEASE_BUILD_KIND: &str = "release";

/// The workspace crates a scaffolded project pins, with the `cargo:rustc-env`
/// name carrying each one's resolved version.
///
/// Their version is the workspace's own: a build inside this repository reads
/// each package's manifest, while a packaged build has no manifest to read and
/// substitutes the CLI's own package version — the release wave publishes the
/// workspace crates together with it.
const WORKSPACE_PACKAGES: &[(&str, &str)] = &[
    ("waterui", "WATERUI_CLI_WATERUI_VERSION"),
    ("waterui-core", "WATERUI_CLI_WATERUI_CORE_VERSION"),
    ("waterui-testing", "WATERUI_CLI_WATERUI_TESTING_VERSION"),
    ("waterui-ffi", "WATERUI_CLI_WATERUI_FFI_VERSION"),
    ("waterui-gtk", "WATERUI_CLI_WATERUI_GTK_VERSION"),
    (
        "waterui-browser-cef",
        "WATERUI_CLI_WATERUI_BROWSER_CEF_VERSION",
    ),
    ("waterui-preview", "WATERUI_CLI_WATERUI_PREVIEW_VERSION"),
    (
        "waterui-preview-protocol",
        "WATERUI_CLI_WATERUI_PREVIEW_PROTOCOL_VERSION",
    ),
];

/// The extracted crates a scaffolded project pins, with the `cargo:rustc-env`
/// name carrying each one's resolved version.
///
/// The Hydrolysis renderer, its Material 3 theme, and the Dew renderer are
/// released from their own repositories (#480, #481, #614), so no manifest in
/// this workspace carries a version for them — `cli/Cargo.toml`'s scaffold
/// metadata holds the requirement this workspace consumes them at.
const EXTERNAL_PACKAGES: &[(&str, &str)] = &[
    ("hydrolysis", "WATERUI_CLI_HYDROLYSIS_VERSION"),
    ("hydrolysis-m3", "WATERUI_CLI_HYDROLYSIS_M3_VERSION"),
    ("waterui-dew", "WATERUI_CLI_WATERUI_DEW_VERSION"),
];

/// The submodule each native backend repository lives at; the backend name is
/// the directory's basename.
const BACKEND_SUBMODULES: &[&str] = &["backends/apple", "backends/android"];

fn backend_name(submodule_path: &str) -> String {
    submodule_path
        .rsplit('/')
        .next()
        .expect("a submodule path has a basename")
        .to_string()
}

struct BackendReference {
    repository_url: String,
    /// The git ref a scaffolded project pins: the submodule's checked-out
    /// commit in a development build, the `v<cli version>` tag the release
    /// workflow pushes to the backend repository in a release build.
    revision: String,
}

struct ScaffoldMetadata {
    /// Crate name -> resolved version for every package the scaffold pins.
    versions: BTreeMap<String, String>,
    android_kotlin: String,
    /// Backend name -> repository reference.
    backends: BTreeMap<String, BackendReference>,
}

/// The `WaterUI` monorepo this CLI is being built inside, if it is.
struct Workspace {
    root: PathBuf,
    metadata: cargo_metadata::Metadata,
}

impl Workspace {
    /// A packaged `waterui-cli` has no sibling crates to read, so the probe
    /// looks for the workspace root above `cli/` and recognizes it by the
    /// members the scaffold contract needs.
    fn detect(cli_manifest_dir: &Path) -> Option<Self> {
        let root = cli_manifest_dir.parent()?.canonicalize().ok()?;
        if !root.join("Cargo.toml").is_file()
            || !root.join("ffi").join("Cargo.toml").is_file()
            || !root.join("testing").join("Cargo.toml").is_file()
        {
            return None;
        }
        let metadata = MetadataCommand::new()
            .current_dir(&root)
            .no_deps()
            .exec()
            .unwrap_or_else(|error| panic!("cargo metadata failed in {}: {error}", root.display()));
        Some(Self { root, metadata })
    }

    fn package(&self, name: &str) -> &cargo_metadata::Package {
        self.metadata
            .workspace_packages()
            .into_iter()
            .find(|package| package.name.as_str() == name)
            .unwrap_or_else(|| panic!("workspace has no package named {name}"))
    }

    /// The version requirement the workspace consumes an extracted package at.
    ///
    /// A scaffolded project has to agree with the requirement this workspace
    /// resolves against, which `[workspace.dependencies]` records.
    fn dependency_requirement(&self, name: &str) -> String {
        let manifest_path = self.root.join("Cargo.toml");
        let manifest = manifest_value(&manifest_path);
        let dependency = &manifest["workspace"]["dependencies"][name];
        dependency
            .as_str()
            .or_else(|| dependency["version"].as_str())
            .unwrap_or_else(|| {
                panic!(
                    "missing workspace.dependencies.{name} version in {}",
                    manifest_path.display()
                )
            })
            .to_string()
    }
}

fn main() {
    let cli_manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should always be set"),
    );
    let workspace = Workspace::detect(&cli_manifest_dir);
    let repo_root = workspace
        .as_ref()
        .map_or(cli_manifest_dir.as_path(), |workspace| {
            workspace.root.as_path()
        });
    let cli_commit =
        git(repo_root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let release_tag = git(repo_root, &["describe", "--exact-match", "--tags", "HEAD"]);
    let build_kind = if workspace.is_some() && release_tag.is_none() {
        DEV_BRANCH_BUILD_KIND
    } else {
        RELEASE_BUILD_KIND
    };
    let cli_version =
        env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION should always be set");
    let scaffold_metadata = resolve_scaffold_metadata(
        &cli_manifest_dir,
        workspace.as_ref(),
        &cli_version,
        build_kind == RELEASE_BUILD_KIND,
    );
    check_scaffold_metadata(&cli_manifest_dir, workspace.as_ref(), &scaffold_metadata);

    emit_scaffold_metadata(&cli_commit, build_kind, &cli_version, &scaffold_metadata);
    register_rerun_inputs(workspace.as_ref());
}

fn emit_scaffold_metadata(
    cli_commit: &str,
    build_kind: &str,
    cli_version: &str,
    scaffold_metadata: &ScaffoldMetadata,
) {
    println!("cargo:rustc-env=WATERUI_CLI_COMMIT={cli_commit}");
    println!("cargo:rustc-env=WATERUI_CLI_BUILD_KIND={build_kind}");
    println!("cargo:rustc-env=WATERUI_CLI_BACKEND_RELEASE_TAG=v{cli_version}");
    for &(name, variable) in WORKSPACE_PACKAGES.iter().chain(EXTERNAL_PACKAGES.iter()) {
        println!(
            "cargo:rustc-env={variable}={}",
            scaffold_metadata.versions[name]
        );
    }
    println!(
        "cargo:rustc-env=WATERUI_CLI_ANDROID_KOTLIN_VERSION={}",
        scaffold_metadata.android_kotlin
    );
    for &submodule in BACKEND_SUBMODULES {
        let name = backend_name(submodule);
        let reference = &scaffold_metadata.backends[&name];
        let prefix = format!(
            "WATERUI_CLI_{}_BACKEND",
            name.to_uppercase().replace('-', "_")
        );
        println!("cargo:rustc-env={prefix}_URL={}", reference.repository_url);
        println!("cargo:rustc-env={prefix}_REVISION={}", reference.revision);
    }
}

/// Hold the remaining `cli/Cargo.toml` scaffold pins to what this workspace
/// actually is.
///
/// Everything a workspace manifest can supply is derived at build time, so the
/// table only carries what no manifest knows: the requirements for crates
/// released from their own repositories, the Android Kotlin toolchain version,
/// and the backend repository coordinates. A released CLI scaffolds projects
/// against those literals, so a drifted one stops the build with the line to
/// write — and a key the table no longer owns stops it too, since a stale
/// literal would shadow the derived value for every reader that does not know
/// the difference.
fn check_scaffold_metadata(
    cli_manifest_dir: &Path,
    workspace: Option<&Workspace>,
    resolved: &ScaffoldMetadata,
) {
    if workspace.is_none() {
        return;
    }
    let cli_manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));
    let scaffold_metadata = &cli_manifest["package"]["metadata"]["waterui-scaffold"];

    let mut stale: Vec<String> = Vec::new();
    let mut expect = |field: String, resolved_value: &str| {
        let pinned = manifest_scaffold_string(scaffold_metadata, &field);
        if pinned != resolved_value {
            stale.push(format!(
                "  {field} = \"{resolved_value}\"   (pinned {pinned})"
            ));
        }
    };
    for &(name, _) in EXTERNAL_PACKAGES {
        expect(format!("{name}-version"), &resolved.versions[name]);
    }
    for &submodule in BACKEND_SUBMODULES {
        let name = backend_name(submodule);
        expect(
            format!("{name}-backend-url"),
            &resolved.backends[&name].repository_url,
        );
    }

    let expected: BTreeSet<String> = EXTERNAL_PACKAGES
        .iter()
        .map(|&(name, _)| format!("{name}-version"))
        .chain(
            BACKEND_SUBMODULES
                .iter()
                .map(|submodule| format!("{}-backend-url", backend_name(submodule))),
        )
        .chain(["android-kotlin-version".to_string()])
        .collect();
    if let Some(table) = scaffold_metadata.as_table() {
        for key in table.keys() {
            if !expected.contains(key.as_str()) {
                stale.push(format!(
                    "  remove `{key}` — the value is derived at build time now"
                ));
            }
        }
    }

    assert!(
        stale.is_empty(),
        "cli/Cargo.toml [package.metadata.waterui-scaffold] is behind this workspace. \
         A released CLI scaffolds projects against these literals, so they travel with \
         the workspace value. Set:\n{}",
        stale.join("\n")
    );
}

fn register_rerun_inputs(workspace: Option<&Workspace>) {
    println!("cargo:rerun-if-changed=Cargo.toml");
    if let Some(workspace) = workspace {
        for &(name, _) in WORKSPACE_PACKAGES {
            println!(
                "cargo:rerun-if-changed={}",
                workspace.package(name).manifest_path
            );
        }
        println!(
            "cargo:rerun-if-changed={}",
            workspace.root.join("Cargo.toml").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace.root.join(".gitmodules").display()
        );
        register_git_head_rerun(&workspace.root);
        for &submodule in BACKEND_SUBMODULES {
            register_git_head_rerun(&workspace.root.join(submodule));
        }
    }
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

fn resolve_scaffold_metadata(
    cli_manifest_dir: &Path,
    workspace: Option<&Workspace>,
    cli_version: &str,
    release: bool,
) -> ScaffoldMetadata {
    let cli_manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));
    let scaffold_metadata = &cli_manifest["package"]["metadata"]["waterui-scaffold"];
    let mut versions = BTreeMap::new();
    let mut backends = BTreeMap::new();
    if let Some(workspace) = workspace {
        for &(name, _) in WORKSPACE_PACKAGES {
            versions.insert(
                name.to_string(),
                workspace.package(name).version.to_string(),
            );
        }
        for &(name, _) in EXTERNAL_PACKAGES {
            versions.insert(name.to_string(), workspace.dependency_requirement(name));
        }
        for &submodule in BACKEND_SUBMODULES {
            let name = backend_name(submodule);
            backends.insert(
                name,
                BackendReference {
                    repository_url: submodule_url(&workspace.root, submodule),
                    // A release build (a checkout of the CLI's own tag) pins the
                    // `v<version>` tag the workflow pushed to the backend
                    // repository; a development build pins the live submodule
                    // commit.
                    revision: if release {
                        format!("v{cli_version}")
                    } else {
                        submodule_head(&workspace.root, submodule)
                    },
                },
            );
        }
    } else {
        // A packaged build has no workspace to read: a scaffolded project pins
        // every workspace crate at the CLI's own version, each extracted crate
        // at the requirement the manifest still carries, and each backend at
        // the `v<version>` tag the release workflow pushed before publish.
        for &(name, _) in WORKSPACE_PACKAGES {
            versions.insert(name.to_string(), cli_version.to_string());
        }
        for &(name, _) in EXTERNAL_PACKAGES {
            versions.insert(
                name.to_string(),
                manifest_scaffold_string(scaffold_metadata, &format!("{name}-version")),
            );
        }
        for &submodule in BACKEND_SUBMODULES {
            let name = backend_name(submodule);
            backends.insert(
                name.clone(),
                BackendReference {
                    repository_url: manifest_scaffold_string(
                        scaffold_metadata,
                        &format!("{name}-backend-url"),
                    ),
                    revision: format!("v{cli_version}"),
                },
            );
        }
    }
    ScaffoldMetadata {
        versions,
        android_kotlin: manifest_scaffold_string(scaffold_metadata, "android-kotlin-version"),
        backends,
    }
}

fn manifest_scaffold_string(scaffold_metadata: &Value, key: &str) -> String {
    scaffold_metadata[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing package.metadata.waterui-scaffold.{key}"))
        .to_string()
}

fn submodule_url(workspace_root: &Path, submodule_path: &str) -> String {
    git(
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
    })
}

fn submodule_head(workspace_root: &Path, submodule_path: &str) -> String {
    git(&workspace_root.join(submodule_path), &["rev-parse", "HEAD"]).unwrap_or_else(|| {
        panic!(
            "failed to resolve {submodule_path} commit from {}",
            workspace_root.display()
        )
    })
}

fn register_git_head_rerun(repo_root: &Path) {
    let Some(git_dir) = git(repo_root, &["rev-parse", "--git-dir"]) else {
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
