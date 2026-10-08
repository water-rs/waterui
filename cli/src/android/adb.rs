//! The `adb` client, with its server running.
//!
//! `adb`'s first client command launches the server daemon, and on Windows
//! that daemon inherits every inheritable handle the client held — which is
//! every one this process held, including the pipe whoever ran `water` is
//! reading. The server outlives us, so that reader never sees end-of-file.
//! An [`Adb`](crate::android::adb::Adb) therefore exists only once `adb
//! start-server` has run through
//! [`Host::run_detached`](crate::toolchain::Host::run_detached), where the
//! launcher gets no handle of ours at all;
//! every later client command finds the server already up and spawns
//! nothing. Code that needs the server takes an `&Adb`, never a bare path.
//!
//! Every command runs through [`run_bounded_adb_output`]: an `adb` verb can
//! wedge on a stalled transport, so each carries a deadline.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output};
use std::time::Duration;

use futures_util::future::{Either, select};

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

/// An `adb` device command could not be spawned, timed out, or exited
/// unsuccessfully.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AdbCommandError {
    /// The `adb` process could not be spawned.
    #[error(transparent)]
    Spawn(#[from] CommandError),
    /// The command did not finish within the bound.
    #[error("{operation} timed out after {seconds} seconds")]
    Timeout {
        /// Human-readable name of the operation.
        operation: String,
        /// The bound that elapsed.
        seconds: u64,
    },
    /// The command exited with a non-zero status.
    #[error("{operation} failed with status {status}{details}")]
    Failed {
        /// Human-readable name of the operation.
        operation: String,
        /// The process exit status.
        status: ExitStatus,
        /// Formatted stdout/stderr tail.
        details: String,
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

    /// `adb -s <serial> <args…>` under `timeout`, returning the full output.
    async fn device_output<A, S>(
        &self,
        host: &Host,
        serial: &str,
        args: A,
        operation: &str,
        timeout: Duration,
    ) -> Result<Output, AdbCommandError>
    where
        A: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let full_args: Vec<OsString> = [OsStr::new("-s"), OsStr::new(serial)]
            .into_iter()
            .map(OsString::from)
            .chain(args.into_iter().map(|arg| arg.as_ref().to_os_string()))
            .collect();
        run_bounded_adb_output(host, self, full_args, operation, timeout).await
    }

    /// `adb -s <serial> push <local> <remote>`.
    ///
    /// # Errors
    /// Returns an error if the push fails.
    pub(crate) async fn push(
        &self,
        host: &Host,
        serial: &str,
        local: &Path,
        remote: &str,
        timeout: Duration,
    ) -> Result<(), AdbCommandError> {
        self.device_command(
            host,
            serial,
            [OsStr::new("push"), local.as_os_str(), OsStr::new(remote)],
            "pushing a payload to the device",
            timeout,
        )
        .await?;
        Ok(())
    }

    /// `adb -s <serial> shell <words>` — `words` is an argv, joined with
    /// `shlex` so callers never hand a pre-quoted line. Returns the full
    /// output: the caller judges the status (`am instrument` reports results
    /// in its stdout, not its exit code).
    ///
    /// # Errors
    /// Returns an error if `words` cannot be quoted or `adb` itself fails.
    pub(crate) async fn shell(
        &self,
        host: &Host,
        serial: &str,
        words: &[&str],
        timeout: Duration,
    ) -> eyre::Result<Output> {
        let joined = shlex::try_join(words.iter().copied())
            .map_err(|error| eyre::eyre!("cannot quote the adb shell words {words:?}: {error}"))?;
        Ok(self
            .device_output(
                host,
                serial,
                [OsStr::new("shell"), OsStr::new(joined.as_str())],
                "running a shell command on the device",
                timeout,
            )
            .await?)
    }

    /// [`Self::shell`] failing on a non-zero exit and returning stdout —
    /// for shell verbs whose own status carries the verdict.
    ///
    /// # Errors
    /// Returns an error if `words` cannot be quoted or the command fails.
    pub(crate) async fn shell_run(
        &self,
        host: &Host,
        serial: &str,
        words: &[&str],
        timeout: Duration,
    ) -> eyre::Result<String> {
        let output = self.shell(host, serial, words, timeout).await?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(eyre::eyre!(
                "adb -s {serial} shell {} failed: {}{}",
                words.join(" "),
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            ))
        }
    }

    /// `adb -s <serial> shell run-as <package> <words>` — a command inside
    /// `package`'s private data, so a debuggable host's staged files stay
    /// reachable. Returns stdout, failing on a non-zero exit.
    ///
    /// # Errors
    /// Returns an error if `words` cannot be quoted or the command fails.
    pub(crate) async fn run_as(
        &self,
        host: &Host,
        serial: &str,
        package: &str,
        words: &[&str],
        timeout: Duration,
    ) -> eyre::Result<String> {
        let joined = shlex::try_join(words.iter().copied())
            .map_err(|error| eyre::eyre!("cannot quote the run-as words {words:?}: {error}"))?;
        Ok(run_bounded_adb_command(
            host,
            self,
            [
                OsString::from("-s"),
                OsString::from(serial),
                OsString::from("shell"),
                OsString::from("run-as"),
                OsString::from(package),
                OsString::from(joined),
            ],
            "running a command inside the preview host's private data",
            timeout,
        )
        .await?)
    }

    /// `adb -s <serial> exec-out run-as <package> cat <path>` — the bytes of
    /// a file inside `package`'s private data. A non-zero status or empty
    /// stdout is an error naming the path: a pushed payload never reads back
    /// empty.
    ///
    /// # Errors
    /// Returns an error if the read fails or produces nothing.
    pub(crate) async fn exec_out_run_as_cat(
        &self,
        host: &Host,
        serial: &str,
        package: &str,
        path: &str,
        timeout: Duration,
    ) -> eyre::Result<Vec<u8>> {
        let output = self
            .device_output(
                host,
                serial,
                ["exec-out", "run-as", package, "cat", path],
                "reading a file from the preview host's private data",
                timeout,
            )
            .await?;
        if !output.status.success() || output.stdout.is_empty() {
            eyre::bail!(
                "failed to read {path} from package {package} on {serial}: {}{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
        }
        Ok(output.stdout)
    }

    /// The `versionCode` the device reports for `package`, `None` when it is
    /// not installed — parsed from `pm list packages --show-versioncode`.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub(crate) async fn installed_version_code(
        &self,
        host: &Host,
        serial: &str,
        package: &str,
        timeout: Duration,
    ) -> Result<Option<u32>, AdbCommandError> {
        let output = self
            .device_command(
                host,
                serial,
                [
                    OsStr::new("shell"),
                    OsStr::new("pm"),
                    OsStr::new("list"),
                    OsStr::new("packages"),
                    OsStr::new("--show-versioncode"),
                    OsStr::new(package),
                ],
                "querying the installed package's version code",
                timeout,
            )
            .await?;
        Ok(parse_installed_version_code(&output, package))
    }

    /// `adb -s <serial> install -r -d <apk>` — replace any installed copy of
    /// the package, including one with a higher `versionCode` (`-d`), since
    /// the code is a fingerprint projection rather than a monotone version.
    ///
    /// # Errors
    /// Returns an error if the install fails.
    pub(crate) async fn install_any_version(
        &self,
        host: &Host,
        serial: &str,
        apk: &Path,
        timeout: Duration,
    ) -> Result<(), AdbCommandError> {
        self.device_command(
            host,
            serial,
            [
                OsStr::new("install"),
                OsStr::new("-r"),
                OsStr::new("-d"),
                apk.as_os_str(),
            ],
            "installing the preview host APK",
            timeout,
        )
        .await?;
        Ok(())
    }

    /// [`Self::device_output`] failing on a non-zero exit, stdout on success.
    async fn device_command<A, S>(
        &self,
        host: &Host,
        serial: &str,
        args: A,
        operation: &str,
        timeout: Duration,
    ) -> Result<String, AdbCommandError>
    where
        A: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self
            .device_output(host, serial, args, operation, timeout)
            .await?;
        check_bounded_output(&output, operation)
    }
}

