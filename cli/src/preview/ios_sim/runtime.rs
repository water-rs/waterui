//! Discovery of installed Xcode iOS Simulator runtimes for headless preview.

use std::path::{Path, PathBuf};

use eyre::{Context as _, Result, bail, eyre};
use serde::Deserialize;
use serde_json::from_str;
use smol::fs;

use crate::toolchain::Host;

/// Metadata for an available arm64 Xcode iOS Simulator runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosSimRuntime {
    /// Runtime version string (e.g. `"27.0"`).
    pub version: String,
    /// Runtime build identifier (e.g. `"24A434"`).
    pub build_version: String,
    /// Path to `.../Contents/Resources/RuntimeRoot`.
    pub runtime_root: PathBuf,
}

#[derive(Debug, Deserialize)]
struct SimctlRuntimesOutput {
    runtimes: Vec<SimctlRuntimeEntry>,
}

#[derive(Debug, Deserialize)]
struct SimctlRuntimeEntry {
    #[serde(rename = "isAvailable")]
    is_available: bool,
    platform: Option<String>,
    version: String,
    #[serde(rename = "buildversion")]
    build_version: String,
    #[serde(rename = "runtimeRoot")]
    runtime_root: Option<String>,
    #[serde(rename = "supportedArchitectures", default)]
    supported_architectures: Vec<String>,
}

/// Discover the newest installed arm64 iOS Simulator runtime via `xcrun simctl`.
///
/// # Errors
///
/// Returns an error if `xcrun simctl list runtimes --json` fails, produces
/// invalid JSON, or reports no available arm64 iOS Simulator runtime with an
/// existing `RuntimeRoot` directory.
pub async fn discover_ios_sim_runtime(host: &Host) -> Result<IosSimRuntime> {
    let output = host
        .run("xcrun", ["simctl", "list", "runtimes", "--json"])
        .await
        .wrap_err(
            "Failed to query installed iOS Simulator runtimes via `xcrun simctl list runtimes --json`",
        )?;
    let selected = select_ios_sim_runtime(&output)?;
    let exists = fs::metadata(&selected.runtime_root)
        .await
        .is_ok_and(|metadata| metadata.is_dir());
    if !exists {
        bail!(
            "Installed iOS Simulator runtime {} ({}) points to missing RuntimeRoot {}",
            selected.version,
            selected.build_version,
            selected.runtime_root.display()
        );
    }
    Ok(selected)
}

/// Select the newest available arm64 iOS runtime from `xcrun simctl list runtimes --json`.
pub fn select_ios_sim_runtime(json: &str) -> Result<IosSimRuntime> {
    let parsed: SimctlRuntimesOutput =
        from_str(json).wrap_err("Failed to parse `xcrun simctl list runtimes --json` output")?;
    let mut candidates = parsed
        .runtimes
        .into_iter()
        .filter(|entry| {
            entry.is_available
                && entry
                    .platform
                    .as_deref()
                    .is_some_and(|platform| platform.eq_ignore_ascii_case("iOS"))
                && entry
                    .supported_architectures
                    .iter()
                    .any(|arch| arch == "arm64")
        })
        .filter_map(|entry| {
            let root = entry.runtime_root?;
            Some(IosSimRuntime {
                version: entry.version,
                build_version: entry.build_version,
                runtime_root: PathBuf::from(root),
            })
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        parse_version_tuple(&right.version)
            .cmp(&parse_version_tuple(&left.version))
            .then_with(|| right.build_version.cmp(&left.build_version))
    });

    candidates.into_iter().next().ok_or_else(|| {
        eyre!(
            "No available arm64 iOS Simulator runtime is installed. \
             Install an iOS Simulator runtime in Xcode (`xcodebuild -downloadPlatform iOS`)."
        )
    })
}

fn parse_version_tuple(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0))
        .collect()
}

/// Query the Xcode SDK path for `sdk` (`macosx` or `iphonesimulator`).
pub async fn sdk_path(host: &Host, sdk: &str) -> Result<PathBuf> {
    let raw = host
        .run("xcrun", ["--sdk", sdk, "--show-sdk-path"])
        .await
        .wrap_err_with(|| format!("Failed to resolve Xcode SDK path for `{sdk}`"))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("`xcrun --sdk {sdk} --show-sdk-path` returned an empty path");
    }
    let path = PathBuf::from(trimmed);
    if !Path::new(&path).is_dir() {
        bail!(
            "Resolved `{sdk}` SDK path does not exist: {}",
            path.display()
        );
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::select_ios_sim_runtime;

    #[test]
    fn selects_newest_available_arm64_ios_runtime() {
        let json = r#"{
            "runtimes": [
                {
                    "isAvailable": true,
                    "platform": "watchOS",
                    "version": "27.0",
                    "buildversion": "24R362",
                    "supportedArchitectures": ["arm64"],
                    "runtimeRoot": "/watchos/RuntimeRoot"
                },
                {
                    "isAvailable": false,
                    "platform": "iOS",
                    "version": "28.0",
                    "buildversion": "25A100",
                    "supportedArchitectures": ["arm64"],
                    "runtimeRoot": "/ios28/RuntimeRoot"
                },
                {
                    "isAvailable": true,
                    "platform": "iOS",
                    "version": "26.2",
                    "buildversion": "23C55",
                    "supportedArchitectures": ["arm64"],
                    "runtimeRoot": "/ios26/RuntimeRoot"
                },
                {
                    "isAvailable": true,
                    "platform": "iOS",
                    "version": "27.0",
                    "buildversion": "24A434",
                    "supportedArchitectures": ["arm64"],
                    "runtimeRoot": "/ios27/RuntimeRoot"
                }
            ]
        }"#;

        let runtime = select_ios_sim_runtime(json).expect("select runtime");
        assert_eq!(runtime.version, "27.0");
        assert_eq!(runtime.build_version, "24A434");
        assert_eq!(runtime.runtime_root.to_string_lossy(), "/ios27/RuntimeRoot");
    }

    #[test]
    fn errors_when_no_arm64_ios_runtime_exists() {
        let json = r#"{
            "runtimes": [
                {
                    "isAvailable": true,
                    "platform": "iOS",
                    "version": "18.0",
                    "buildversion": "22A1",
                    "supportedArchitectures": ["x86_64"],
                    "runtimeRoot": "/ios-intel/RuntimeRoot"
                }
            ]
        }"#;

        let err = select_ios_sim_runtime(json).expect_err("must reject non-arm64 runtime");
        assert!(err.to_string().contains("arm64 iOS Simulator runtime"));
    }
}
