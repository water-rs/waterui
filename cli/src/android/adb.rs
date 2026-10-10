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
//! Every command runs through `run_bounded_adb_output`: an `adb` verb can
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
    #[error("{operation} timed out after {seconds} seconds running `{command}`")]
    Timeout {
        /// Human-readable name of the operation.
        operation: String,
        /// The bound that elapsed.
        seconds: u64,
        /// The adb invocation that timed out.
        command: String,
    },
    /// The command exited with a non-zero status.
    #[error("{operation} failed with status {status} running `{command}`{details}")]
    Failed {
        /// Human-readable name of the operation.
        operation: String,
        /// The process exit status.
        status: ExitStatus,
        /// The adb invocation that failed.
        command: String,
        /// Formatted stdout/stderr tail.
        details: String,
    },
}

/// `adb` from the platform-tools on a host, its server already running.
#[derive(Debug, Clone)]
pub struct Adb {
    path: PathBuf,
}

/// One `adb forward` registration on a device: a local TCP port bound to a
/// remote spec.
#[derive(Debug)]
pub(crate) struct Forward {
    /// The local `tcp:` port.
    pub(crate) local_port: u16,
    /// The remote spec the port forwards to (`localabstract:<name>`).
    pub(crate) remote: String,
}

/// Parse one `forward --list` line — `serial tcp:<port> <remote>`. The
/// adb server lists every device's forwards whatever `-s` names, so a line
/// for another serial answers `None`; a line that is not three fields of
/// that shape is an error.
fn parse_forward_line(serial: &str, line: &str) -> eyre::Result<Option<Forward>> {
    let malformed = || eyre::eyre!("an `adb forward --list` line does not parse: {line:?}");
    let mut fields = line.split_whitespace();
    let (Some(entry_serial), Some(local), Some(remote), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(malformed());
    };
    let local_port = local
        .strip_prefix("tcp:")
        .and_then(|port| port.parse::<u16>().ok())
        .ok_or_else(malformed)?;
    Ok((entry_serial == serial).then(|| Forward {
        local_port,
        remote: remote.to_string(),
    }))
}

impl Adb {
    /// Locate `adb` on `host` and make sure its server is up.
    ///
    /// # Errors
    /// [`AdbError::NotFound`] when the SDK has no `adb`; the other variants
    /// when the server could not be started.
    pub async fn locate(host: &Host) -> Result<Self, AdbError> {
        let adb = Self {
            path: AndroidSdk::adb_path(host).ok_or(AdbError::NotFound)?,
        };
        adb.start_server(host).await?;
        Ok(adb)
    }

    /// Run `adb start-server` through
    /// [`Host::run_detached`](crate::toolchain::Host::run_detached): a no-op
    /// against a running server, and a fresh server that inherits none of
    /// our handles when the one this client found has since died.
    ///
    /// A device phase that follows a long build calls this first — a server
    /// killed while the build ran would otherwise be relaunched by the
    /// phase's first ordinary client command, which hands it our pipes.
    ///
    /// # Errors
    /// [`AdbError::ServerLauncher`] when the launcher cannot be run,
    /// [`AdbError::ServerStart`] when it reports failure.
    pub async fn start_server(&self, host: &Host) -> Result<(), AdbError> {
        let status = host.run_detached(&self.path, ["start-server"]).await?;
        if !status.success() {
            return Err(AdbError::ServerStart {
                adb: self.path.clone(),
                status,
            });
        }
        Ok(())
    }

