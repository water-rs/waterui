//! Structured crash diagnostics captured while launching or monitoring an app.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use smol::process::Command;
use tracing::{debug, info};

/// Check if crash debug output is enabled via `WATERUI_CRASH_DEBUG=1`
fn crash_debug_enabled() -> bool {
    std::env::var("WATERUI_CRASH_DEBUG").is_ok_and(|v| v == "1")
}

macro_rules! crash_debug {
    ($($arg:tt)*) => {
        if crash_debug_enabled() {
            info!($($arg)*);
        } else {
            debug!($($arg)*);
        }
    };
}

/// Structured crash diagnostics captured while launching or monitoring an app.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrashReport {
    time: Timestamp,
    device_name: String,
    device_identifier: String,
    app_identifier: String,
    log_path: PathBuf,
    summary: String,
}

impl CrashReport {
    /// Create a new crash report.
    #[must_use]
    pub fn new(
        time: Timestamp,
        device_name: impl Into<String>,
        device_identifier: impl Into<String>,
        app_identifier: impl Into<String>,
        log_path: PathBuf,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            time,
            device_name: device_name.into(),
            device_identifier: device_identifier.into(),
            app_identifier: app_identifier.into(),
            log_path,
            summary: summary.into(),
        }
    }

    /// Time the crash report was generated.
    #[must_use]
    pub const fn time(&self) -> Timestamp {
        self.time
    }

    /// Device name where the crash happened.
    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Device identifier (UDID/hostname) where the crash happened.
    #[must_use]
    pub fn device_identifier(&self) -> &str {
        &self.device_identifier
    }

    /// App identifier (bundle ID).
    #[must_use]
    pub fn app_identifier(&self) -> &str {
        &self.app_identifier
    }

    /// Path to the crash log on disk.
    #[must_use]
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Human-readable crash summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }
}

impl fmt::Display for CrashReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}\n\nCrash report: {}",
            self.summary,
            self.log_path.display()
        )
    }
}

#[derive(Debug)]
struct IpsReport {
    time: Timestamp,
    bundle_id: Option<String>,
    pid: Option<u32>,
    summary: String,
}

/// Find the most recent macOS `.ips` crash report for a specific app run.
pub async fn find_macos_ips_crash_report_since(
    device_name: &str,
    device_identifier: &str,
    app_identifier: &str,
    process_name: &str,
    pid: Option<u32>,
    since: Timestamp,
) -> Option<CrashReport> {
    let home = std::env::var("HOME").ok()?;
    let crash_dir = PathBuf::from(home).join("Library/Logs/DiagnosticReports");

    if !crash_dir.exists() {
        crash_debug!(
            "Crash report directory does not exist: {}",
            crash_dir.display()
        );
        return None;
    }

    let process_pattern = format!("{process_name}*.ips");
    crash_debug!(
        "Looking for crash reports matching pattern '{}' in {} since {:?}",
        process_pattern,
        crash_dir.display(),
        since
    );

    let candidates = list_recent_ips_reports(&crash_dir, &process_pattern).await;
    let candidate_count = candidates.as_ref().map_or(0, Vec::len);
    crash_debug!("Found {} candidates with process pattern", candidate_count);

    let mut best = if let Some(c) = candidates {
        pick_best_ips_report(c, app_identifier, pid, since).await
    } else {
        None
    };

    if best.is_none() {
        // Fallback: if the crash filename doesn't include the process name (common on iOS simulator),
        // scan recent `.ips` reports and filter by bundle ID / PID.
        crash_debug!("No match with process pattern, falling back to *.ips");
        let candidates = list_recent_ips_reports(&crash_dir, "*.ips").await;
        crash_debug!(
            "Found {} candidates with *.ips pattern",
            candidates.as_ref().map_or(0, |v| v.len())
        );
        if let Some(c) = candidates {
            best = pick_best_ips_report(c, app_identifier, pid, since).await;
        }
    }

    let (path, report) = best?;
    crash_debug!("Matched crash report: {}", path.display());
    Some(CrashReport::new(
        report.time,
        device_name,
        device_identifier,
        app_identifier,
        path,
        report.summary,
    ))
}

