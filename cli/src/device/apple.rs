use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{Context, Report, Result, bail};
use serde_json::Value;
use tracing::{debug, warn};
use which::which;

use crate::WATERUI_TRACING_PREFIX;
use crate::{
    backend::apple::ensure_macos_host,
    crash::CrashReport,
    device::Device,
    output,
    platform::{
        PlatformKind,
        apple::{ApplePlatform, AppleSimulatorKind, AppleSimulatorTarget, AppleTarget},
    },
    project::{Project, RunOptions, Swift},
    util,
};

const APPLE_CRASH_OBSERVATION: Duration = Duration::from_secs(8);
const APPLE_LOG_EXCERPT_LINES: usize = 32;

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

    fn run(
        &self,
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

    fn platform(&self) -> Self::Platform {
        self.platform.clone()
    }
}

/// Runs packaged builds on Apple simulators (iOS, iPadOS, watchOS, etc).
#[derive(Clone, Debug)]
pub struct AppleSimulatorDevice {
    platform: ApplePlatform,
}

impl AppleSimulatorDevice {
    #[must_use]
    pub fn new(swift: Swift, kind: AppleSimulatorKind, device_identifier: String) -> Self {
        let reference_is_udid = is_probable_udid(&device_identifier);
        let target = AppleSimulatorTarget {
            kind,
            device_identifier,
            reference_is_udid,
        };
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

    fn run(
        &self,
        project: &Project,
        artifact: &Path,
        options: &RunOptions,
    ) -> Result<Option<CrashReport>> {
        let target = self.simulator_target();
        let device_reference = target.reference();

        let already_booted = simulator_current_state(device_reference)?
            .as_deref()
            .map_or(false, |state| state == "Booted");

        if already_booted {
            debug!("Simulator {device_reference} is already booted; skipping boot step");
        } else {
            let mut boot_cmd = Command::new("xcrun");
            boot_cmd.args(["simctl", "boot", device_reference]);
            let _ = boot_cmd.status();
        }

        let artifact_str = artifact
            .to_str()
            .ok_or_else(|| Report::msg("app bundle path is not valid UTF-8"))?;

        let mut install_cmd = Command::new("xcrun");
        install_cmd.args(["simctl", "install", device_reference, artifact_str]);
        let status = install_cmd
            .status()
            .context("failed to install app on simulator")?;
        if !status.success() {
            bail!("Failed to install app on simulator {}", device_reference);
        }

        let bundle_id = project.bundle_identifier().to_string();
        let process_name = self.platform.swift_config().scheme.clone();
        let crash_collector = AppleCrashCollector::start_simulator(
            project,
            device_reference,
            &process_name,
            &bundle_id,
        )
        .map(Some)
        .unwrap_or_else(|err| {
            warn!("Failed to start simulator crash collector for {device_reference}: {err:?}");
            None
        });
        let mut launch_cmd = Command::new("xcrun");
        launch_cmd.args(["simctl", "launch", "--terminate-running-process"]);
        // Enable Rust backtraces for easier debugging of panics
        launch_cmd.env("SIMCTL_CHILD_RUST_BACKTRACE", "1");
        if options.hot_reload.enabled {
            launch_cmd.env("SIMCTL_CHILD_WATERUI_DISABLE_HOT_RELOAD", "0");
            launch_cmd.env("SIMCTL_CHILD_WATERUI_HOT_RELOAD_HOST", "127.0.0.1");
            if let Some(port) = options.hot_reload.port {
                launch_cmd.env("SIMCTL_CHILD_WATERUI_HOT_RELOAD_PORT", port.to_string());
            }
        } else {
            launch_cmd.env("SIMCTL_CHILD_WATERUI_DISABLE_HOT_RELOAD", "1");
            launch_cmd.env_remove("SIMCTL_CHILD_WATERUI_HOT_RELOAD_HOST");
            launch_cmd.env_remove("SIMCTL_CHILD_WATERUI_HOT_RELOAD_PORT");
        }
        if let Some(filter) = &options.log_filter {
            launch_cmd.env("SIMCTL_CHILD_RUST_LOG", filter);
        }
        launch_cmd.args([device_reference, &bundle_id]);
        let status = launch_cmd.status().context("failed to launch app")?;
        if !status.success() {
            bail!("Failed to launch app on simulator {}", device_reference);
        }

        if crash_collector.is_some() {
            thread::sleep(APPLE_CRASH_OBSERVATION);
        }
        let (device_name, device_identifier) = simulator_device_labels(target);
        let platform_kind = simulator_platform_kind(target.kind);
        let crash_report = match crash_collector {
            Some(collector) => {
                collector.finish(platform_kind, device_name, device_identifier, bundle_id)?
            }
            None => None,
        };

        Ok(crash_report)
    }

    fn platform(&self) -> Self::Platform {
        self.platform.clone()
    }
}

fn simulator_device_labels(target: &AppleSimulatorTarget) -> (Option<String>, Option<String>) {
    if target.reference_is_udid {
        (None, Some(target.device_identifier.clone()))
    } else {
        (Some(target.device_identifier.clone()), None)
    }
}

fn simulator_platform_kind(kind: AppleSimulatorKind) -> PlatformKind {
    match kind {
        AppleSimulatorKind::Ios => PlatformKind::Ios,
        AppleSimulatorKind::Ipados => PlatformKind::Ipados,
        AppleSimulatorKind::Watchos => PlatformKind::Watchos,
        AppleSimulatorKind::Tvos => PlatformKind::Tvos,
        AppleSimulatorKind::Visionos => PlatformKind::Visionos,
    }
}

struct AppleCrashCollector {
    child: Child,
    stop_flag: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<AppleLogResult>>,
    log_path: PathBuf,
    show_lines: bool,
    footer_printed: bool,
}

impl AppleCrashCollector {
    fn start_macos(project: &Project, process_name: &str, bundle_id: &str) -> Result<Self> {
        let log_path = apple_log_path(project, bundle_id)?;
        let predicate = format!("process == \"{process_name}\"");
        let mut cmd = Command::new("log");
        cmd.args(["stream", "--style", "json", "--predicate", &predicate]);
        Self::spawn(cmd, log_path, process_name.to_string())
    }

