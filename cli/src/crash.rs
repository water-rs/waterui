use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::platform::PlatformKind;

/// Structured crash diagnostics captured while launching or monitoring an app.
#[derive(Debug, Clone, Serialize)]
pub struct CrashReport {
    pub platform: PlatformKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_identifier: Option<String>,
    /// Bundle identifier / package name used to launch the app.
    pub app_identifier: String,
    /// Milliseconds since UNIX_EPOCH when the crash was recorded.
    pub timestamp_ms: u64,
    /// Path to the log file saved on disk.
    pub log_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_excerpt: Option<String>,
}

impl CrashReport {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        platform: PlatformKind,
        device_name: Option<String>,
        device_identifier: Option<String>,
        app_identifier: String,
        log_path: PathBuf,
        summary: Option<String>,
        log_excerpt: Option<String>,
    ) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        Self {
            platform,
            device_name,
            device_identifier,
            app_identifier,
            timestamp_ms,
            log_path,
            summary,
            log_excerpt,
        }
    }
}
