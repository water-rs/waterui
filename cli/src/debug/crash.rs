//! Structured crash diagnostics captured while launching or monitoring an app.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::PrimitiveDateTime;

/// Structured crash diagnostics captured while launching or monitoring an app.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrashReport {
    time: PrimitiveDateTime,
    device_name: String,
    device_identifier: String,
    app_identifier: String,
    log_path: PathBuf,
}

impl CrashReport {
    /// Create a new crash report
    pub fn new(
        time: PrimitiveDateTime,
        device_name: impl Into<String>,
        device_identifier: impl Into<String>,
        app_identifier: impl Into<String>,
        log_path: PathBuf,
    ) -> Self {
        Self {
            time,
            device_name: device_name.into(),
            device_identifier: device_identifier.into(),
            app_identifier: app_identifier.into(),
            log_path,
        }
    }
}