    fn start_simulator(
        project: &Project,
        device_reference: &str,
        process_name: &str,
        bundle_id: &str,
    ) -> Result<Self> {
        let log_path = apple_log_path(project, bundle_id)?;
        let predicate = format!("process == \"{process_name}\"");
        let mut cmd = Command::new("xcrun");
        cmd.args([
            "simctl",
            "spawn",
            device_reference,
            "log",
            "stream",
            "--style",
            "json",
            "--predicate",
            &predicate,
        ]);
        Self::spawn(cmd, log_path, process_name.to_string())
    }

    fn spawn(mut cmd: Command, log_path: PathBuf, label: String) -> Result<Self> {
        cmd.stdout(Stdio::piped());
        let mut child = cmd.spawn().context("failed to start Apple log stream")?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Report::msg("failed to capture log stream output"))?;
        let file = File::create(&log_path).context("failed to create Apple crash log")?;
        let stop_flag = Arc::new(AtomicBool::new(false));
        let reader_flag = Arc::clone(&stop_flag);
        let show_lines = !output::global_output_format().is_json();

        if show_lines {
            let header = format!("Apple logs ({label})");
            println!();
            println!("{}", header);
        }

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

                let parsed = serde_json::from_str::<Value>(&line).ok();
                let message = parsed
                    .as_ref()
                    .and_then(|value| value.get("eventMessage").and_then(Value::as_str))
                    .unwrap_or(&line);

                if show_lines && !is_forwarded_tracing_line(message) {
                    println!("{message}");
                }

                if is_crash_event(parsed.as_ref(), message) {
                    crash_detected = true;
                    if summary.is_none() {
                        summary = Some(message.to_string());
                    }
                }

                excerpt.push_back(line);
                if excerpt.len() > APPLE_LOG_EXCERPT_LINES {
                    excerpt.pop_front();
                }
            }