/// Run `adb` with `args` under `timeout`, returning its full output.
/// `host.output` pipes both streams, so a verbose verb drains concurrently
/// rather than deadlocking on a full buffer; the deadline bounds a stalled
/// transport, and `kill_on_drop` leaves no orphan when it fires.
///
/// The timeout applies to the whole spawn-and-drain: `Host::output` itself
/// carries no bound.
///
/// # Errors
/// [`AdbCommandError::Spawn`] when `adb` cannot be started, or
/// [`AdbCommandError::Timeout`] when it exceeds `timeout`.
pub(crate) async fn run_bounded_adb_output<A, S>(
    host: &Host,
    adb: &Adb,
    args: A,
    operation: &str,
    timeout: Duration,
) -> Result<Output, AdbCommandError>
where
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|argument| argument.as_ref().to_os_string())
        .collect::<Vec<_>>();
    let operation = operation.to_owned();
    let command = Box::pin(async move {
        host.output(adb.path(), &args)
            .await
            .map_err(AdbCommandError::from)
    });
    let seconds = timeout.as_secs();
    let timeout = Box::pin(async move {
        smol::Timer::after(timeout).await;
        Err(AdbCommandError::Timeout { operation, seconds })
    });

    match select(command, timeout).await {
        Either::Left((result, _)) | Either::Right((result, _)) => result,
    }
}

