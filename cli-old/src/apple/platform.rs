use std::path::Path;

use color_eyre::eyre;
use heck::ToUpperCamelCase;
use smol::process::Command;
use target_lexicon::Triple;
use tracing::warn;

use crate::{
    apple::{device::AppleDevice, toolchain::AppleToolchain},
    platform::Platform,
    project::Project,
    utils::run_command,
};

pub struct ApplePlatform {
    triple: Triple,
}

impl Platform for ApplePlatform {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;

    async fn clean(&self, project: &Project) -> color_eyre::eyre::Result<()> {
        clean_project(project).await
    }

    async fn package(
        &self,
        project: &Project,
        options: &crate::platform::PackageOptions,
    ) -> eyre::Result<()> {
    }

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {}

    fn description(&self) -> String {
        format!("Apple Platform ({})", self.triple)
    }

    fn toolchain(&self) -> &Self::Toolchain {
        todo!()
    }
}

async fn clean_project(project: &Project) -> eyre::Result<()> {
    // run command, clean xcode build artifacts
    let ident = project.crate_name().to_upper_camel_case(); // TODO: validate here
    let mut cmd = Command::new("xcodebuild");
    let cmd = cmd
        .arg("-workspace")
        .arg(format!("apple/{ident}.xcworkspace"))
        .arg("-scheme")
        .arg(ident)
        .arg("clean")
        .current_dir(project.root());

    run_command(cmd).await?;

    Ok(())
}

fn run_macos(
    project: &Project,
    artifact: &Path,
    options: &RunOptions,
) -> Result<Option<CrashReport>> {
    let bundle_id = project.bundle_identifier().to_string();
    let process_name = self.platform.swift_config().scheme.clone();
    let crash_collector = AppleCrashCollector::start_macos(project, &process_name, &bundle_id)
        .map(Some)
        .unwrap_or_else(|err| {
            warn!("Failed to start macOS crash collector: {err:?}");
            None
        });
    if options.hot_reload.enabled {
        let executable = self.executable_path(artifact);
        if !executable.exists() {
            bail!("App executable not found at {}", executable.display());
        }
        let mut cmd = Command::new(&executable);
        // Enable Rust backtraces for easier debugging of panics
        cmd.env("RUST_BACKTRACE", "1");
        util::configure_hot_reload_env(&mut cmd, true, options.hot_reload.port);
        if let Some(filter) = &options.log_filter {
            cmd.env("RUST_LOG", filter);
        }
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

    if crash_collector.is_some() {
        thread::sleep(APPLE_CRASH_OBSERVATION);
    }

    let crash_report = match crash_collector {
        Some(collector) => collector.finish(
            PlatformKind::Macos,
            Some("macOS".to_string()),
            None,
            bundle_id,
        )?,
        None => None,
    };

    Ok(crash_report)
}
