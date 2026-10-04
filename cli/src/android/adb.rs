//! The `adb` client, with its server running.
//!
//! `adb`'s first client command launches the server daemon, and on Windows
//! that daemon inherits every inheritable handle the client held — which is
//! every one this process held, including the pipe whoever ran `water` is
//! reading. The server outlives us, so that reader never sees end-of-file.
//! An [`Adb`] therefore exists only once `adb start-server` has run through
//! [`Host::run_detached`], where the launcher gets no handle of ours at all;
//! every later client command finds the server already up and spawns
//! nothing. Code that needs the server takes an `&Adb`, never a bare path.

use std::path::{Path, PathBuf};

use crate::{android::toolchain::AndroidSdk, toolchain::Host, utils::CommandError};

/// Why no [`Adb`] could be produced.
#[derive(Debug, thiserror::Error)]
pub enum AdbError {
    /// No Android SDK, or its platform-tools carry no `adb`.
    #[error("Android SDK not found or adb not installed")]
    NotFound,
    /// `adb start-server` could not be run.
    #[error("failed to start the adb server: {0}")]
    ServerLauncher(#[from] CommandError),
    /// `adb start-server` ran and reported failure.
    #[error("`{adb} start-server` failed with status {status}", adb = .adb.display())]
    ServerStart {
        /// The client that was run.
        adb: PathBuf,
        /// Its exit status.
        status: std::process::ExitStatus,
    },
}

/// `adb` from the platform-tools on a host, its server already running.
#[derive(Debug, Clone)]
pub struct Adb {
    path: PathBuf,
}

impl Adb {
    /// Locate `adb` on `host` and make sure its server is up.
    ///
    /// # Errors
    /// [`AdbError::NotFound`] when the SDK has no `adb`; the other variants
    /// when the server could not be started.
    pub async fn locate(host: &Host) -> Result<Self, AdbError> {
        let path = AndroidSdk::adb_path(host).ok_or(AdbError::NotFound)?;
        let status = host.run_detached(&path, ["start-server"]).await?;
        if !status.success() {
            return Err(AdbError::ServerStart { adb: path, status });
        }
        Ok(Self { path })
    }

    /// The client executable.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Adb, AdbError};
    use crate::toolchain::testing::TestMachine;

    #[test]
    fn without_platform_tools_there_is_no_adb() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        let host = machine.host([(
            OsString::from("ANDROID_SDK_ROOT"),
            sdk.as_os_str().to_os_string(),
        )]);
        let error = smol::block_on(Adb::locate(&host)).expect_err("no adb staged");
        assert!(matches!(error, AdbError::NotFound), "{error:?}");
    }

    /// The staged `adb.exe` on Windows carries shell text that `CreateProcess`
    /// cannot run, so the launcher tests are Unix-only.
    #[test]
    #[cfg(unix)]
    fn locating_adb_starts_its_server_first() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        let staged = machine.install_adb();
        let host = machine.host([(
            OsString::from("ANDROID_SDK_ROOT"),
            sdk.as_os_str().to_os_string(),
        )]);
        let adb = smol::block_on(Adb::locate(&host)).expect("the fake adb starts its server");
        assert_eq!(adb.path(), staged);
    }

    #[test]
    #[cfg(unix)]
    fn a_failing_server_launch_is_an_error() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let host = machine.host([
            (
                OsString::from("ANDROID_SDK_ROOT"),
                sdk.as_os_str().to_os_string(),
            ),
            (
                OsString::from("WATERUI_FAKE_ADB_START_SERVER_STATUS"),
                OsString::from("3"),
            ),
        ]);
        let error = smol::block_on(Adb::locate(&host)).expect_err("the launcher exits 3");
        match error {
            AdbError::ServerStart { status, .. } => assert_eq!(status.code(), Some(3)),
            other => panic!("expected ServerStart, got {other:?}"),
        }
    }
}
