use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

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
const PID_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
const PID_DISAPPEAR_GRACE: Duration = Duration::from_secs(2);
const APP_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(500);

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

    fn clear_logcat_buffer(&self) -> Result<()> {
        let mut cmd = adb_command(&self.adb_path, self.selection_identifier());
        cmd.args(["logcat", "-c"]);
        let status = cmd
            .status()
            .context("failed to clear Android logcat buffer")?;
        if !status.success() {
            bail!("ADB failed while clearing logcat buffer: {status}");
        }
        Ok(())
    }

    fn start_log_stream(&self, package: &str) -> Result<AndroidLogcatStream> {
        let mut cmd = adb_command(&self.adb_path, self.selection_identifier());
        cmd.args([
            "logcat", "-b", "crash", "-b", "main", "-b", "system", "-v", "time",
        ]);
        cmd.stdout(Stdio::piped());
        let mut child = cmd.spawn().context("failed to start adb logcat")?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Report::msg("failed to capture adb logcat output"))?;

        Ok(AndroidLogcatStream::new(package, child, stdout))
    }

    fn wait_for_app_exit(&self, package: &str) -> Result<()> {
        let mut seen_pid = false;
        let mut last_seen = Instant::now();
        let start = Instant::now();

        loop {
            match self.query_app_pid(package)? {
                Some(_) => {
                    seen_pid = true;
                    last_seen = Instant::now();
                }
                None => {
                    if seen_pid && last_seen.elapsed() > PID_DISAPPEAR_GRACE {
                        break;
                    }
                    if !seen_pid && start.elapsed() > PID_APPEAR_TIMEOUT {
                        break;
                    }
                }
            }

            thread::sleep(APP_EXIT_POLL_INTERVAL);
        }

        Ok(())
    }

    fn query_app_pid(&self, package: &str) -> Result<Option<String>> {
        let mut cmd = adb_command(&self.adb_path, self.selection_identifier());
        cmd.args(["shell", "pidof", package]);
        let output = cmd
            .output()
            .context("failed to query application PID via adb")?;
        if !output.status.success() {
            return Ok(None);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
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

        let sanitized = sanitize_package_name(project.bundle_identifier());
        let capture_logs = !output::global_output_format().is_json();
        let mut log_stream = if capture_logs {
            self.clear_logcat_buffer()?;
            Some(self.start_log_stream(&sanitized)?)
        } else {
            None
        };

        let mut install_cmd = adb_command(&self.adb_path, self.selection_identifier());
        install_cmd.args(["install", "-r", artifact_str]);
        let status = install_cmd.status().context("failed to install APK")?;
        if !status.success() {
            bail!("Failed to install APK on target device");
        }

        let activity = format!("{sanitized}/.MainActivity");
        let mut launch_cmd = adb_command(&self.adb_path, self.selection_identifier());
        launch_cmd.args(["shell", "am", "start", "-n", &activity]);
        let status = launch_cmd.status().context("failed to launch app")?;
        if !status.success() {
            bail!("Failed to launch Android activity");
        }

        if let Some(stream) = log_stream.as_mut() {
            println!(
                "{} {}",
                style("•").blue(),
                format!(
                    "Streaming Android warnings/errors for {sanitized}. Close the app or press Ctrl+C to stop."
                )
            );
            self.wait_for_app_exit(&sanitized)?;
            stream.stop();
        }

        Ok(())
    }

    fn platform(&self) -> &Self::Platform {
        &self.platform
    }
}

struct AndroidLogcatStream {
    child: Child,
    stop_flag: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
    footer_printed: bool,
}

impl AndroidLogcatStream {
    fn new(package: &str, child: Child, stdout: ChildStdout) -> Self {
        let header = style(format!("Android warnings/errors ({package})"))
            .yellow()
            .bold();
        println!();
        println!("{header}");

        let stop_flag = Arc::new(AtomicBool::new(false));
        let reader_flag = Arc::clone(&stop_flag);
        let package = package.to_string();

        let handle = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if reader_flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(line) = line else {
                    break;
                };
                if is_relevant_android_log_line(&line, &package) {
                    if is_trigger_line(&line) {
                        println!("{}", style(line).red().bold());
                    } else {
                        println!("{}", style(line).yellow());
                    }
                }
            }
        });

        Self {
            child,
            stop_flag,
            join_handle: Some(handle),
            footer_printed: false,
        }
    }

    fn stop(&mut self) {
        if self.footer_printed {
            return;
        }
        self.footer_printed = true;
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
        let footer = style("End of Android logs").yellow();
        println!("{footer}");
        println!();
    }
}

impl Drop for AndroidLogcatStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn is_relevant_android_log_line(line: &str, package: &str) -> bool {
    if !package.is_empty() && line.contains(package) {
        return true;
    }
    let lower = line.to_ascii_lowercase();
    const KEYWORDS: &[&str] = &[
        "fatal exception",
        "fatal signal",
        "sigabrt",
        "sigsegv",
        "abort message",
        "unsatisfiedlinkerror",
        "androidruntime",
        "beginning of crash",
        "waterui",
        "panic",
        "waterui_root_ready",
    ];
    KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn is_trigger_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    const TRIGGERS: &[&str] = &[
        "fatal exception",
        "fatal signal",
        "sigabrt",
        "sigsegv",
        "abort message",
        "unsatisfiedlinkerror",
        "androidruntime",
        "panic",
        "waterui_root_ready",
    ];
    TRIGGERS.iter().any(|kw| lower.contains(kw))
}
