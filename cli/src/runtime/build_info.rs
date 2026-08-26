//! Build-time metadata embedded into the `water` CLI binary.

/// The git commit hash embedded at build time.
pub const CLI_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
/// Exact `waterui` version used when scaffolding registry-based projects.
pub const WATERUI_VERSION: &str = env!("WATERUI_CLI_WATERUI_VERSION");
/// Exact `waterui-core` version used when scaffolding registry-based projects.
pub const WATERUI_CORE_VERSION: &str = env!("WATERUI_CLI_WATERUI_CORE_VERSION");
/// Exact `waterui-ffi` version used when scaffolding registry-based projects.
pub const WATERUI_FFI_VERSION: &str = env!("WATERUI_CLI_WATERUI_FFI_VERSION");
/// Exact `hydrolysis` version used when scaffolding registry-based projects.
pub const HYDROLYSIS_VERSION: &str = env!("WATERUI_CLI_HYDROLYSIS_VERSION");
/// Exact `hydrolysis-m3` version used when scaffolding registry-based projects.
pub const HYDROLYSIS_M3_VERSION: &str = env!("WATERUI_CLI_HYDROLYSIS_M3_VERSION");
/// Exact `waterui-dew` version used when scaffolding registry-based projects.
pub const DEW_VERSION: &str = env!("WATERUI_CLI_WATERUI_DEW_VERSION");
/// Exact `waterui-gtk` version used when scaffolding registry-based projects.
pub const GTK_BACKEND_VERSION: &str = env!("WATERUI_CLI_WATERUI_GTK_VERSION");
/// Exact `waterui-preview` version used when scaffolding registry-based projects.
pub const PREVIEW_VERSION: &str = env!("WATERUI_CLI_WATERUI_PREVIEW_VERSION");
/// Exact `waterui-preview-protocol` version used when scaffolding registry-based projects.
pub const PREVIEW_PROTOCOL_VERSION: &str = env!("WATERUI_CLI_WATERUI_PREVIEW_PROTOCOL_VERSION");
/// Exact Android Kotlin compiler version required by the embedded Android backend/runtime.
pub const ANDROID_KOTLIN_VERSION: &str = env!("WATERUI_CLI_ANDROID_KOTLIN_VERSION");
/// Remote Apple backend repository URL used for release scaffolding.
pub const APPLE_BACKEND_URL: &str = env!("WATERUI_CLI_APPLE_BACKEND_URL");
/// Remote Apple backend commit used for release scaffolding.
pub const APPLE_BACKEND_COMMIT: &str = env!("WATERUI_CLI_APPLE_BACKEND_COMMIT");
/// Remote Android backend repository URL used for release scaffolding.
pub const ANDROID_BACKEND_URL: &str = env!("WATERUI_CLI_ANDROID_BACKEND_URL");
/// Remote Android backend commit used for release scaffolding.
pub const ANDROID_BACKEND_COMMIT: &str = env!("WATERUI_CLI_ANDROID_BACKEND_COMMIT");

const BUILD_KIND: &str = env!("WATERUI_CLI_BUILD_KIND");

/// Git repository reference embedded into the CLI binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendReference {
    /// Git remote URL for the backend repository.
    pub repository_url: &'static str,
    /// Exact commit pinned into the CLI binary at build time.
    pub commit: &'static str,
}

/// Embedded Apple backend repository reference.
pub const APPLE_BACKEND: BackendReference = BackendReference {
    repository_url: APPLE_BACKEND_URL,
    commit: APPLE_BACKEND_COMMIT,
};