    /// The client executable.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `adb -s <serial> <args…>` under `timeout`, returning the output and
    /// the rendered invocation for the caller's error messages.
    async fn device_output<A, S>(
        &self,
        host: &Host,
        serial: &str,
        args: A,
        operation: &str,
        timeout: Duration,
    ) -> Result<(Output, String), AdbCommandError>
    where
        A: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let full_args: Vec<OsString> = [OsStr::new("-s"), OsStr::new(serial)]
            .into_iter()
            .map(OsString::from)
            .chain(args.into_iter().map(|arg| arg.as_ref().to_os_string()))
            .collect();
        let invocation = command_string(self.path(), &full_args);
        let output = run_bounded_adb_output(host, self, full_args, operation, timeout).await?;
        Ok((output, invocation))
    }

    /// `adb -s <serial> push <local> <remote>` — adb's sync protocol, which
    /// carries the bytes unaltered and fails the command on any transfer
    /// error.
    ///
    /// # Errors
    /// Returns an error if the push fails or exceeds `timeout`.
    pub(crate) async fn push(
        &self,
        host: &Host,
        serial: &str,
        local: &Path,
        remote: &str,
        timeout: Duration,
    ) -> Result<(), AdbCommandError> {
        let operation = "pushing files to the device";
        let (output, invocation) = self
            .device_output(
                host,
                serial,
                [OsStr::new("push"), local.as_os_str(), OsStr::new(remote)],
                operation,
                timeout,
            )
            .await?;
        check_bounded_output(&output, operation, &invocation)
    }

    /// The shared `shell` argv build-and-run behind [`Self::shell_run`]:
    /// `words` joined with `shlex` so callers never hand a pre-quoted line,
    /// returning the full output and the rendered invocation for
    /// [`check_bounded_output`].
    async fn shell_output(
        &self,
        host: &Host,
        serial: &str,
        words: &[&str],
        timeout: Duration,
    ) -> eyre::Result<(Output, String)> {
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

    /// [`Self::shell_output`] failing on a non-zero exit and returning
    /// stdout — for shell verbs whose own status carries the verdict.
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
        let (output, command) = self.shell_output(host, serial, words, timeout).await?;
        Ok(checked_stdout(
            &output,
            "running a shell command on the device",
            &command,
        )?)
    }

    /// `adb -s <serial> shell -T run-as <package> cat <path>` — the bytes of
    /// a file inside `package`'s private data.
    ///
    /// `exec-out` always exits 0 and mixes remote stderr into its stdout, so
    /// a failing `cat` there would hand back its own error text as file
    /// bytes; `shell -T` keeps the remote exit status and stderr separate,
    /// and `-T` pins the no-PTY byte channel a binary read needs.
    ///
    /// # Errors
    /// Returns an error if the read fails.
    pub(crate) async fn run_as_cat(
        &self,
        host: &Host,
        serial: &str,
        package: &str,
        path: &str,
        timeout: Duration,
    ) -> eyre::Result<Vec<u8>> {
        let operation = "reading a file from the preview host's private data";
        let args = shell_byte_channel_args(serial, &["run-as", package, "cat", path])?;
        let invocation = command_string(self.path(), &args);
        let output = bounded_adb(
            host.output(self.path(), &args),
            operation,
            invocation.clone(),
            timeout,
        )
        .await?;
        check_bounded_output(&output, operation, &invocation)?;
        Ok(output.stdout)
    }

    /// `adb -s <serial> shell -T <words>` with `input` streamed into the
    /// remote command's stdin. Returns stdout, failing on a non-zero exit.
    ///
    /// The shell protocol behind `shell -T` carries the remote exit status
    /// and signals the end of `input` to the remote command; `exec-in`
    /// does neither, and the device kills its command once the client
    /// disconnects, so a remote failure there would read as success.
    ///
    /// # Errors
    /// Returns an error if `words` cannot be quoted, `input` cannot be fed,
    /// or the command fails or exceeds `timeout`.
    pub(crate) async fn shell_with_input(
        &self,
        host: &Host,
        serial: &str,
        words: &[&str],
        input: impl smol::io::AsyncRead,
        timeout: Duration,
    ) -> eyre::Result<String> {
        let operation = "running a shell command with input on the device";
        let args = shell_byte_channel_args(serial, words)?;
        let invocation = command_string(self.path(), &args);
        let output = bounded_adb(
            host.output_with_input(self.path(), &args, input),
            operation,
            invocation.clone(),
            timeout,
        )
        .await?;
        Ok(checked_stdout(&output, operation, &invocation)?)
    }

    /// The `versionCode` the device reports for `package`, `None` when it is
    /// not installed — parsed from `pm list packages --show-versioncode`.
    ///
    /// # Errors
    /// Returns an error if the query fails or its output names `package` in
    /// a line that does not parse — silently treating a malformed line as
    /// "not installed" would reinstall over a working host.
    pub(crate) async fn installed_version_code(
        &self,
        host: &Host,
        serial: &str,
        package: &str,
        timeout: Duration,
    ) -> eyre::Result<Option<u32>> {
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
        parse_installed_version_code(&output, package)
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

    /// `adb -s <serial> forward --list` — this serial's registrations.
    ///
    /// # Errors
    /// Returns an error if the list fails or a line does not parse — a
    /// garbled line silently dropped could hide the forward a caller is
    /// looking for.
    pub(crate) async fn forwards(
        &self,
        host: &Host,
        serial: &str,
        timeout: Duration,
    ) -> eyre::Result<Vec<Forward>> {
        let output = self
            .device_command(
                host,
                serial,
                [OsStr::new("forward"), OsStr::new("--list")],
                "listing adb forwards",
                timeout,
            )
            .await?;
        output
            .lines()
            .filter_map(|line| parse_forward_line(serial, line).transpose())
            .collect()
    }

    /// `adb -s <serial> logcat -T <since> -m 1 -e <pattern> -s <tag>:*` —
    /// block until `tag` logs a line newer than `since` that matches
    /// `pattern`, answering that line. `logcat` exits on the match itself,
    /// so the wait ends on the device's own signal; `timeout` bounds a
    /// device that never logs it.
    ///
    /// # Errors
    /// Returns an error if `logcat` fails, exceeds `timeout`, or exits
    /// without printing a line.
    pub(crate) async fn first_log_line(
        &self,
        host: &Host,
        serial: &str,
        since: &str,
        tag: &str,
        pattern: &str,
        timeout: Duration,
    ) -> eyre::Result<String> {
        let filter = format!("{tag}:*");
        let output = self
            .device_command(
                host,
                serial,
                [
                    "logcat", "-T", since, "-m", "1", "-e", pattern, "-s", &filter,
                ],
                "waiting for a logcat line",
                timeout,
            )
            .await?;
        output
            .lines()
            .find(|line| !line.trim().is_empty() && !line.starts_with("--------- "))
            .map(str::to_string)
            .ok_or_else(|| eyre::eyre!("`logcat` exited without a line matching {pattern:?}"))
    }

    /// `adb -s <serial> forward tcp:0 <remote>` — bind a fresh local TCP
    /// port to `remote`, answering the port adb chose.
    ///
    /// The registration lives on the adb server, so it survives the client
    /// that created it: a later run's `forward --list` sees it again.
    ///
    /// # Errors
    /// Returns an error if the command fails or answers something that is
    /// not a port.
    pub(crate) async fn forward(
        &self,
        host: &Host,
        serial: &str,
        remote: &str,
        timeout: Duration,
    ) -> eyre::Result<u16> {
        let output = self
            .device_command(
                host,
                serial,
                [
                    OsStr::new("forward"),
                    OsStr::new("tcp:0"),
                    OsStr::new(remote),
                ],
                "creating an adb forward",
                timeout,
            )
            .await?;
        output.trim().parse::<u16>().map_err(|error| {
            eyre::eyre!(
                "`adb forward` answered {output:?} where a local port was expected: {error}"
            )
        })
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
        let (output, command) = self
            .device_output(host, serial, args, operation, timeout)
            .await?;
        checked_stdout(&output, operation, &command)
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
    let invocation = command_string(adb.path(), &args);
    bounded_adb(
        host.output(adb.path(), &args),
        operation,
        invocation,
        timeout,
    )
    .await
}

/// Bound one spawned `adb` invocation by `timeout`. The command future
/// owns a `kill_on_drop` child, so the deadline firing drops it and leaves
/// no orphan.
async fn bounded_adb(
    command: impl Future<Output = Result<Output, CommandError>>,
    operation: &str,
    invocation: String,
    timeout: Duration,
) -> Result<Output, AdbCommandError> {
    let operation = operation.to_owned();
    let command = Box::pin(async move { command.await.map_err(AdbCommandError::from) });
    let seconds = timeout.as_secs();
    let timeout = Box::pin(async move {
        smol::Timer::after(timeout).await;
        Err(AdbCommandError::Timeout {
            operation,
            seconds,
            command: invocation,
        })
    });

    match select(command, timeout).await {
        Either::Left((result, _)) | Either::Right((result, _)) => result,
    }
}

/// `-s <serial> shell -T <words>`: the command is one `shlex`-quoted shell
/// word, and `-T` pins the no-PTY byte channel a binary read or a fed stdin
/// needs.
fn shell_byte_channel_args(serial: &str, words: &[&str]) -> eyre::Result<Vec<OsString>> {
    let joined = shlex::try_join(words.iter().copied())
        .map_err(|error| eyre::eyre!("cannot quote the shell words {words:?}: {error}"))?;
    Ok(["-s", serial, "shell", "-T", joined.as_str()]
        .into_iter()
        .map(OsString::from)
        .collect())
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
    let args = args
        .into_iter()
        .map(|argument| argument.as_ref().to_os_string())
        .collect::<Vec<_>>();
    let command = command_string(adb.path(), &args);
    let output = run_bounded_adb_output(host, adb, args, operation, timeout).await?;
    checked_stdout(&output, operation, &command)
}

/// [`check_bounded_output`] answering the stdout text of a successful
/// invocation.
fn checked_stdout(
    output: &Output,
    operation: &str,
    command: &str,
) -> Result<String, AdbCommandError> {
    check_bounded_output(output, operation, command)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// One line naming the invocation an [`AdbCommandError`] reports.
fn command_string(adb: &Path, args: &[OsString]) -> String {
    let mut command = adb.display().to_string();
    for arg in args {
        command.push(' ');
        command.push_str(&arg.to_string_lossy());
    }
    command
}

/// The non-zero-status branch shared by every bounded adb call —
/// [`run_bounded_adb_command`], [`Adb::device_command`], [`Adb::shell_run`]
/// and [`Adb::run_as_cat`] — so a failure carries one wording and one error
/// type no matter which verb produced it.
fn check_bounded_output(
    output: &Output,
    operation: &str,
    command: &str,
) -> Result<(), AdbCommandError> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut details = String::new();
    if !stderr.is_empty() {
        details.push_str("\nstderr:\n");
        details.push_str(&stderr);
    }
    if !stdout.is_empty() {
        details.push_str("\nstdout:\n");
        details.push_str(&stdout);
    }
    Err(AdbCommandError::Failed {
        operation: operation.to_owned(),
        status: output.status,
        command: command.to_owned(),
        details,
    })
}

/// The recent crash evidence `logcat` holds for a launch or preview that
/// produced no output — the AndroidRuntime/DEBUG/WaterUI tags plus the
/// `hydrolysis` tag the preview entry writes under and the
/// `HydrolysisPreview` tag the instrumentation logs failures with.
///
/// `since` bounds the dump to lines from this run — a `logcat -T` time spec
/// (`MM-DD HH:MM:SS.mmm`) captured on the device before the run started.
/// `None` bounds by the last 100 lines instead.
///
/// # Errors
/// Never fails: a `logcat` that itself errors answers the failure text, so
/// the caller's diagnostic is always complete.
pub(crate) async fn recent_crash_log(
    host: &Host,
    adb: &Adb,
    serial: &str,
    since: Option<&str>,
) -> String {
    let mut args = vec!["-s", serial, "logcat", "-d"];
    match since {
        Some(time) => args.extend(["-T", time]),
        None => args.extend(["-t", "100"]),
    }
    args.extend([
        "-s",
        "AndroidRuntime:E",
        "DEBUG:*",
        "WaterUI:*",
        "hydrolysis:*",
        "HydrolysisPreview:*",
    ]);
    match run_bounded_adb_command(
        host,
        adb,
        args,
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
/// `package:<package> versionCode:<n>`. A missing package answers `None`; a
/// line that names `package` without a well-formed `versionCode` is a
/// malformed response — an error, not "not installed".
fn parse_installed_version_code(output: &str, package: &str) -> eyre::Result<Option<u32>> {
    for line in output.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("package:") else {
            continue;
        };
        // A bare `package:<name>` carries no versionCode field at all —
        // malformed, and below it errors rather than skipping.
        let (name, version) = rest.split_once(' ').unwrap_or((rest, ""));
        if name != package {
            continue;
        }
        return version
            .strip_prefix("versionCode:")
            .and_then(|code| code.parse::<u32>().ok())
            .map(Some)
            .ok_or_else(|| {
                eyre::eyre!(
                    "malformed `pm list packages --show-versioncode` line for {package}: {trimmed}"
                )
            });
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;

    use super::{Adb, AdbError, parse_forward_line, parse_installed_version_code};
    use crate::toolchain::testing::TestMachine;

    /// `forward --list` names every device's forwards: a line for another
    /// serial is skipped, one for this serial parses, and a line of another
    /// shape is an error naming it.
    #[test]
    fn forward_lines_parse_for_their_serial_only() {
        let forward = parse_forward_line("a", "a tcp:4567 localabstract:x")
            .expect("parses")
            .expect("this serial's forward");
        assert_eq!(
            (forward.local_port, forward.remote.as_str()),
            (4567, "localabstract:x")
        );
        assert!(
            parse_forward_line("a", "b tcp:4567 localabstract:x")
                .expect("parses")
                .is_none()
        );
        for line in [
            "a tcp:4567",
            "a tcp:4567 localabstract:x extra",
            "a udp:4567 localabstract:x",
        ] {
            let error = parse_forward_line("a", line).expect_err(line);
            assert!(error.to_string().contains(line), "{error}");
        }
    }

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
            )
            .expect("the line parses"),
            Some(42)
        );
        // A package that is not installed yields no line at all.
        assert_eq!(
            parse_installed_version_code("", "dev.waterui.hydrolysis.preview")
                .expect("an absent package is not an error"),
            None
        );
        // Another package's line is not this package's.
        assert_eq!(
            parse_installed_version_code(
                "package:dev.waterui.other versionCode:7\n",
                "dev.waterui.hydrolysis.preview"
            )
            .expect("an unrelated line parses"),
            None
        );
    }

    /// A line that names the package without a well-formed `versionCode` is
    /// malformed output — an error, never "not installed".
    #[test]
    fn a_malformed_version_code_line_is_an_error() {
        for line in [
            "package:dev.waterui.hydrolysis.preview versionCode:NaN\n",
            "package:dev.waterui.hydrolysis.preview\n",
            "package:dev.waterui.hydrolysis.preview versionCode:\n",
        ] {
            let error = parse_installed_version_code(line, "dev.waterui.hydrolysis.preview")
                .expect_err("a malformed line must error");
            assert!(error.to_string().contains("malformed"), "{error}");
        }
    }

    /// `push`, the fed `shell -T` and the `shell -T` read-back log their
    /// argv through the fake — paths with spaces travel inside one quoted
    /// shell word — and the fed input reaches the remote command's stdin.
    #[test]
    #[cfg(unix)]
    fn push_fed_shell_and_cat_log_their_invocations() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let received = machine.root().join("received.bin");
        let host = machine.host([
            (
                OsString::from("ANDROID_SDK_ROOT"),
                sdk.as_os_str().to_os_string(),
            ),
            (
                OsString::from("WATERUI_FAKE_ADB_LOG"),
                log.as_os_str().to_os_string(),
            ),
            (
                OsString::from("WATERUI_FAKE_ADB_STDIN"),
                received.as_os_str().to_os_string(),
            ),
        ]);
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb");
        // More than one feed chunk, so the input spans several writes.
        let input: Vec<u8> = (0..200_000u32)
            .map(|index| index.to_le_bytes()[0])
            .collect();

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
            adb.shell_with_input(
                &host,
                "serial",
                &[
                    "run-as",
                    "dev.waterui.hydrolysis.preview",
                    "sh",
                    "-c",
                    "cat > \"$1\"",
                    "sh",
                    "files/payload with space",
                ],
                input.as_slice(),
                Duration::from_secs(5),
            )
            .await
            .expect("the fake adb accepts the input");
            std::fs::create_dir_all(machine.responses()).expect("responses dir");
            std::fs::write(
                machine.responses().join("ADB_CAT"),
                b"\x89PNG\r\n\x1a\nbytes",
            )
            .expect("stage canned cat");
            adb.run_as_cat(
                &host,
                "serial",
                "dev.waterui.hydrolysis.preview",
                "files/payload/out.png",
                Duration::from_secs(5),
            )
            .await
            .expect("the fake adb reads back");
        });

        let argv = std::fs::read_to_string(&log).expect("read the argv log");
        assert!(
            argv.lines()
                .any(|line| line
                    == "-s serial push dir with space/lib /data/local/tmp/waterui-preview"),
            "the push invocation logged: {argv}"
        );
        assert!(
            argv.contains(
                "-s serial shell -T run-as dev.waterui.hydrolysis.preview sh -c 'cat > \"$1\"' sh \
                 'files/payload with space'"
            ),
            "the fed shell invocation logged: {argv}"
        );
        assert!(
            argv.contains(
                "-s serial shell -T run-as dev.waterui.hydrolysis.preview cat files/payload/out.png"
            ),
            "the shell -T cat invocation logged: {argv}"
        );
        assert_eq!(
            std::fs::read(&received).expect("the input landed"),
            input,
            "the remote command read the whole input"
        );
    }

    /// A non-zero `run-as` carries its status into the error.
    #[test]
    #[cfg(unix)]
    fn a_failing_run_as_reports_its_status() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let host = machine.host([
            (
                OsString::from("ANDROID_SDK_ROOT"),
                sdk.as_os_str().to_os_string(),
            ),
            (
                OsString::from("WATERUI_FAKE_ADB_RUN_AS_STATUS"),
                OsString::from("7"),
            ),
        ]);
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb");
        let error = smol::block_on(adb.shell_with_input(
            &host,
            "serial",
            &[
                "run-as",
                "dev.waterui.hydrolysis.preview",
                "sh",
                "-c",
                "cat > config",
            ],
            &b"config"[..],
            Duration::from_secs(5),
        ))
        .expect_err("a non-zero run-as must fail");
        let message = error.to_string();
        assert!(message.contains("status"), "{message}");
    }

    /// A wedged `adb` surfaces as a timeout naming the invocation. The
    /// hanging host must not be the one `locate` runs under — its
    /// `start-server` would hang too.
    #[test]
    #[cfg(unix)]
    fn a_hanging_adb_times_out_with_its_argv() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let host = machine.host([(
            OsString::from("ANDROID_SDK_ROOT"),
            sdk.as_os_str().to_os_string(),
        )]);
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb");
        let wedged = machine.host([(OsString::from("WATERUI_FAKE_ADB_HANG"), OsString::from("1"))]);
        let error = smol::block_on(adb.shell_run(
            &wedged,
            "serial",
            &["getprop", "ro.build.id"],
            Duration::from_millis(200),
        ))
        .expect_err("a wedged adb must time out");
        let message = error.to_string();
        assert!(
            message.contains("timed out") && message.contains("getprop"),
            "{message}"
        );
    }
}