            let _ = writer.flush();
            AppleLogResult {
                crash_detected,
                summary,
                log_excerpt: if excerpt.is_empty() {
                    None
                } else {
                    Some(excerpt.into_iter().collect::<Vec<_>>().join("\n"))
                },
            }
        });

        Ok(Self {
            child,
            stop_flag,
            join_handle: Some(handle),
            log_path,
            show_lines,
            footer_printed: false,
        })
    }

    fn finish(
        mut self,
        platform: PlatformKind,
        device_name: Option<String>,
        device_identifier: Option<String>,
        bundle_id: String,
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
            println!("End of Apple logs");
            println!();
            self.footer_printed = true;
        }

        if result.crash_detected {
            Ok(Some(CrashReport::new(
                platform,
                device_name,
                device_identifier,
                bundle_id,
                self.log_path.clone(),
                result.summary,
                result.log_excerpt,
            )))
        } else {
            let _ = fs::remove_file(&self.log_path);
            Ok(None)
        }
    }
}

impl Drop for AppleCrashCollector {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
        if self.show_lines && !self.footer_printed {
            println!("End of Apple logs");
            println!();
            self.footer_printed = true;
        }
    }
}

#[derive(Default)]
struct AppleLogResult {
    crash_detected: bool,
    summary: Option<String>,
    log_excerpt: Option<String>,
}

fn apple_log_path(project: &Project, bundle_id: &str) -> Result<PathBuf> {
    let log_dir = project.root().join(".water/logs");
    util::ensure_directory(&log_dir)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sanitized = bundle_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    Ok(log_dir.join(format!("apple-crash-{sanitized}-{timestamp}.log")))
}

fn is_relevant_apple_log_line(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    const KEYWORDS: &[&str] = &[
        "fatal",
        "sigsegv",
        "sigabrt",
        "exception",
        "terminating",
        "crash",
        "assertion",
    ];
    KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn is_crash_event(parsed: Option<&Value>, message: &str) -> bool {
    if let Some(value) = parsed {
        if let Some(event_type) = value.get("eventType").and_then(Value::as_str) {
            if matches!(
                event_type.to_ascii_lowercase().as_str(),
                "fault" | "crash" | "error"
            ) {
                return true;
            }
        }
    }
    is_relevant_apple_log_line(message)
}

fn is_forwarded_tracing_line(message: &str) -> bool {
    message.contains(WATERUI_TRACING_PREFIX)
}

fn debug_launch_simulator_app() -> Result<()> {
    Command::new("open")
        .arg("-a")
        .arg("Simulator")
        .status()
        .context("failed to open Simulator app")?;
    Ok(())
}

fn simulator_current_state(device_reference: &str) -> Result<Option<String>> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "-j", "devices"])
        .output()
        .context("failed to query simulator list")?;
    if !output.status.success() {
        return Ok(None);
    }

    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let devices = match value.get("devices").and_then(Value::as_object) {
        Some(devices) => devices,
        None => return Ok(None),
    };

    for entries in devices.values() {
        if let Some(array) = entries.as_array() {
            for entry in array {
                let udid = entry
                    .get("udid")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let matches_reference = (!udid.is_empty()
                    && udid.eq_ignore_ascii_case(device_reference))
                    || name == device_reference;
                if matches_reference {
                    if let Some(state) = entry.get("state").and_then(Value::as_str) {
                        return Ok(Some(state.to_string()));
                    }
                }
            }
        }
    }

    Ok(None)
}

fn is_probable_udid(value: &str) -> bool {
    if value.trim().is_empty() {
        return false;
    }
    let trimmed = value.trim();
    if trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return matches!(trimmed.len(), 24..=40);
    }
    if trimmed
        .chars()
        .all(|ch| ch.is_ascii_hexdigit() || ch == '-')
    {
        let cleaned_len = trimmed.chars().filter(|c| *c != '-').count();
        return matches!(cleaned_len, 24..=40);
    }
    false
}

fn require_tool(tool: &str, hint: &str) -> Result<()> {
    if which(tool).is_ok() {
        Ok(())
    } else {
        bail!("{tool} not found. {hint}")
    }
}