async fn pick_best_ips_report(
    candidates: Vec<PathBuf>,
    app_identifier: &str,
    pid: Option<u32>,
    since: Timestamp,
) -> Option<(PathBuf, IpsReport)> {
    let mut best: Option<(PathBuf, IpsReport)> = None;
    for path in candidates {
        let Some(report) = parse_ips_report(&path).await else {
            crash_debug!("Failed to parse crash report: {}", path.display());
            continue;
        };

        crash_debug!(
            "Checking {} - report_time={:?}, since={:?}, bundle_id={:?}, report_pid={:?}, expected_pid={:?}",
            path.display(),
            report.time,
            since,
            report.bundle_id,
            report.pid,
            pid
        );

        if report.time <= since {
            crash_debug!(
                "Skipping {} - report time {:?} is not after start time {:?}",
                path.display(),
                report.time,
                since
            );
            continue;
        }

        match (report.bundle_id.as_deref(), pid, report.pid) {
            (Some(found_bundle_id), _, _) if found_bundle_id != app_identifier => {
                crash_debug!(
                    "Skipping {} - bundle_id '{}' != expected '{}'",
                    path.display(),
                    found_bundle_id,
                    app_identifier
                );
                continue;
            }
            (None, Some(expected_pid), Some(found_pid)) if expected_pid != found_pid => {
                crash_debug!(
                    "Skipping {} - pid {} != expected {}",
                    path.display(),
                    found_pid,
                    expected_pid
                );
                continue;
            }
            (None, Some(_expected_pid), None) => {
                crash_debug!(
                    "Skipping {} - no bundle_id and no pid in report (expected pid {})",
                    path.display(),
                    _expected_pid
                );
                continue;
            }
            (None, None, _) => {
                crash_debug!(
                    "Skipping {} - no bundle_id in report and no expected pid",
                    path.display()
                );
                continue;
            }
            _ => {
                crash_debug!(
                    "Candidate match: {} bundle_id={:?} pid={:?}",
                    path.display(),
                    report.bundle_id,
                    report.pid
                );
            }
        }

        if best
            .as_ref()
            .is_none_or(|(_, current)| report.time > current.time)
        {
            best = Some((path, report));
        }
    }

    best
}

async fn list_recent_ips_reports(crash_dir: &Path, pattern: &str) -> Option<Vec<PathBuf>> {
    let output = Command::new("find")
        .args([
            crash_dir.to_str()?,
            "-name",
            pattern,
            "-type",
            "f",
            "-mmin",
            "-10",
        ])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(stdout.lines().map(PathBuf::from).collect())
}

async fn parse_ips_report(path: &Path) -> Option<IpsReport> {
    let content = smol::fs::read_to_string(path).await.ok()?;

    let mut iter = serde_json::Deserializer::from_str(&content).into_iter::<serde_json::Value>();
    let header = iter.next()?.ok()?;
    let crash = iter.next()?.ok()?;

    let timestamp_str = header.get("timestamp")?.as_str()?;
    crash_debug!(
        "Parsing timestamp from {}: '{}'",
        path.display(),
        timestamp_str
    );
    let time = parse_ips_timestamp(timestamp_str);
    if time.is_none() {
        crash_debug!("Failed to parse timestamp: '{}'", timestamp_str);
        return None;
    }
    let time = time?;

    let crash = crash.get("crash").unwrap_or(&crash);

    let bundle_id = header
        .get("bundleID")
        .or_else(|| header.get("bundleId"))
        .or_else(|| header.get("bundle_identifier"))
        .or_else(|| header.get("bundleIdentifier"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            crash
                .get("bundleID")
                .or_else(|| crash.get("bundleId"))
                .or_else(|| crash.get("bundleIdentifier"))
                .or_else(|| crash.get("bundle_identifier"))
                .or_else(|| crash.get("identifier"))
                .and_then(|v| v.as_str())
        })
        .map(str::to_string);

    let pid = header
        .get("pid")
        .or_else(|| header.get("processID"))
        .or_else(|| header.get("processId"))
        .and_then(value_as_u32);

    let pid = pid.or_else(|| {
        crash
            .get("pid")
            .or_else(|| crash.get("procPid"))
            .or_else(|| crash.get("processID"))
            .or_else(|| crash.get("processId"))
            .and_then(value_as_u32)
    });

    let summary = extract_ips_crash_summary(crash);

    crash_debug!(
        "Parsed IPS report: time={:?}, bundle_id={:?}, pid={:?}",
        time,
        bundle_id,
        pid
    );

    Some(IpsReport {
        time,
        bundle_id,
        pid,
        summary,
    })
}

fn parse_ips_timestamp(timestamp: &str) -> Option<Timestamp> {
    timestamp
        .parse::<Timestamp>()
        .ok()
        .or_else(|| Timestamp::strptime("%Y-%m-%d %H:%M:%S%.f %z", timestamp).ok())
        .or_else(|| Timestamp::strptime("%Y-%m-%d %H:%M:%S %z", timestamp).ok())
        .or_else(|| Timestamp::strptime("%Y-%m-%d %H:%M:%S%.f %:z", timestamp).ok())
        .or_else(|| Timestamp::strptime("%Y-%m-%d %H:%M:%S %:z", timestamp).ok())
}

fn value_as_u32(value: &serde_json::Value) -> Option<u32> {
    match value {
        serde_json::Value::Number(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
        serde_json::Value::String(s) => s.parse::<u32>().ok(),
        _ => None,
    }
}

fn extract_ips_crash_summary(crash: &serde_json::Value) -> String {
    let crash = crash.get("crash").unwrap_or(crash);

    let mut parts = Vec::new();

    if let Some(exception) = crash.get("exception") {
        if let Some(exc_type) = exception.get("type").and_then(|v| v.as_str()) {
            parts.push(format!("Exception: {exc_type}"));
        }
        if let Some(signal) = exception.get("signal").and_then(|v| v.as_str()) {
            parts.push(format!("Signal: {signal}"));
        }
    }

    if let Some(termination) = crash.get("termination") {
        if let Some(indicator) = termination.get("indicator").and_then(|v| v.as_str()) {
            parts.push(format!("Reason: {indicator}"));
        }
    }

    if parts.is_empty() {
        "App crashed".to_string()
    } else {
        parts.join(", ")
    }
}
