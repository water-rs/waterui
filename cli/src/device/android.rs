use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{Context, Report, Result, bail};
use console::style;

use crate::WATERUI_TRACING_PREFIX;
use crate::{
    backend::android::{
        adb_command, configure_rust_android_linker_env, device_preferred_targets,
        find_android_tool, sanitize_package_name, wait_for_android_device,
    },
    crash::CrashReport,
    device::{Device, DeviceKind},
    output,
    platform::{PlatformKind, android::AndroidPlatform},
    project::{Project, RunOptions},
    util,
};
const PID_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
const PID_DISAPPEAR_GRACE: Duration = Duration::from_secs(2);
const APP_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const LOG_EXCERPT_LINES: usize = 20;

/// Represents the target Android device or emulator selected by the user.
#[derive(Clone, Debug)]
pub struct AndroidSelection {
    pub name: String,
    pub identifier: Option<String>,
    pub kind: DeviceKind,
}

#[derive(Debug)]
pub struct AndroidDevice {
    platform: AndroidPlatform,
    /// Detected device target architectures, set during prepare()
    detected_targets: RwLock<Option<Vec<String>>>,
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
            detected_targets: RwLock::new(None),
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

    fn start_crash_collector(
        &self,
        project: &Project,
        package: &str,
    ) -> Result<AndroidCrashCollector> {
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

        let log_dir = project.root().join(".water/logs");
        util::ensure_directory(&log_dir)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let logfile = log_dir.join(format!("android-crash-{package}-{timestamp}.log"));
        let file = File::create(&logfile).context("failed to create Android crash log")?;
        let show_lines = !output::global_output_format().is_json();

        if show_lines {
            let header = style(format!("Android warnings/errors ({package})"))
                .yellow()
                .bold();
            println!();
            println!("{header}");
        }

        Ok(AndroidCrashCollector::new(
            child,
            stdout,
            file,
            logfile,
            package.to_string(),
            show_lines,
        ))
    }

