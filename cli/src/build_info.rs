//! Build-time metadata embedded into the `water` CLI binary.

/// The git commit hash embedded at build time.
pub const CLI_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
/// Exact `waterui` version used when scaffolding registry-based projects.
pub const WATERUI_VERSION: &str = env!("WATERUI_CLI_WATERUI_VERSION");
/// Exact `waterui-ffi` version used when scaffolding registry-based projects.
pub const WATERUI_FFI_VERSION: &str = env!("WATERUI_CLI_WATERUI_FFI_VERSION");
/// Exact `hydrolysis` version used when scaffolding registry-based projects.
pub const HYDROLYSIS_VERSION: &str = env!("WATERUI_CLI_HYDROLYSIS_VERSION");

const BUILD_KIND: &str = env!("WATERUI_CLI_BUILD_KIND");

/// How this CLI binary was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    /// Built from a local, non-release WaterUI checkout and should force local-checkout dev behavior.
    DevBranch,
    /// Built from any non-dev source and should default to registry dependencies.
    Release,
}

/// Return the embedded CLI build kind.
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
    use std::{fs, path::Path};

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
            scaffold_metadata["hydrolysis-version"]
                .as_str()
                .expect("missing package.metadata.waterui-scaffold.hydrolysis-version"),
            package_version(&workspace_root.join("backends/hydrolysis/Cargo.toml")),
        );
    }
}
