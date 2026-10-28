//! The `water test-runner` plumbing: cargo/nextest target runners for the
//! Apple destinations.
//!
//! Each destination runs one test binary per invocation. A `--list`
//! invocation or a binary without a `__TEXT,__info_plist` section takes the
//! bare route where one exists; a marked binary is staged as `<binary>.app`
//! (its `Info.plist` is the embedded manifest verbatim), signed for the
//! destination, installed, and launched one case at a time. The launched
//! app's stdout reaches the runner's stdout line-for-line — nextest parses
//! it — and its stderr the runner's stderr; every diagnostic of the runner
//! itself goes to stderr. The process exits with the status the harness
//! wrote to `WATERUI_NATIVE_TEST_STATUS`, never the launcher's exit code.
//!
//! # Device cleanup
//!
//! The device destination leaves the bundle installed between cases and
//! between runs — an uninstall would turn every launch into a cold
//! install. The bundle stays installed at the end of a session and must
//! never be uninstalled: its identifier is shared, so removing it would
//! delete another project's app.

pub(crate) mod binary;
mod bundle;
mod device;
mod lock;
mod simulator;
mod status;

use std::ffi::OsString;
use std::io::Write;

use eyre::Context as _;
use smol::stream::StreamExt as _;

use crate::device::{DeviceEvent, Running};
use crate::toolchain::Host;

pub use binary::{HarnessManifest, MachOPlatform, TestBinary};
pub use bundle::HarnessBundle;
pub use device::{DeviceIdentity, IosDevice};
pub use lock::{DestinationLock, LOCK_BOUND};
pub use simulator::IosSimulator;
pub use status::{StatusFile, TestStatus};

/// The environment variable naming the file the harness writes its exit
/// status to.
pub const STATUS_PATH_VARIABLE: &str = "WATERUI_NATIVE_TEST_STATUS";

/// The arguments cargo forwarded, and whether they ask for `--list`.
#[derive(Debug)]
pub struct TestArguments(Vec<OsString>);

impl TestArguments {
    /// Wraps the argv tail.
    #[must_use]
    pub const fn new(arguments: Vec<OsString>) -> Self {
        Self(arguments)
    }

    /// Whether the invocation is a nextest/libtest `--list`.
    #[must_use]
    pub fn is_listing(&self) -> bool {
        self.0.iter().any(|argument| argument == "--list")
    }

    /// The arguments verbatim.
    #[must_use]
    pub fn as_slice(&self) -> &[OsString] {
        &self.0
    }
}

/// Caller variables the runner forwards into the test process, by name.
///
/// Today that is only `WATERUI_REFERENCE_METRICS`; each destination
/// applies its own prefix (`SIMCTL_CHILD_`, `DEVICECTL_CHILD_`).
#[derive(Debug)]
pub struct ForwardedEnv(Vec<(String, String)>);

impl ForwardedEnv {
    /// The variable a run forwards when the caller set it.
    pub const REFERENCE_METRICS: &'static str = "WATERUI_REFERENCE_METRICS";

    /// Builds the set from `(name, value)` pairs.
    #[must_use]
    pub fn new(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        Self(vars.into_iter().collect())
    }

    /// The `(name, value)` pairs.
    #[must_use]
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }
}

/// Everything one `run` works on: the binary, its arguments, and the
/// caller environment forwarded into the harness.
#[derive(Debug)]
#[expect(
    clippy::exhaustive_structs,
    reason = "the terminal constructs it from the parsed invocation; fields are complete by definition"
)]
pub struct Invocation {
    /// The test binary to run.
    pub binary: TestBinary,
    /// The arguments after the binary path.
    pub arguments: TestArguments,
    /// Caller environment forwarded into the harness.
    pub forwarded_env: ForwardedEnv,
}

/// What every destination implements.
pub trait TestDestination {
    /// The Mach-O platform this destination runs; any other is an error.
    fn platform(&self) -> MachOPlatform;

    /// Runs one invocation, writing the test's stdout to `stdout` and
    /// diagnostics to `stderr`, and returns the harness status.
    fn run(
        &self,
        host: &Host,
        invocation: &Invocation,
        interrupts: smol::channel::Receiver<()>,
        stdout: &mut (impl Write + Send + ?Sized),
        stderr: &mut (impl Write + Send + ?Sized),
    ) -> impl Future<Output = eyre::Result<TestStatus>> + Send;
}

/// Object-safe front of [`TestDestination`] for the enum-dispatched
/// destination; the terminal only parses and prints.
pub trait Run {
    /// See [`TestDestination::run`].
    fn run<'a>(
        &'a self,
        host: &'a Host,
        invocation: &'a Invocation,
        interrupts: smol::channel::Receiver<()>,
        stdout: &'a mut (dyn Write + Send + 'a),
        stderr: &'a mut (dyn Write + Send + 'a),
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = eyre::Result<TestStatus>> + Send + 'a>>;
}

impl<T: TestDestination> Run for T {
    fn run<'a>(
        &'a self,
        host: &'a Host,
        invocation: &'a Invocation,
        interrupts: smol::channel::Receiver<()>,
        stdout: &'a mut (dyn Write + Send + 'a),
        stderr: &'a mut (dyn Write + Send + 'a),
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = eyre::Result<TestStatus>> + Send + 'a>>
    {
        Box::pin(TestDestination::run(
            self, host, invocation, interrupts, stdout, stderr,
        ))
    }
}

/// Consumes a run's supervised events: the app's stdout lines to `out`,
/// everything else to `err`, until the terminal event. Interrupts reach
/// the monitor through [`Running::supervise`], so a SIGTERM terminates the
/// launched app on the destination rather than leaving it running.
async fn forward_run(
    running: Running,
    interrupts: smol::channel::Receiver<()>,
    out: &mut (impl Write + Send + ?Sized),
    err: &mut (impl Write + Send + ?Sized),
) -> eyre::Result<()> {
    let mut events = std::pin::pin!(running.supervise(interrupts));
    while let Some(event) = events.next().await {
        match event {
            DeviceEvent::Stdout { message } | DeviceEvent::Log { message, .. } => {
                writeln!(out, "{message}").wrap_err("failed to forward the test's stdout")?;
            }
            DeviceEvent::Stderr { message } | DeviceEvent::MonitorError { message } => {
                writeln!(err, "{message}").wrap_err("failed to write runner diagnostics")?;
            }
            DeviceEvent::Started
            | DeviceEvent::Stopped
            | DeviceEvent::Exited(_)
            | DeviceEvent::Crashed(_) => {}
        }
    }
    out.flush()?;
    err.flush()?;
    Ok(())
}