    fn wait_for_app_exit(&self, package: &str) -> Result<()> {
        let mut seen_pid = false;
        let mut last_seen = Instant::now();
        let start = Instant::now();

        loop {
            if let Some(_) = self.query_app_pid(package)? {
                seen_pid = true;
                last_seen = Instant::now();
            } else {
                if seen_pid && last_seen.elapsed() > PID_DISAPPEAR_GRACE {
                    break;
                }
                if !seen_pid && start.elapsed() > PID_APPEAR_TIMEOUT {
                    break;
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

        // Store detected targets to pass to platform.package() later
        *self.detected_targets.write().unwrap() =
            Some(targets.iter().map(|s| (*s).to_string()).collect());

        Ok(())
    }

    fn run(
        &self,
        project: &Project,
        artifact: &Path,
        options: &RunOptions,
    ) -> Result<Option<CrashReport>> {
        let artifact_str = artifact
            .to_str()
            .ok_or_else(|| Report::msg("APK path is not valid UTF-8"))?;

        let sanitized = sanitize_package_name(project.bundle_identifier());
        self.clear_logcat_buffer()?;
        let crash_collector = self.start_crash_collector(project, &sanitized)?;

        let mut install_cmd = adb_command(&self.adb_path, self.selection_identifier());
        install_cmd.args(["install", "-r", artifact_str]);
        let status = install_cmd.status().context("failed to install APK")?;
        if !status.success() {
            bail!("Failed to install APK on target device");
        }

        let activity = format!("{sanitized}/.MainActivity");
        let mut reverse_guard = None;
        let mut launch_cmd = adb_command(&self.adb_path, self.selection_identifier());
        launch_cmd.args(["shell", "am", "start", "-n", &activity]);
        if options.hot_reload.enabled {
            let port = options.hot_reload.port.ok_or_else(|| {
                Report::msg("Hot reload server port missing; restart the CLI and try again.")
            })?;
            reverse_guard = Some(AdbReverseGuard::new(
                &self.adb_path,
                self.selection_identifier(),
                port,
            )?);
            launch_cmd
                .args(["--es", "WATERUI_HOT_RELOAD_HOST", "127.0.0.1"])
                .args(["--es", "WATERUI_HOT_RELOAD_PORT", &port.to_string()])
                .args(["--ez", "WATERUI_DISABLE_HOT_RELOAD", "false"]);
        } else {
            launch_cmd.args(["--ez", "WATERUI_DISABLE_HOT_RELOAD", "true"]);
        }
        if let Some(filter) = &options.log_filter {
            launch_cmd.args(["--es", "WATERUI_LOG_FILTER", filter]);
        }
        let status = launch_cmd.status().context("failed to launch app")?;
        if !status.success() {
            bail!("Failed to launch Android activity");
        }

        if !output::global_output_format().is_json() {
            println!(
                "{} {}",
                style("•").blue(),
                format!(
                    "Streaming Android warnings/errors for {sanitized}. Close the app or press Ctrl+C to stop."
                )
            );
        }
        self.wait_for_app_exit(&sanitized)?;
        let crash_report = crash_collector.finish(
            PlatformKind::Android,
            Some(self.selection.name.clone()),
            self.selection.identifier.clone(),
            sanitized,
        )?;

        drop(reverse_guard);

        Ok(crash_report)
    }

    fn platform(&self) -> Self::Platform {
        let targets = self.detected_targets.read().unwrap().clone();
        self.platform.clone().with_targets(targets)
    }
}

struct AdbReverseGuard {
    adb_path: PathBuf,
    identifier: Option<String>,
    port: u16,
    active: bool,
}

impl AdbReverseGuard {
    fn new(adb_path: &Path, identifier: Option<&str>, port: u16) -> Result<Self> {
        let spec = format!("tcp:{port}");
        let mut cmd = adb_command(adb_path, identifier);
        cmd.args(["reverse", &spec, &spec]);
        let status = cmd
            .status()
            .context("failed to configure adb reverse tunnel for hot reload")?;
        if !status.success() {
            bail!(
                "adb reverse failed with status {} while enabling hot reload",
                status
            );
        }

        Ok(Self {
            adb_path: adb_path.to_path_buf(),
            identifier: identifier.map(|value| value.to_string()),
            port,
            active: true,
        })
    }
}

impl Drop for AdbReverseGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let spec = format!("tcp:{}", self.port);
        let mut cmd = adb_command(&self.adb_path, self.identifier.as_deref());
        cmd.args(["reverse", "--remove", &spec]);
        let _ = cmd.status();
        self.active = false;
    }
}

struct AndroidCrashCollector {
    child: Child,
    stop_flag: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<LogCaptureResult>>,
    log_path: PathBuf,
    show_lines: bool,
    footer_printed: bool,
}

impl AndroidCrashCollector {
    fn new(
        child: Child,
        stdout: ChildStdout,
        file: File,
        log_path: PathBuf,
        package: String,
        show_lines: bool,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let reader_flag = Arc::clone(&stop_flag);

        let handle = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut writer = BufWriter::new(file);
            let mut crash_detected = false;
            let mut summary = None;
            let mut excerpt: VecDeque<String> = VecDeque::new();

            for line in reader.lines() {
                if reader_flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(line) = line else {
                    break;
                };

                let _ = writeln!(writer, "{line}");

                if show_lines
                    && is_relevant_android_log_line(&line, &package)
                    && !is_forwarded_tracing_line(&line)
                {
                    if is_trigger_line(&line) {
                        println!("{}", style(&line).red().bold());
                    } else {
                        println!("{}", style(&line).yellow());
                    }
                }

                if is_trigger_line(&line) {
                    crash_detected = true;
                    if summary.is_none() {
                        summary = Some(line.clone());
                    }
                }

                excerpt.push_back(line);
                if excerpt.len() > LOG_EXCERPT_LINES {
                    excerpt.pop_front();
                }
            }

            let _ = writer.flush();
            LogCaptureResult {
                crash_detected,
                summary,
                log_excerpt: if excerpt.is_empty() {
                    None
                } else {
                    Some(excerpt.into_iter().collect::<Vec<_>>().join("\n"))
                },
            }
        });

        Self {
            child,
            stop_flag,
            join_handle: Some(handle),
            log_path,
            show_lines,
            footer_printed: false,
        }
    }

    fn finish(
        mut self,
        platform: PlatformKind,
        device_name: Option<String>,
        device_identifier: Option<String>,
        app_identifier: String,
    ) -> Result<Option<CrashReport>> {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();

        let result = self
            .join_handle
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();

        if self.show_lines {
            let footer = style("End of Android logs").yellow();
            println!("{footer}");
            println!();
            self.footer_printed = true;
        }

        if result.crash_detected {
            Ok(Some(CrashReport::new(
                platform,
                device_name,
                device_identifier,
                app_identifier,
                self.log_path.clone(),
                result.summary,
                result.log_excerpt,
            )))
        } else {
            Ok(None)
        }
    }
}

impl Drop for AndroidCrashCollector {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
        if self.show_lines && !self.footer_printed {
            let footer = style("End of Android logs").yellow();
            println!("{footer}");
            println!();
            self.footer_printed = true;
        }
    }
}

#[derive(Default)]
struct LogCaptureResult {
    crash_detected: bool,
    summary: Option<String>,
    log_excerpt: Option<String>,
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

fn is_forwarded_tracing_line(line: &str) -> bool {
    line.contains(WATERUI_TRACING_PREFIX)
}
