use std::{path::Path, process::Command, thread, time::Duration};

use color_eyre::eyre::{Context, Report, Result, bail};
use console::style;

use crate::{
    backend::android::{
        adb_command, configure_rust_android_linker_env, device_preferred_targets,
        find_android_tool, sanitize_package_name, wait_for_android_device,
    },
    device::{Device, DeviceKind},
    output,
    platform::android::AndroidPlatform,
    project::{Project, RunOptions},
};
use tracing::warn;

const FALLBACK_LOG_LINE_LIMIT: usize = 200;
const LOGCAT_CAPTURE_DELAY: Duration = Duration::from_secs(2);
const LOGCAT_CAPTURE_ATTEMPTS: usize = 3;
const LOGCAT_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Represents the target Android device or emulator selected by the user.
#[derive(Clone, Debug)]
pub struct AndroidSelection {
    pub name: String,
    pub identifier: Option<String>,
    pub kind: DeviceKind,
}

#[derive(Clone, Debug)]
pub struct AndroidDevice {
    platform: AndroidPlatform,
    selection: AndroidSelection,
    adb_path: std::path::PathBuf,
    emulator_path: Option<std::path::PathBuf>,
}

impl AndroidDevice {
    /// Create a new Android device wrapper bound to the provided platform configuration.
    ///
    /// # Errors
    /// Returns an error if required Android SDK tools cannot be located.
    pub fn new(platform: AndroidPlatform, selection: AndroidSelection) -> Result<Self> {
        let adb_path = find_android_tool("adb").ok_or_else(|| {
            Report::msg(
                "`adb` not found. Install the Android SDK platform-tools and ensure they are available in your Android SDK directory or on PATH."
            )
        })?;
        let emulator_path = find_android_tool("emulator");
        Ok(Self {
            platform,
            selection,
            adb_path,
            emulator_path,
        })
    }

    fn selection_identifier(&self) -> Option<&str> {
        if self.selection.kind == DeviceKind::Device {
            self.selection.identifier.as_deref()
        } else {
            None
        }
    }

    fn launch_emulator_if_needed(&self) -> Result<()> {
        if self.selection.kind != DeviceKind::Emulator {
            return Ok(());
        }

        let emulator = self.emulator_path.as_ref().ok_or_else(|| {
            Report::msg(
                "`emulator` not found. Install the Android SDK emulator tools or add them to PATH.",
            )
        })?;

        Command::new(emulator)
            .arg("-avd")
            .arg(&self.selection.name)
            .spawn()
            .context("failed to launch Android emulator")?;
        Ok(())
    }

    fn clear_logcat_buffer(&self) {
        let mut cmd = adb_command(&self.adb_path, self.selection_identifier());
        cmd.args(["logcat", "-c"]);
        match cmd.status() {
            Ok(status) => {
                if !status.success() {
                    warn!("Failed to clear Android logcat buffer: {status}");
                }
            }
            Err(err) => warn!("Unable to clear Android logcat buffer: {err:?}"),
        }
    }

    fn capture_recent_logs(&self) -> Result<Option<String>> {
        let mut cmd = adb_command(&self.adb_path, self.selection_identifier());
        cmd.args([
            "logcat", "-b", "crash", "-b", "main", "-b", "system", "-d", "*:V",
        ]);
        let output = cmd
            .output()
            .context("failed to capture Android logcat output")?;
        if !output.status.success() {
            bail!("adb logcat exited with {}", output.status);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let limited = limit_log_lines(trimmed, FALLBACK_LOG_LINE_LIMIT);
        Ok(Some(limited))
    }

    fn collect_log_capture(&self) -> Result<Option<String>> {
        for attempt in 0..LOGCAT_CAPTURE_ATTEMPTS {
            if let Some(capture) = self.capture_recent_logs()? {
                return Ok(Some(capture));
            }
            if attempt + 1 < LOGCAT_CAPTURE_ATTEMPTS {
                thread::sleep(LOGCAT_RETRY_DELAY);
            }
        }
        Ok(None)
    }

    fn print_log_capture(&self, package: &str) -> Result<()> {
        if output::global_output_format().is_json() {
            return Ok(());
        }

        thread::sleep(LOGCAT_CAPTURE_DELAY);

        if let Some(capture) = self.collect_log_capture()? {
            let header = style(format!("Android warnings/errors ({package})"))
                .yellow()
                .bold();
            let footer = style("End of Android logs").yellow();
            println!();
            println!("{header}");
            println!("{capture}");
            println!("{footer}");
            println!();
        } else {
            println!(
                "{} {}",
                style("•").blue(),
                format!(
                    "Unable to capture recent Android warnings/errors for {package}. Try `adb logcat` for more details."
                )
            );
        }
        Ok(())
    }
}

impl Device for AndroidDevice {
    type Platform = AndroidPlatform;

    fn prepare(&self, _project: &Project, _options: &RunOptions) -> Result<()> {
        self.launch_emulator_if_needed()?;

        wait_for_android_device(&self.adb_path, self.selection_identifier())
            .context("failed while waiting for Android device")?;

        let targets = device_preferred_targets(&self.adb_path, self.selection_identifier())
            .context("Failed to determine device CPU architecture")?;
        configure_rust_android_linker_env(&targets)
            .context("Failed to configure Android NDK toolchain for Rust builds")?;
        Ok(())
    }

    fn run(&self, project: &Project, artifact: &Path, _options: &RunOptions) -> Result<()> {
        let artifact_str = artifact
            .to_str()
            .ok_or_else(|| Report::msg("APK path is not valid UTF-8"))?;

        let capture_logs = !output::global_output_format().is_json();
        if capture_logs {
            self.clear_logcat_buffer();
        }

        let mut install_cmd = adb_command(&self.adb_path, self.selection_identifier());
        install_cmd.args(["install", "-r", artifact_str]);
        let status = install_cmd.status().context("failed to install APK")?;
        if !status.success() {
            bail!("Failed to install APK on target device");
        }

        let sanitized = sanitize_package_name(project.bundle_identifier());
        let activity = format!("{sanitized}/.MainActivity");
        let mut launch_cmd = adb_command(&self.adb_path, self.selection_identifier());
        launch_cmd.args(["shell", "am", "start", "-n", &activity]);
        let status = launch_cmd.status().context("failed to launch app")?;
        if !status.success() {
            bail!("Failed to launch Android activity");
        }

        if capture_logs {
            thread::sleep(LOGCAT_CAPTURE_DELAY);
            if let Err(err) = self.print_log_capture(&sanitized) {
                warn!("Unable to capture Android warnings/errors: {err:?}");
            }
        }

        Ok(())
    }

    fn platform(&self) -> &Self::Platform {
        &self.platform
    }
}

fn limit_log_lines(contents: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= max_lines {
        return contents.to_string();
    }
    lines[lines.len() - max_lines..].join("\n")
}
