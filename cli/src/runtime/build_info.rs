//! Build-time metadata embedded into the `water` CLI binary.

/// The git commit hash embedded at build time.
pub const CLI_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
/// Exact `waterui` version used when scaffolding registry-based projects.
pub const WATERUI_VERSION: &str = env!("WATERUI_CLI_WATERUI_VERSION");
/// Exact `waterui-core` version used when scaffolding registry-based projects.
pub const WATERUI_CORE_VERSION: &str = env!("WATERUI_CLI_WATERUI_CORE_VERSION");
/// Exact `waterui-testing` version used when scaffolding registry-based projects.
pub const WATERUI_TESTING_VERSION: &str = env!("WATERUI_CLI_WATERUI_TESTING_VERSION");
/// Exact `waterui-ffi` version used when scaffolding registry-based projects.
pub const WATERUI_FFI_VERSION: &str = env!("WATERUI_CLI_WATERUI_FFI_VERSION");
/// Exact `waterui-locale` version used when scaffolding registry-based projects.
pub const WATERUI_LOCALE_VERSION: &str = env!("WATERUI_CLI_WATERUI_LOCALE_VERSION");
/// Exact `hydrolysis` version used when scaffolding registry-based projects.
pub const HYDROLYSIS_VERSION: &str = env!("WATERUI_CLI_HYDROLYSIS_VERSION");
/// Exact `hydrolysis-m3` version used when scaffolding registry-based projects.
pub const HYDROLYSIS_M3_VERSION: &str = env!("WATERUI_CLI_HYDROLYSIS_M3_VERSION");
/// Exact `waterui-dew` version used when scaffolding registry-based projects.
pub const DEW_VERSION: &str = env!("WATERUI_CLI_WATERUI_DEW_VERSION");
/// Exact `waterui-gtk` version used when scaffolding registry-based projects.
pub const GTK_BACKEND_VERSION: &str = env!("WATERUI_CLI_WATERUI_GTK_VERSION");
/// Exact `waterui-browser-cef` version used when scaffolding a CEF subprocess helper.
pub const WATERUI_BROWSER_CEF_VERSION: &str = env!("WATERUI_CLI_WATERUI_BROWSER_CEF_VERSION");
/// Exact `waterui-preview` version used when scaffolding registry-based projects.
pub const PREVIEW_VERSION: &str = env!("WATERUI_CLI_WATERUI_PREVIEW_VERSION");
/// Exact `waterui-preview-protocol` version used when scaffolding registry-based projects.
pub const PREVIEW_PROTOCOL_VERSION: &str = env!("WATERUI_CLI_WATERUI_PREVIEW_PROTOCOL_VERSION");
/// Exact `waterui-mcp` version used when scaffolding registry-based projects.
pub const MCP_VERSION: &str = env!("WATERUI_CLI_WATERUI_MCP_VERSION");
/// Exact Android Kotlin compiler version required by the embedded Android backend/runtime.
pub const ANDROID_KOTLIN_VERSION: &str = env!("WATERUI_CLI_ANDROID_KOTLIN_VERSION");
/// The tag a released CLI pins each backend repository at: `v` followed by
/// this package's version, pushed to the backend repositories by the release
/// workflow before the crates are published.
pub const BACKEND_RELEASE_TAG: &str = env!("WATERUI_CLI_BACKEND_RELEASE_TAG");
/// Remote Apple backend repository URL used for scaffolding.
pub const APPLE_BACKEND_URL: &str = env!("WATERUI_CLI_APPLE_BACKEND_URL");
/// Apple backend git ref used for scaffolding: the submodule's commit in a
/// development build, `BACKEND_RELEASE_TAG` in a release build.
pub const APPLE_BACKEND_REVISION: &str = env!("WATERUI_CLI_APPLE_BACKEND_REVISION");
/// Remote Android backend repository URL used for scaffolding.
pub const ANDROID_BACKEND_URL: &str = env!("WATERUI_CLI_ANDROID_BACKEND_URL");
/// Android backend git ref used for scaffolding: the submodule's commit in a
/// development build, `BACKEND_RELEASE_TAG` in a release build.
pub const ANDROID_BACKEND_REVISION: &str = env!("WATERUI_CLI_ANDROID_BACKEND_REVISION");