/// [`run_bounded_adb_output`] plus the status check: stdout text on success,
/// [`AdbCommandError::Failed`] with the stream tails otherwise.
///
/// # Errors
/// Returns an error if the adb invocation fails or exits non-zero.
pub(crate) async fn run_bounded_adb_command<A, S>(
    host: &Host,
    adb: &Adb,
    args: A,
    operation: &str,
    timeout: Duration,
) -> Result<String, AdbCommandError>
where
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_bounded_adb_output(host, adb, args, operation, timeout).await?;
    check_bounded_output(&output, operation)
}

/// The non-zero-status branch shared by [`run_bounded_adb_command`] and
/// [`Adb::device_command`].
fn check_bounded_output(output: &Output, operation: &str) -> Result<String, AdbCommandError> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = if !stderr.is_empty() {
        format!("\nstderr:\n{stderr}")
    } else if !stdout.is_empty() {
        format!("\nstdout:\n{stdout}")
    } else {
        String::new()
    };
    Err(AdbCommandError::Failed {
        operation: operation.to_owned(),
        status: output.status,
        details,
    })
}

/// The recent crash evidence `logcat` holds for a launch or preview that
/// produced no output — the AndroidRuntime/DEBUG/WaterUI tags plus the
/// `hydrolysis` tag the preview entry writes under.
///
/// # Errors
/// Never fails: a `logcat` that itself errors answers the failure text, so
/// the caller's diagnostic is always complete.
pub(crate) async fn recent_crash_log(host: &Host, adb: &Adb, serial: &str) -> String {
    match run_bounded_adb_command(
        host,
        adb,
        [
            "-s",
            serial,
            "logcat",
            "-d",
            "-t",
            "100",
            "-s",
            "AndroidRuntime:E",
            "DEBUG:*",
            "WaterUI:*",
            "hydrolysis:*",
        ],
        "collecting Android crash logs",
        Duration::from_secs(10),
    )
    .await
    {
        Ok(output) => output,
        Err(error) => format!("(failed to collect logcat crash info: {error})"),
    }
}

/// Parse `pm list packages --show-versioncode` output for `package`: a line
/// `package:<package> versionCode:<n>`. Missing or malformed output answers
/// `None` — the device simply does not have that package.
fn parse_installed_version_code(output: &str, package: &str) -> Option<u32> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("package:")?;
        let (name, version) = rest.split_once(' ')?;
        if name != package {
            return None;
        }
        version.strip_prefix("versionCode:")?.parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;

    use super::{Adb, AdbError, parse_installed_version_code};
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

    #[test]
    fn installed_version_code_parses_the_show_versioncode_line() {
        assert_eq!(
            parse_installed_version_code(
                "package:dev.waterui.hydrolysis.preview versionCode:42\n",
                "dev.waterui.hydrolysis.preview"
            ),
            Some(42)
        );
        // A package that is not installed yields no line at all.
        assert_eq!(
            parse_installed_version_code("", "dev.waterui.hydrolysis.preview"),
            None
        );
        // Another package's line is not this package's.
        assert_eq!(
            parse_installed_version_code(
                "package:dev.waterui.other versionCode:7\n",
                "dev.waterui.hydrolysis.preview"
            ),
            None
        );
    }

    /// The fake `adb` records nothing, but its shell dispatch answers every
    /// verb the same way; what matters here is that `push` and `run-as`
    /// construct their word lists without a quoting escape.
    #[test]
    #[cfg(unix)]
    fn push_and_run_as_construct_their_word_lists() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let host = machine.host([(
            OsString::from("ANDROID_SDK_ROOT"),
            sdk.as_os_str().to_os_string(),
        )]);
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb");

        smol::block_on(async {
            adb.push(
                &host,
                "serial",
                Path::new("dir with space/lib"),
                "/data/local/tmp/waterui-preview",
                Duration::from_secs(5),
            )
            .await
            .expect("the fake adb accepts push");
            let pwd = adb
                .run_as(
                    &host,
                    "serial",
                    "dev.waterui.hydrolysis.preview",
                    &["pwd"],
                    Duration::from_secs(5),
                )
                .await
                .expect("the fake adb accepts run-as");
            let _ = pwd;
        });
    }
}
