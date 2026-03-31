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
    let scaffold_versions = resolve_scaffold_versions(&cli_manifest_dir, workspace_root.as_deref());

    println!("cargo:rustc-env=WATERUI_CLI_COMMIT={cli_commit}");
    println!("cargo:rustc-env=WATERUI_CLI_BUILD_KIND={build_kind}");
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_VERSION={}",
        scaffold_versions.waterui
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_WATERUI_FFI_VERSION={}",
        scaffold_versions.waterui_ffi
    );
    println!(
        "cargo:rustc-env=WATERUI_CLI_HYDROLYSIS_VERSION={}",
        scaffold_versions.hydrolysis
    );

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
                .join("hydrolysis")
                .join("Cargo.toml")
                .display()
        );

        let git_dir = workspace_root.join(".git");
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join("refs").join("heads").display()
        );
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

fn resolve_scaffold_versions(
    cli_manifest_dir: &Path,
    workspace_root: Option<&Path>,
) -> ScaffoldVersions {
    if let Some(workspace_root) = workspace_root {
        return ScaffoldVersions {
            waterui: manifest_package_version(&workspace_root.join("Cargo.toml")),
            waterui_ffi: manifest_package_version(&workspace_root.join("ffi").join("Cargo.toml")),
            hydrolysis: manifest_package_version(
                &workspace_root
                    .join("backends")
                    .join("hydrolysis")
                    .join("Cargo.toml"),
            ),
        };
    }

    let cli_manifest = manifest_value(&cli_manifest_dir.join("Cargo.toml"));
    let scaffold_metadata = &cli_manifest["package"]["metadata"]["waterui-scaffold"];
    ScaffoldVersions {
        waterui: scaffold_metadata["waterui-version"]
            .as_str()
            .expect("missing package.metadata.waterui-scaffold.waterui-version")
            .to_string(),
        waterui_ffi: scaffold_metadata["waterui-ffi-version"]
            .as_str()
            .expect("missing package.metadata.waterui-scaffold.waterui-ffi-version")
            .to_string(),
        hydrolysis: scaffold_metadata["hydrolysis-version"]
            .as_str()
            .expect("missing package.metadata.waterui-scaffold.hydrolysis-version")
            .to_string(),
    }
}

fn manifest_package_version(path: &Path) -> String {
    manifest_value(path)["package"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("missing package.version in {}", path.display()))
        .to_string()
}

fn manifest_value(path: &Path) -> Value {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    toml::from_str::<Value>(&contents)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}