const BUILD_KIND: &str = env!("WATERUI_CLI_BUILD_KIND");

/// Git repository reference embedded into the CLI binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendReference {
    /// Git remote URL for the backend repository.
    pub repository_url: &'static str,
    /// Git ref the scaffold pins — the submodule's commit in a development
    /// build, the `v<version>` release tag in a release build.
    pub revision: &'static str,
}

/// Embedded Apple backend repository reference.
pub const APPLE_BACKEND: BackendReference = BackendReference {
    repository_url: APPLE_BACKEND_URL,
    revision: APPLE_BACKEND_REVISION,
};

/// Embedded Android backend repository reference.
pub const ANDROID_BACKEND: BackendReference = BackendReference {
    repository_url: ANDROID_BACKEND_URL,
    revision: ANDROID_BACKEND_REVISION,
};

/// Embedded TUI backend repository reference.
///
/// The experimental TUI backend is not a workspace submodule, so its pin is a
/// source literal rather than a build-script value — bump `revision` when the
/// CLI starts depending on a newer `waterui-tui` API.
pub const TUI_BACKEND: BackendReference = BackendReference {
    repository_url: "https://github.com/water-rs/tui",
    revision: "4782df8a39626a9f24fd27b909d884924e2bce95",
};

/// Every crate a scaffolded project pins, as `(crate name, version)` pairs —
/// re-keyed as `<name>-version` entries in a resolved framework's scaffold
/// metadata.
pub const SCAFFOLD_PACKAGE_VERSIONS: &[(&str, &str)] = &[
    ("waterui", WATERUI_VERSION),
    ("waterui-core", WATERUI_CORE_VERSION),
    ("waterui-testing", WATERUI_TESTING_VERSION),
    ("waterui-ffi", WATERUI_FFI_VERSION),
    ("waterui-locale", WATERUI_LOCALE_VERSION),
    ("waterui-dew", DEW_VERSION),
    ("waterui-gtk", GTK_BACKEND_VERSION),
    ("waterui-browser-cef", WATERUI_BROWSER_CEF_VERSION),
    ("waterui-preview", PREVIEW_VERSION),
    ("waterui-preview-protocol", PREVIEW_PROTOCOL_VERSION),
    ("waterui-mcp", MCP_VERSION),
    ("hydrolysis", HYDROLYSIS_VERSION),
    ("hydrolysis-m3", HYDROLYSIS_M3_VERSION),
];

/// How this CLI binary was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    /// Built from a local, non-release `WaterUI` checkout and should force local-checkout dev behavior.
    DevBranch,
    /// Built from any non-dev source and should default to registry dependencies.
    Release,
}

