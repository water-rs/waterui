//! The exit-status channel between the harness application and the
//! runner.
//!
//! `simctl launch` and `devicectl ... --console` both exit independently of
//! the application's status, so the harness writes its conclusion byte to
//! the file `WATERUI_NATIVE_TEST_STATUS` names and the runner exits with
//! it. A launch that leaves no status fails explicitly.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use eyre::{Context as _, bail, eyre};

/// The byte the harness wrote; the runner's process exits with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestStatus(u8);

impl TestStatus {
    /// The status for a reported exit `code`.
    #[must_use]
    pub const fn new(code: u8) -> Self {
        Self(code)
    }

    /// The byte the harness wrote.
    #[must_use]
    pub const fn code(&self) -> u8 {
        self.0
    }
}

/// What the `WATERUI_NATIVE_TEST_STATUS` value names for one launch.
///
/// Unique per launch, so a stale file from an earlier launch can never be
/// read as this launch's result — the device has no remote delete, and
/// uniqueness is how staleness is ruled out there.
#[derive(Debug)]
#[non_exhaustive]
pub enum StatusFile {
    /// A host-absolute path (the simulator destination).
    HostAbsolute(PathBuf),
    /// A path relative to the application's home — its data container —
    /// which the harness resolves itself (the device destination; the
    /// device's filesystem is not the host's).
    DeviceRelative(PathBuf),
}

/// A launch-unique file name — collisions with a stale file from an
/// earlier launch must never happen.
pub fn unique_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    format!("waterui-native-test-status-{}-{nanos}", std::process::id())
}

impl StatusFile {
    /// A unique file inside `state_dir` (already created).
    #[must_use]
    pub fn host_absolute(state_dir: &std::path::Path) -> Self {
        Self::HostAbsolute(state_dir.join(unique_name()))
    }

    /// A unique path under the app container's `tmp/`, which iOS purges.
    #[must_use]
    pub fn device_relative() -> Self {
        Self::DeviceRelative(PathBuf::from("tmp").join(unique_name()))
    }

    /// The value the harness gets in `WATERUI_NATIVE_TEST_STATUS`.
    #[must_use]
    pub fn env_value(&self) -> String {
        match self {
            Self::HostAbsolute(path) | Self::DeviceRelative(path) => path.display().to_string(),
        }
    }

    /// The file name a host-side copy of the status lands under — the
    /// remote name is already unique.
    #[must_use]
    pub fn local_name(&self) -> PathBuf {
        match self {
            Self::HostAbsolute(path) | Self::DeviceRelative(path) => path
                .file_name()
                .map_or_else(|| PathBuf::from("status"), PathBuf::from),
        }
    }

    /// The host-side file the runner reads the status from.
    ///
    /// On the device destination this is the local pull destination, not
    /// the remote path: it lives under `state_dir`.
    #[must_use]
    pub fn local_path(&self, state_dir: &std::path::Path) -> PathBuf {
        match self {
            Self::HostAbsolute(path) => path.clone(),
            Self::DeviceRelative(_) => state_dir.join(self.local_name()),
        }
    }

    /// The remote path `devicectl device copy from --source` takes —
    /// `env_value` for the device-relative variant.
    ///
    /// # Errors
    /// Returns an error when the value is not a string the tool accepts.
    pub fn remote_path(&self) -> eyre::Result<String> {
        match self {
            Self::DeviceRelative(path) => Ok(path
                .to_str()
                .ok_or_else(|| eyre!("the status path is not UTF-8: {}", path.display()))?
                .to_string()),
            Self::HostAbsolute(path) => bail!(
                "a host-absolute status ({}) has no device-relative path",
                path.display()
            ),
        }
    }

    /// Turns the status bytes into the runner's exit status.
    ///
    /// A missing or empty file — the process died before the harness
    /// concluded — fails with the "died before its trials concluded"
    /// message; a non-integer or >255 value quotes the content.
    ///
    /// # Errors
    /// Returns an error when `content` is absent or is not a status byte.
    pub fn conclude(content: Option<&[u8]>, bundle_id: &str) -> eyre::Result<TestStatus> {
        let content = content.filter(|bytes| !bytes.is_empty()).ok_or_else(|| {
            eyre!(
                "{bundle_id} exited without reporting a status — the harness application died \
                 before its trials concluded"
            )
        })?;
        let text = std::str::from_utf8(content)
            .wrap_err("the status file the harness wrote is not UTF-8")?;
        let trimmed = text.trim();
        let status: u32 = trimmed.parse().map_err(|_| {
            eyre!("the status {bundle_id} reported is not an exit status: {trimmed:?}")
        })?;
        let status = u8::try_from(status)
            .map_err(|_| eyre!("the status {bundle_id} reported is out of range: {trimmed:?}"))?;
        Ok(TestStatus::new(status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_status_is_died_before_conclusion() {
        let error = StatusFile::conclude(None, "dev.test").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("died before its trials concluded")
        );
        let error = StatusFile::conclude(Some(b""), "dev.test").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("died before its trials concluded")
        );
    }

    #[test]
    fn a_status_byte_round_trips() {
        assert_eq!(
            StatusFile::conclude(Some(b"0"), "dev.test").unwrap().code(),
            0
        );
        assert_eq!(
            StatusFile::conclude(Some(b"42\n"), "dev.test")
                .unwrap()
                .code(),
            42
        );
        assert_eq!(
            StatusFile::conclude(Some(b"255"), "dev.test")
                .unwrap()
                .code(),
            255
        );
    }

    #[test]
    fn garbage_status_quotes_the_content() {
        let error = StatusFile::conclude(Some(b"crashed"), "dev.test").unwrap_err();
        assert!(error.to_string().contains("\"crashed\""));
        let error = StatusFile::conclude(Some(b"300"), "dev.test").unwrap_err();
        assert!(error.to_string().contains("\"300\""));
    }
}
