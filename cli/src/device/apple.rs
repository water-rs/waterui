use std::{
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::eyre::{Context, Report, Result, bail};
use which::which;

use crate::{
    backend::apple::ensure_macos_host,
    device::Device,
    platform::apple::{ApplePlatform, AppleSimulatorKind, AppleSimulatorTarget, AppleTarget},
    project::{Project, RunOptions, Swift},
    util,
};

/// Launches the packaged macOS application on the local host.
#[derive(Clone, Debug)]
pub struct MacosDevice {
    platform: ApplePlatform,
}

impl MacosDevice {
    #[must_use]
    pub const fn new(swift: Swift) -> Self {
        Self {
            platform: ApplePlatform::new(swift, AppleTarget::Macos),
        }
    }

    fn executable_path(&self, artifact: &Path) -> PathBuf {
        let scheme = &self.platform.swift_config().scheme;
        artifact.join("Contents").join("MacOS").join(scheme)
    }
}

impl Device for MacosDevice {
    type Platform = ApplePlatform;

    fn prepare(&self, _project: &Project, _options: &RunOptions) -> Result<()> {
        ensure_macos_host("macOS runtime")?;
        Ok(())
    }

    fn run(&self, _project: &Project, artifact: &Path, options: &RunOptions) -> Result<()> {
        if options.hot_reload.enabled {
            let executable = self.executable_path(artifact);
            if !executable.exists() {
                bail!("App executable not found at {}", executable.display());
            }
            let mut cmd = Command::new(&executable);
            util::configure_hot_reload_env(&mut cmd, true, options.hot_reload.port);
            cmd.spawn()
                .context("failed to launch macOS app executable")?;
        } else {
            let status = Command::new("open")
                .arg(artifact)
                .status()
                .context("failed to open app bundle")?;
            if !status.success() {
                bail!("Failed to launch macOS app");
            }
        }

        Ok(())
    }

    fn platform(&self) -> &Self::Platform {
        &self.platform
    }
}

/// Runs packaged builds on Apple simulators (iOS, iPadOS, watchOS, etc).
#[derive(Clone, Debug)]
pub struct AppleSimulatorDevice {
    platform: ApplePlatform,
}

impl AppleSimulatorDevice {
    #[must_use]
    pub const fn new(swift: Swift, kind: AppleSimulatorKind, device_name: String) -> Self {
        let target = AppleSimulatorTarget { kind, device_name };
        Self {
            platform: ApplePlatform::new(swift, AppleTarget::Simulator(target)),
        }
    }

    fn simulator_target(&self) -> &AppleSimulatorTarget {
        match self.platform.target() {
            AppleTarget::Simulator(target) => target,
            _ => unreachable!("simulator device must be constructed with simulator target"),
        }
    }
}

impl Device for AppleSimulatorDevice {
    type Platform = ApplePlatform;

    fn prepare(&self, _project: &Project, _options: &RunOptions) -> Result<()> {
        ensure_macos_host("Apple simulators")?;
        require_tool(
            "xcrun",
            "Install Xcode and command line tools (xcode-select --install)",
        )?;
        require_tool(
            "xcodebuild",
            "Install Xcode and command line tools (xcode-select --install)",
        )?;
        debug_launch_simulator_app()?;
        Ok(())
    }

    fn run(&self, project: &Project, artifact: &Path, options: &RunOptions) -> Result<()> {
        let target = self.simulator_target();
        let device_name = &target.device_name;

        let mut boot_cmd = Command::new("xcrun");
        boot_cmd.args(["simctl", "boot", device_name]);
        let _ = boot_cmd.status();

        let artifact_str = artifact
            .to_str()
            .ok_or_else(|| Report::msg("app bundle path is not valid UTF-8"))?;

        let mut install_cmd = Command::new("xcrun");
        install_cmd.args(["simctl", "install", device_name, artifact_str]);
        let status = install_cmd
            .status()
            .context("failed to install app on simulator")?;
        if !status.success() {
            bail!("Failed to install app on simulator {}", device_name);
        }

        let bundle_id = project.bundle_identifier();
        let mut launch_cmd = Command::new("xcrun");
        launch_cmd.args(["simctl", "launch", "--terminate-running-process"]);
        if options.hot_reload.enabled {
            launch_cmd.env("SIMCTL_CHILD_WATERUI_DISABLE_HOT_RELOAD", "0");
            if let Some(port) = options.hot_reload.port {
                launch_cmd.env("SIMCTL_CHILD_WATERUI_HOT_RELOAD_PORT", port.to_string());
            }
        } else {
            launch_cmd.env("SIMCTL_CHILD_WATERUI_DISABLE_HOT_RELOAD", "1");
        }
        launch_cmd.args([device_name, bundle_id]);
        let status = launch_cmd.status().context("failed to launch app")?;
        if !status.success() {
            bail!("Failed to launch app on simulator {}", device_name);
        }

        Ok(())
    }

    fn platform(&self) -> &Self::Platform {
        &self.platform
    }
}

fn debug_launch_simulator_app() -> Result<()> {
    Command::new("open")
        .arg("-a")
        .arg("Simulator")
        .status()
        .context("failed to open Simulator app")?;
    Ok(())
}

fn require_tool(tool: &str, hint: &str) -> Result<()> {
    if which(tool).is_ok() {
        Ok(())
    } else {
        bail!("{tool} not found. {hint}")
    }
}
