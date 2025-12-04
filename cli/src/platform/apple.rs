use std::{
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::eyre::{Result, bail};
use tracing::{debug, info};
use which::which;

use crate::{
    backend::{
        Backend,
        apple::{
            Apple, derived_data_dir, disable_code_signing, ensure_macos_host,
            prepare_derived_data_dir, resolve_xcode_project, run_xcodebuild_with_progress,
            xcodebuild_base,
        },
    },
    build::{self, BuildOptions},
    doctor::{AnyToolchainIssue, ToolchainIssue},
    platform::Platform,
    project::{Project, Swift},
};

/// Target describing how Xcode should build artifacts for Apple platforms.
#[derive(Clone, Debug)]
pub enum AppleTarget {
    Macos,
    IosDevice,
    Simulator(AppleSimulatorTarget),
}

/// Simulator-specific target metadata captured at construction time.
#[derive(Clone, Debug)]
pub struct AppleSimulatorTarget {
    pub kind: AppleSimulatorKind,
    pub device_identifier: String,
    pub reference_is_udid: bool,
}

impl AppleSimulatorTarget {
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.device_identifier
    }

    #[must_use]
    pub const fn destination_selector(&self) -> &'static str {
        if self.reference_is_udid { "id" } else { "name" }
    }
}

/// High-level simulator families supported by the Apple backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppleSimulatorKind {
    Ios,
    Ipados,
    Watchos,
    Tvos,
    Visionos,
}

#[derive(Clone, Debug)]
pub struct ApplePlatform {
    backend: Apple,
    swift: Swift,
    target: AppleTarget,
}

impl ApplePlatform {
    #[must_use]
    pub const fn new(swift: Swift, target: AppleTarget) -> Self {
        Self {
            backend: Apple,
            swift,
            target,
        }
    }

    #[must_use]
    pub(crate) const fn swift_config(&self) -> &Swift {
        &self.swift
    }

    #[must_use]
    pub(crate) const fn target(&self) -> &AppleTarget {
        &self.target
    }

    const fn configuration(release: bool) -> &'static str {
        if release { "Release" } else { "Debug" }
    }

    fn products_dir(&self, derived_root: &Path, configuration: &str) -> PathBuf {
        match &self.target {
            AppleTarget::Macos => derived_root.join(format!("Build/Products/{configuration}")),
            AppleTarget::IosDevice => {
                derived_root.join(format!("Build/Products/{configuration}-iphoneos"))
            }
            AppleTarget::Simulator(sim) => derived_root.join(format!(
                "Build/Products/{configuration}-{}",
                sim.kind.products_suffix()
            )),
        }
    }
}

impl Platform for ApplePlatform {
    type ToolchainIssue = AnyToolchainIssue;
    type Backend = Apple;

    fn target_triple(&self) -> &'static str {
        match &self.target {
            AppleTarget::Macos => {
                #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
                {
                    "x86_64-apple-darwin"
                }
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    "aarch64-apple-darwin"
                }
                #[cfg(not(target_os = "macos"))]
                {
                    "aarch64-apple-darwin"
                }
            }
            AppleTarget::IosDevice => "aarch64-apple-ios",
            AppleTarget::Simulator(sim) => match sim.kind {
                AppleSimulatorKind::Ios | AppleSimulatorKind::Ipados => "aarch64-apple-ios-sim",
                AppleSimulatorKind::Watchos => "aarch64-apple-watchos-sim",
                AppleSimulatorKind::Tvos => "aarch64-apple-tvos",
                AppleSimulatorKind::Visionos => "aarch64-apple-visionos-sim",
            },
        }
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        let mut issues = Vec::new();

        if let Err(mut backend_issues) = self.backend.check_requirements(project) {
            issues.append(&mut backend_issues);
        }

        if let Some(target) = self.required_rust_target() {
            if let Err(issue) = verify_rust_target_installed(target) {
                issues.push(Box::new(issue) as AnyToolchainIssue);
            }
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    fn package(&self, project: &Project, release: bool) -> Result<PathBuf> {
        ensure_macos_host("Apple packaging")?;

        let project_dir = project.root();
        let xcode = resolve_xcode_project(project_dir, &self.swift)?;
        let derived_root = derived_data_dir(project_dir);
        prepare_derived_data_dir(&derived_root)?;

        let configuration = Self::configuration(release);

        // Build the Rust library first (use internal to allow playground builds)
        let rust_target = self.target_triple();
        info!("Building Rust library for {rust_target}");
        let build_options = BuildOptions::new().with_release(release);
        let build_result = build::build_for_target_internal(project, rust_target, &build_options)?;

        // Copy libwaterui_app.a to where Xcode expects it (BUILT_PRODUCTS_DIR)
        let products_dir = self.products_dir(&derived_root, configuration);
        std::fs::create_dir_all(&products_dir)?;
        let dest_lib = products_dir.join("libwaterui_app.a");
        std::fs::copy(&build_result.artifact_path, &dest_lib)?;
        info!("Copied {} to {}", build_result.artifact_path.display(), dest_lib.display());

        let mut build_cmd = xcodebuild_base(&xcode, configuration, &derived_root);

        match &self.target {
            AppleTarget::Macos => {
                build_cmd.arg("-destination").arg("platform=macOS");
                disable_code_signing(&mut build_cmd);
            }
            AppleTarget::IosDevice => {
                build_cmd.arg("-destination").arg("generic/platform=iOS");
            }
            AppleTarget::Simulator(sim) => {
                build_cmd.arg("-destination").arg(format!(
                    "platform={},{}={}",
                    sim.kind.destination_label(),
                    sim.destination_selector(),
                    sim.reference()
                ));
                disable_code_signing(&mut build_cmd);
            }
        }

        // Skip Rust build in Xcode's build script - we already built it above
        build_cmd.env("WATERUI_SKIP_RUST_BUILD", "1");

        debug!("Executing xcodebuild command: {:?}", build_cmd);
        let log_dir = project.root().join(".water/logs");
        run_xcodebuild_with_progress(
            build_cmd,
            &format!("Building {} ({configuration})", xcode.scheme),
            &log_dir,
        )?;

        let products_dir = self.products_dir(&derived_root, configuration);
        let app_bundle = products_dir.join(format!("{}.app", xcode.scheme));
        if !app_bundle.exists() {
            bail!("Expected app bundle at {}", app_bundle.display());
        }

        Ok(app_bundle)
    }

    fn backend(&self) -> &Self::Backend {
        &self.backend
    }
}

