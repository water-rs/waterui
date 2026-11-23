//! Headless execution of `aarch64-apple-ios-sim` preview binaries using the
//! installed Xcode iOS Simulator runtime directly (without booting
//! `CoreSimulator`/`SpringBoard` and without creating any on-screen macOS window).

#[cfg(target_os = "macos")]
mod cache;
#[cfg(target_os = "macos")]
mod runtime;

use std::path::Path;
use std::process::Output;

use eyre::{Result, bail};

use crate::toolchain::Host;

/// Execution parameters for a headless iOS Simulator preview binary.
#[derive(Debug, Clone, Copy)]
pub struct IosSimPreviewLaunch<'a> {
    /// Working directory for the preview process.
    pub working_dir: &'a Path,
    /// Built `aarch64-apple-ios-sim` preview executable.
    pub binary_path: &'a Path,
    /// Path to `preview-run.json` passed in `WATERUI_PREVIEW_RUN_CONFIG`.
    pub run_config_path: &'a Path,
    /// Logical viewport width in points.
    pub width: f32,
    /// Logical viewport height in points.
    pub height: f32,
    /// Display scale factor (e.g. `2.0` or `3.0`).
    pub scale: f32,
    /// Requested capture frame rate in Hz (defaults to `60`).
    pub fps: Option<u32>,
}

/// Spawn and run an `aarch64-apple-ios-sim` preview binary headlessly through
/// `waterui-simhost`, returning its captured [`Output`].
///
/// # Errors
///
/// Returns an error on non-macOS hosts, when no arm64 iOS Simulator runtime is
/// installed, when helper compilation fails, or when the preview binary exits
/// non-zero.
#[cfg(target_os = "macos")]
pub async fn run_ios_sim_preview(host: &Host, launch: IosSimPreviewLaunch<'_>) -> Result<Output> {
    use eyre::Context as _;
    use smol::fs;
    use tempfile::Builder;
    use waterui_preview_protocol::run::PREVIEW_RUN_CONFIG_ENV;

    use crate::utils::command;

    let runtime = runtime::discover_ios_sim_runtime(host).await?;
    let helpers = cache::ensure_helpers(host, &runtime).await?;

    let container = Builder::new()
        .prefix("waterui-ios-sim-")
        .tempdir()
        .wrap_err("Failed to create temporary container for iOS Simulator preview")?;
    let tmp_dir = container.path().join("tmp");
    fs::create_dir_all(&tmp_dir)
        .await
        .wrap_err("Failed to create temporary directory inside simulator container")?;

    let scale_int = launch.scale.round().max(1.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "viewport dimensions and scale are validated positive preview bounds"
    )]
    let pixel_width = (f64::from(launch.width) * f64::from(scale_int))
        .round()
        .max(1.0) as u32;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "viewport dimensions and scale are validated positive preview bounds"
    )]
    let pixel_height = (f64::from(launch.height) * f64::from(scale_int))
        .round()
        .max(1.0) as u32;
    let pitch = if scale_int >= 3.0 { 460.0 } else { 326.0 };
    let refresh_centihz = launch.fps.unwrap_or(60).max(1) * 100;

    let mut child = host.command(&helpers.simhost)?;
    let child = command(&mut child, host.std_output());
    child.current_dir(launch.working_dir);
    child.args([
        helpers.simns.as_os_str(),
        helpers.simfb.as_os_str(),
        helpers.interpose.as_os_str(),
        launch.binary_path.as_os_str(),
    ]);
    child.env(PREVIEW_RUN_CONFIG_ENV, launch.run_config_path);
    child.env("WATERUI_SIMHOST_RUNTIME_ROOT", &runtime.runtime_root);
    child.env("WATERUI_SIMHOST_CONTAINER", container.path());
    child.env("SIMULATOR_ARCHS", "arm64");
    child.env("SIMULATOR_DEVICE_NAME", "WaterUI Preview");
    child.env("SIMULATOR_LEGACY_ASSET_SUFFIX", "iphone");
    child.env("SIMULATOR_UDID", "00000000-0000-0000-0000-000000000000");
    child.env("SIMULATOR_MAINSCREEN_WIDTH", pixel_width.to_string());
    child.env("SIMULATOR_MAINSCREEN_HEIGHT", pixel_height.to_string());
    child.env("SIMULATOR_MAINSCREEN_SCALE", format!("{scale_int:.6}"));
    child.env("SIMULATOR_MAINSCREEN_PITCH", format!("{pitch:.6}"));
    child.env("SIMULATOR_RUNTIME_VERSION", &runtime.version);
    child.env("SIMULATOR_RUNTIME_BUILD_VERSION", &runtime.build_version);
    child.env("WATERUI_SIMFB_REFRESH_CENTIHZ", refresh_centihz.to_string());
    child.env("WATERUI_SIMNS_DAEMONS", "com.apple.notifyd");
    child.env("XPC_FLAGS", "0x0");
    child.env("XPC_SIMULATOR_LAUNCHD_NAME", "dev.waterui.preview.simns");

    let output = child.output().await.wrap_err_with(|| {
        format!(
            "Failed to run iOS Simulator preview binary {}",
            launch.binary_path.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        bail!("iOS Simulator preview binary failed: {details}");
    }

    Ok(output)
}

/// Non-macOS stub failing cleanly when `--platform ios` preview is invoked off macOS.
#[cfg(not(target_os = "macos"))]
pub async fn run_ios_sim_preview(_host: &Host, launch: IosSimPreviewLaunch<'_>) -> Result<Output> {
    let IosSimPreviewLaunch {
        working_dir,
        binary_path,
        run_config_path,
        width,
        height,
        scale,
        fps,
    } = launch;
    let _ = (working_dir, run_config_path, width, height, scale, fps);
    bail!(
        "`water preview --platform ios` (for {}) requires a macOS host with an iOS Simulator runtime",
        binary_path.display()
    )
}