/// Return the embedded CLI build kind.
///
/// # Panics
///
/// Panics when the build script embedded an unknown `WATERUI_CLI_BUILD_KIND`.
#[must_use]
pub fn build_kind() -> BuildKind {
    match BUILD_KIND {
        "dev-branch" => BuildKind::DevBranch,
        "release" => BuildKind::Release,
        other => panic!("invalid WATERUI_CLI_BUILD_KIND: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use cargo_metadata::MetadataCommand;
    use toml::Value;

    use super::{
        ANDROID_BACKEND, APPLE_BACKEND, BACKEND_RELEASE_TAG, BackendReference, BuildKind,
        SCAFFOLD_PACKAGE_VERSIONS, build_kind,
    };

    /// The `WaterUI` workspace root when this binary was built inside the
    /// monorepo, `None` for a packaged build — the same probe build.rs runs.
    fn workspace_root() -> Option<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .canonicalize()
            .ok()?;
        (root.join("Cargo.toml").is_file()
            && root.join("ffi").join("Cargo.toml").is_file()
            && root.join("testing").join("Cargo.toml").is_file())
        .then_some(root)
    }

    fn manifest_value(path: &Path) -> Value {
        let contents = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        toml::from_str::<Value>(&contents)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
    }

    /// The version requirement the workspace itself consumes an extracted
    /// package at, read from `[workspace.dependencies]`.
    fn workspace_dependency_requirement(workspace_manifest: &Value, name: &str) -> String {
        let dependency = &workspace_manifest["workspace"]["dependencies"][name];
        dependency
            .as_str()
            .or_else(|| dependency["version"].as_str())
            .unwrap_or_else(|| panic!("missing workspace.dependencies.{name} version"))
            .to_string()
    }

    fn git_output(repo_root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed in {}",
            repo_root.display()
        );
        String::from_utf8(output.stdout)
            .unwrap_or_else(|error| panic!("git {args:?} returned non-utf8 output: {error}"))
            .trim()
            .to_string()
    }

    #[test]
    fn backend_release_tag_is_the_cli_version_tag() {
        assert_eq!(
            BACKEND_RELEASE_TAG,
            format!("v{}", env!("CARGO_PKG_VERSION")),
            "a released CLI scaffolds each backend at the tag carrying its own version"
        );
    }

    #[test]
    fn scaffold_versions_match_their_source() {
        let cli_manifest =
            manifest_value(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
        let scaffold_metadata = &cli_manifest["package"]["metadata"]["waterui-scaffold"];
        if let Some(root) = workspace_root() {
            // A workspace build derives each version from the workspace itself:
            // the package manifest for an in-tree crate, the
            // `[workspace.dependencies]` requirement for an extracted one.
            let metadata = MetadataCommand::new()
                .current_dir(&root)
                .no_deps()
                .exec()
                .expect("cargo metadata on the workspace");
            let workspace_manifest = manifest_value(&root.join("Cargo.toml"));
            for (name, version) in SCAFFOLD_PACKAGE_VERSIONS {
                let expected = metadata
                    .workspace_packages()
                    .into_iter()
                    .find(|package| package.name.as_str() == *name)
                    .map_or_else(
                        || workspace_dependency_requirement(&workspace_manifest, name),
                        |package| package.version.to_string(),
                    );
                assert_eq!(*version, expected, "embedded version for {name}");
            }
        } else {
            // A packaged build pins every workspace crate at the CLI's own
            // release version and keeps the extracted crates' literals.
            for (name, version) in SCAFFOLD_PACKAGE_VERSIONS {
                let expected = scaffold_metadata
                    .get(format!("{name}-version"))
                    .and_then(Value::as_str)
                    .unwrap_or(env!("CARGO_PKG_VERSION"));
                assert_eq!(*version, expected, "embedded version for {name}");
            }
        }
    }

    #[test]
    fn backend_revisions_match_their_source() {
        let backends: [(BackendReference, &str); 2] = [
            (APPLE_BACKEND, "backends/apple"),
            (ANDROID_BACKEND, "backends/android"),
        ];
        match build_kind() {
            // A development build pins each backend at its live submodule
            // commit, exactly as the checkout records it.
            BuildKind::DevBranch => {
                let root = workspace_root().expect("a development build is a workspace build");
                for (reference, submodule) in backends {
                    assert_eq!(
                        reference.revision,
                        git_output(&root.join(submodule), &["rev-parse", "HEAD"]),
                        "embedded {submodule} revision"
                    );
                }
            }
            // A release build pins each backend at the tag carrying the CLI's
            // own version.
            BuildKind::Release => {
                for (reference, _) in backends {
                    assert_eq!(reference.revision, BACKEND_RELEASE_TAG);
                }
            }
        }
        // The repository URL comes from `.gitmodules` whenever the workspace
        // is there to read, which the build script's drift check enforces.
        if let Some(root) = workspace_root() {
            for (reference, submodule) in backends {
                assert_eq!(
                    reference.repository_url,
                    git_output(
                        &root,
                        &[
                            "config",
                            "-f",
                            ".gitmodules",
                            "--get",
                            &format!("submodule.{submodule}.url"),
                        ],
                    ),
                    "embedded {submodule} URL"
                );
            }
        }
    }
}