/// Embedded Android backend repository reference.
pub const ANDROID_BACKEND: BackendReference = BackendReference {
    repository_url: ANDROID_BACKEND_URL,
    commit: ANDROID_BACKEND_COMMIT,
};

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
    use std::{fs, path::Path, process::Command};

    use toml::Value;

    fn manifest_value(path: &Path) -> Value {
        let contents = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        toml::from_str::<Value>(&contents)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
    }

    fn package_version(path: &Path) -> String {
        manifest_value(path)["package"]["version"]
            .as_str()
            .unwrap_or_else(|| panic!("missing package.version in {}", path.display()))
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

    fn manifest_backend_field<'a>(cli_manifest: &'a Value, field: &str) -> &'a str {
        cli_manifest["package"]["metadata"]["waterui-scaffold"][field]
            .as_str()
            .unwrap_or_else(|| panic!("missing package.metadata.waterui-scaffold.{field}"))
    }

    fn manifest_scaffold_field<'a>(cli_manifest: &'a Value, field: &str) -> &'a str {
        cli_manifest["package"]["metadata"]["waterui-scaffold"][field]
            .as_str()
            .unwrap_or_else(|| panic!("missing package.metadata.waterui-scaffold.{field}"))
    }

    #[test]
    fn fallback_release_versions_match_workspace_versions() {
        let cli_manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = cli_manifest_dir
            .parent()
            .expect("cli crate should live under workspace root");
        let cli_manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));
        let scaffold_metadata = &cli_manifest["package"]["metadata"]["waterui-scaffold"];

        assert_eq!(
            scaffold_metadata["waterui-version"]
                .as_str()
                .expect("missing package.metadata.waterui-scaffold.waterui-version"),
            package_version(&workspace_root.join("Cargo.toml")),
        );
        assert_eq!(
            scaffold_metadata["waterui-ffi-version"]
                .as_str()
                .expect("missing package.metadata.waterui-scaffold.waterui-ffi-version"),
            package_version(&workspace_root.join("ffi/Cargo.toml")),
        );
        assert_eq!(
            manifest_scaffold_field(&cli_manifest, "waterui-core-version"),
            package_version(&workspace_root.join("core/Cargo.toml")),
        );
        assert_eq!(
            scaffold_metadata["hydrolysis-version"]
                .as_str()
                .expect("missing package.metadata.waterui-scaffold.hydrolysis-version"),
            package_version(&workspace_root.join("backends/hydrolysis/Cargo.toml")),
        );
        assert_eq!(
            manifest_scaffold_field(&cli_manifest, "hydrolysis-m3-version"),
            package_version(&workspace_root.join("backends/hydrolysis_m3/Cargo.toml")),
        );
        assert_eq!(
            manifest_scaffold_field(&cli_manifest, "waterui-dew-version"),
            package_version(&workspace_root.join("backends/dew/Cargo.toml")),
        );
        assert_eq!(
            manifest_scaffold_field(&cli_manifest, "waterui-gtk-version"),
            package_version(&workspace_root.join("backends/gtk/Cargo.toml")),
        );
        assert_eq!(
            manifest_scaffold_field(&cli_manifest, "waterui-preview-version"),
            package_version(&workspace_root.join("components/devtools/preview/runtime/Cargo.toml"))
        );
        assert_eq!(
            manifest_scaffold_field(&cli_manifest, "waterui-preview-protocol-version"),
            package_version(
                &workspace_root.join("components/devtools/preview/protocol/Cargo.toml")
            )
        );
        assert_eq!(
            manifest_scaffold_field(&cli_manifest, "android-kotlin-version"),
            super::ANDROID_KOTLIN_VERSION,
        );
    }

    #[test]
    fn fallback_release_backend_refs_match_workspace_backend_refs() {
        let cli_manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = cli_manifest_dir
            .parent()
            .expect("cli crate should live under workspace root");
        let cli_manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));

        // A workspace build reads the submodules directly, so these literals only ever
        // reach a *published* CLI, which has no submodules and must carry the value. They
        // cannot be derived away — only kept honest. When one goes stale, a released CLI
        // scaffolds projects against a backend behind the one this repository builds
        // against, so say plainly what to write rather than printing two bare hashes.
        for (backend, submodule) in [("apple", "backends/apple"), ("android", "backends/android")] {
            let commit_field = format!("{backend}-backend-commit");
            let pinned = manifest_backend_field(&cli_manifest, &commit_field);
            let actual = git_output(&workspace_root.join(submodule), &["rev-parse", "HEAD"]);
            assert_eq!(
                pinned, actual,
                "cli/Cargo.toml `{commit_field}` is stale. \
                 Set `{commit_field} = \"{actual}\"` under [package.metadata.waterui-scaffold]; \
                 a released CLI scaffolds against the pinned commit, not the submodule."
            );

            let url_field = format!("{backend}-backend-url");
            assert_eq!(
                manifest_backend_field(&cli_manifest, &url_field),
                git_output(
                    workspace_root,
                    &[
                        "config",
                        "-f",
                        ".gitmodules",
                        "--get",
                        &format!("submodule.{submodule}.url"),
                    ],
                ),
                "cli/Cargo.toml `{url_field}` disagrees with .gitmodules",
            );
        }
    }
}