impl AppleSimulatorKind {
    const fn destination_label(self) -> &'static str {
        match self {
            Self::Ios | Self::Ipados => "iOS Simulator",
            Self::Watchos => "watchOS Simulator",
            Self::Tvos => "tvOS Simulator",
            Self::Visionos => "visionOS Simulator",
        }
    }

    const fn products_suffix(self) -> &'static str {
        match self {
            Self::Ios | Self::Ipados => "iphonesimulator",
            Self::Watchos => "watchsimulator",
            Self::Tvos => "appletvsimulator",
            Self::Visionos => "xrsimulator",
        }
    }
}

#[derive(Debug)]
enum ApplePlatformIssue {
    RustupMissing,
    MissingRustTarget { target: &'static str },
}

impl core::fmt::Display for ApplePlatformIssue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RustupMissing => write!(f, "`rustup` was not found on PATH"),
            Self::MissingRustTarget { target } => {
                write!(f, "Rust target `{target}` is not installed")
            }
        }
    }
}

impl ToolchainIssue for ApplePlatformIssue {
    fn suggestion(&self) -> String {
        match self {
            Self::RustupMissing => {
                "Install Rust using https://rustup.rs/ so additional Apple targets can be added."
                    .to_string()
            }
            Self::MissingRustTarget { target } => {
                crate::installer::rust_target_suggestion(target)
            }
        }
    }

    fn fix(&self) -> color_eyre::eyre::Result<()> {
        match self {
            Self::MissingRustTarget { target } => crate::installer::rust_target(target),
            Self::RustupMissing => crate::installer::rust_toolchain(),
        }
    }
}

fn verify_rust_target_installed(target: &'static str) -> Result<(), ApplePlatformIssue> {
    if which("rustup").is_err() {
        return Err(ApplePlatformIssue::RustupMissing);
    }

    if crate::installer::is_rust_target_installed(target) {
        return Ok(());
    }

    // Double-check with direct query for more detailed error
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map_err(|_| ApplePlatformIssue::RustupMissing)?;

    let installed = String::from_utf8_lossy(&output.stdout);
    let has_target = installed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .any(|name| name == target);

    if has_target {
        Ok(())
    } else {
        Err(ApplePlatformIssue::MissingRustTarget { target })
    }
}

impl ApplePlatform {
    const fn required_rust_target(&self) -> Option<&'static str> {
        match &self.target {
            AppleTarget::Macos => None,
            AppleTarget::IosDevice => Some("aarch64-apple-ios"),
            AppleTarget::Simulator(sim) => Some(match sim.kind {
                AppleSimulatorKind::Ios | AppleSimulatorKind::Ipados => "aarch64-apple-ios-sim",
                AppleSimulatorKind::Watchos => "aarch64-apple-watchos-sim",
                AppleSimulatorKind::Tvos => "aarch64-apple-tvos",
                AppleSimulatorKind::Visionos => "aarch64-apple-visionos-sim",
            }),
        }
    }
}
