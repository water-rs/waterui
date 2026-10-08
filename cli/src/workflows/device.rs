//! Device management and application running utilities for `WaterUI` CLI.

use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
    pin::Pin,
};

use smol::{
    channel::{Receiver, Sender, unbounded},
    stream::Stream,
};

use crate::{debug::CrashReport, toolchain::Host};

#[cfg(all(test, unix))]
pub(crate) mod test_support {
    use std::os::unix::fs::PermissionsExt as _;

    use crate::toolchain::testing::TestMachine;

    #[derive(Clone, Default)]
    pub struct ProcessGroupGuard(std::sync::Arc<std::sync::Mutex<Option<nix::unistd::Pid>>>);

    impl ProcessGroupGuard {
        pub fn set(&self, pid: nix::unistd::Pid) {
            *self.0.lock().expect("process-group guard lock") = Some(pid);
        }
    }

    impl Drop for ProcessGroupGuard {
        fn drop(&mut self) {
            let pgid = *self.0.lock().expect("process-group guard lock");
            if let Some(pgid) = pgid {
                match nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL) {
                    Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                    Err(error) => {
                        tracing::error!("Failed to kill fixture process group: {error}");
                    }
                }
            }
        }
    }

    pub fn term_ignoring_fixture(machine: &TestMachine) -> std::path::PathBuf {
        let fifo = machine.root().join("stop-fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU)
            .expect("mkfifo the fixture's blocking fifo");
        let script = machine.file(
            "ignore-term",
            &format!(
                "#!/bin/sh\n\
                 echo pid=$$ >&2\n\
                 trap 'echo term-seen >&2' TERM\n\
                 echo trap-armed >&2\n\
                 while ! IFS= read -r _done < \"{}\"; do :; done\n",
                fifo.display()
            ),
        );
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");
        script
    }
}

#[cfg(target_os = "macos")]
use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

/// The environment variable that carries the log level to the launched application.
///
/// the generated crate reads it when it installs `tracing`: the CLI names the level
/// here and the runtime composes its own filter around it.
const LOG_LEVEL_ENV: &str = "WATERUI_LOG";

/// The longest a device monitor waits for the app to exit after a stop request.
pub(crate) const STOPPED_EXIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// The longest the post-exit log query for a panic may take.
pub(crate) const PANIC_LOG_QUERY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Minimum log level for streaming device logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Only errors
    Error,
    /// Warnings and errors
    Warn,
    /// Info, warnings, and errors
    #[default]
    Info,
    /// Debug and above
    Debug,
    /// All logs including verbose
    Verbose,
}

impl LogLevel {
    /// Convert to Android logcat priority character.
    #[must_use]
    pub const fn to_android_priority(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warn => 'W',
            Self::Info => 'I',
            Self::Debug => 'D',
            Self::Verbose => 'V',
        }
    }

    /// Convert to iOS/macOS `log stream --level` argument.
    ///
    /// Apple's unified logging `log stream --level` accepts: default, info, debug
    /// - `debug` includes all messages (debug, info, default, error, fault)
    /// - `info` includes info and above
    /// - `default` includes default (notice) and above
    ///
    /// Since we want to capture errors/warnings, we need at least `default` level.
    #[must_use]
    pub const fn to_apple_level(self) -> &'static str {
        match self {
            Self::Error | Self::Warn | Self::Info => "default",
            Self::Debug | Self::Verbose => "debug",
        }
    }

    /// The `tracing` level the launched application logs at for this setting.
    ///
    /// Streaming is only half of `--logs`: the runtime records nothing above
    /// `error` unless it is told a level, so the same choice travels to the
    /// process and decides what it emits in the first place.
    #[must_use]
    pub const fn to_tracing_level(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Verbose => "trace",
        }
    }
}

/// Options for running an application on a device
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// # Note
    ///
    /// Android does not support environment variables yet.
    /// `iOS`/`macOS` support environment variables via `export SIMCTL_CHILD_KEY=Val`.
    ///
    /// As a workaround, on Android we pass values as Activity intent extras using the
    /// `waterui.env.<KEY>` namespace, and the app reads them on startup and calls `Os.setenv()`.
    env_vars: HashMap<String, String>,

    /// If set, stream device logs at or above this level.
    log_level: Option<LogLevel>,

    /// If true, stream all native platform logs (`NSLog`, `print`, etc.), not just `WaterUI` logs.
    /// This filters by process ID instead of subsystem, which is noisier but includes all output.
    native_logs: bool,

    /// If true, terminate existing local macOS app instances for the same executable before
    /// launching a new one. Preview support apps must disable this so multiple pooled instances
    /// can coexist across runtime fingerprints.
    replace_existing_macos_app_instances: bool,

    /// TCP ports to forward from the host loopback to the device's loopback
    /// (`adb forward`) for the lifetime of the run.
    ///
    /// Only Android honors this: the preview support app binds its TCP server
    /// to the device's loopback, which the host cannot reach otherwise. The
    /// mappings are removed when the [`Running`] is dropped, unless it is
    /// detached — a detached preview app keeps serving future sessions through
    /// the same ports.
    forward_tcp_ports: Vec<u16>,

    /// File the launched app's stdout and stderr append to instead of pipes.
    ///
    /// A pipe is only as alive as the reader at its other end: a support app
    /// designed to outlive the CLI (the preview support app detaches and keeps
    /// serving the next `water preview`) would keep writing to a pipe whose
    /// reader is gone — on unix that write raises SIGPIPE and kills the app
    /// (water-rs/cli#197). A file is the right channel for a process that
    /// outlives the command; the launch still follows it into [`DeviceEvent::Log`]
    /// events while the command runs.
    app_log_file: Option<PathBuf>,
}

impl RunOptions {
    /// Create new run options
    #[must_use]
    pub fn new() -> Self {
        Self {
            env_vars: HashMap::new(),
            log_level: None,
            native_logs: false,
            replace_existing_macos_app_instances: true,
            forward_tcp_ports: Vec::new(),
            app_log_file: None,
        }
    }

    /// Insert an environment variable to be set when running the application
    pub fn insert_env_var(&mut self, key: String, value: String) {
        self.env_vars.insert(key, value);
    }

    /// Tells the application which project it came from and what it is called.
    ///
    /// A launched application knows neither. It has no working directory worth
    /// the name — a macOS bundle gets `/` — so nothing it starts on the
    /// developer's behalf could find the project, and nothing inside `WaterUI`
    /// knows the name the project gave itself, which is why a window with no
    /// title of its own has to be told what to fall back to.
    pub fn describe_project(&mut self, project: &crate::project::Project) {
        self.insert_env_var(
            String::from("WATERUI_PROJECT_DIR"),
            project.root().display().to_string(),
        );
        let name = project.manifest().package.name.clone();
        self.insert_env_var(String::from("WATERUI_APP_NAME"), name);
    }

    /// Get an iterator over the environment variables
    pub fn env_vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Set the minimum log level to stream, and have the application log at it.
    ///
    /// The level reaches the process through the `WATERUI_LOG` variable on every launch
    /// path, since each of them forwards [`Self::env_vars`].
    pub fn set_log_level(&mut self, level: LogLevel) {
        self.log_level = Some(level);
        self.insert_env_var(
            String::from(LOG_LEVEL_ENV),
            String::from(level.to_tracing_level()),
        );
    }

    /// Get the log level if set.
    #[must_use]
    pub const fn log_level(&self) -> Option<LogLevel> {
        self.log_level
    }

    /// Set whether to stream all native platform logs.
    pub const fn set_native_logs(&mut self, native_logs: bool) {
        self.native_logs = native_logs;
    }

    /// Get whether native logs are enabled.
    #[must_use]
    pub const fn native_logs(&self) -> bool {
        self.native_logs
    }

    /// Set whether launching a local macOS `.app` should replace existing instances of the same
    /// executable.
    pub const fn set_replace_existing_macos_app_instances(&mut self, replace: bool) {
        self.replace_existing_macos_app_instances = replace;
    }

    /// Get whether launching a local macOS `.app` should replace existing instances.
    #[must_use]
    pub const fn replace_existing_macos_app_instances(&self) -> bool {
        self.replace_existing_macos_app_instances
    }

    /// Send the launched app's stdout and stderr to `path` (appended, created
    /// with parents) instead of pipes. Use for a process designed to outlive
    /// this command — a pipe whose reader has exited raises SIGPIPE on unix.
    pub fn set_app_log_file(&mut self, path: PathBuf) {
        self.app_log_file = Some(path);
    }

    /// The file the launched app's stdout and stderr are redirected to, if any.
    #[must_use]
    pub fn app_log_file(&self) -> Option<&Path> {
        self.app_log_file.as_deref()
    }

    /// Forward the given TCP ports from the host loopback to the device's
    /// loopback for the lifetime of the run.
    pub fn set_forward_tcp_ports(&mut self, ports: impl IntoIterator<Item = u16>) {
        self.forward_tcp_ports = ports.into_iter().collect();
    }

    /// The TCP ports to forward to the device's loopback, if any.
    #[must_use]
    pub fn forward_tcp_ports(&self) -> &[u16] {
        &self.forward_tcp_ports
    }
}

/// Represents a build artifact to be run on a device
#[derive(Debug)]
pub struct Artifact {
    bundle_id: String,
    path: PathBuf,
}

impl Artifact {
    /// Create a new artifact
    #[must_use]
    pub fn new(bundle_id: impl Into<String>, path: PathBuf) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            path,
        }
    }

    /// Get the bundle identifier of the artifact
    #[must_use]
    pub const fn bundle_id(&self) -> &str {
        self.bundle_id.as_str()
    }

    /// Get the path to the artifact
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Trait representing a device (e.g., emulator, simulator, physical device)
///
/// Devices are decoupled from platforms - a device just knows how to execute artifacts.
/// The same device can be used with different backends (e.g., Local device works with
/// both Apple and GTK4 backends on macOS).
///
/// Each device type knows how to scan for available devices of its kind via the
/// associated `scan()` function.
pub trait Device: Sized + Send {
    /// Human-readable name for display purposes.
    fn name(&self) -> &str;

    /// Launch the device emulator or simulator.
    ///
    /// If the device is a physical device or local machine, this should do nothing.
    fn launch(&self, host: &Host) -> impl Future<Output = eyre::Result<()>> + Send;

    /// Run the given artifact on the device with the specified options.
    fn run(
        &self,
        host: &Host,
        artifact: Artifact,
        options: RunOptions,
    ) -> impl Future<Output = Result<Running, FailToRun>> + Send;

    /// Scan for available devices of this type on `host`.
    ///
    /// Each device type knows how to discover its own kind:
    /// - `Local::scan()` → always returns `vec![Local]`
    /// - `AppleSimulator::scan()` → uses `simctl list`
    /// - `AndroidDevice::scan()` → uses `adb devices`
    fn scan(host: &Host) -> impl Future<Output = eyre::Result<Vec<Self>>> + Send;

    /// The hardware UDID a package built for this device must be
    /// provisioned for — a physical Apple device returns its UDID so the
    /// development provisioning profile can be bound to it; simulators,
    /// emulators and local targets need no device-bound profile and return
    /// `None`.
    fn device_udid(&self) -> Option<&str> {
        None
    }
}

/// Represents a running application on a device.
///
/// The run ends through its device monitor, the one owner of stop timing: the
/// monitor bounds its stop and kill waits and the output join after every
/// exit, so its terminal event always arrives. Owners end a run with
/// [`Running::supervise`] or [`Running::shutdown`], which await that event.
/// Dropping a run that has not ended only queues a kill without waiting for
/// it and logs an error: the run's retained resources are released before
/// the monitor confirms the app is gone.
pub struct Running {
    receiver: Pin<Box<Receiver<DeviceEvent>>>,
    control: Option<Sender<StopRequest>>,
    ended: bool,
    on_drop: Vec<Box<dyn FnOnce() + Send>>,
}

/// What a run's supervisor asks the device monitor to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRequest {
    /// Send one termination signal, then wait for the monitor's grace bound.
    Terminate,
    /// End the app now.
    Kill,
}

impl Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running").finish_non_exhaustive()
    }
}

impl Running {
    /// Create a new `Running` instance and its event and control channels.
    #[must_use]
    pub fn new() -> (Self, Sender<DeviceEvent>, Receiver<StopRequest>) {
        let (sender, receiver) = unbounded();
        let _ = sender.try_send(DeviceEvent::Started);
        let (control, control_receiver) = unbounded();
        (
            Self {
                receiver: Box::pin(receiver),
                control: Some(control),
                ended: false,
                on_drop: Vec::new(),
            },
            sender,
            control_receiver,
        )
    }

    /// Ask the monitor to stop the app; returns whether the request was
    /// delivered.
    fn request(&self, request: StopRequest) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.try_send(request).is_ok())
    }

    /// Ask the monitor to stop the app and wait for its terminal event.
    ///
    /// There is no timeout here: the monitor bounds every step of its stop
    /// and kill paths and the output join after any exit, and always sends
    /// the terminal event. Events that arrive meanwhile are discarded; a run
    /// that already ended returns at once.
    pub async fn shutdown(mut self, request: StopRequest) {
        use smol::stream::StreamExt as _;

        if self.ended {
            return;
        }
        self.request(request);
        while let Some(event) = self.next().await {
            if let DeviceEvent::MonitorError { message } = &event {
                tracing::error!("{message}");
            }
            if event.is_terminal() {
                break;
            }
        }
    }

    /// Consume this run into a stream of its device events that owns the
    /// CLI's stop policy.
    ///
    /// - The first `interrupts` message asks the monitor to terminate the app.
    /// - A second message asks the monitor to kill it, while the stream keeps
    ///   yielding events until the monitor acknowledges the end.
    /// - After a terminal event the stream still yields whatever the
    ///   forwarders already queued behind it, so shutdown output written
    ///   alongside the exit is not lost; then the stream ends.
    ///
    /// The returned stream replaces any per-command select between the
    /// events and the interrupt channel — callers only print events.
    pub fn supervise(self, interrupts: Receiver<()>) -> impl Stream<Item = DeviceEvent> {
        use futures_util::future::{Either, select};
        use futures_util::stream::unfold;
        use smol::stream::StreamExt as _;

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Stop {
            Running,
            Terminating,
            Killing,
        }

        enum Next {
            Event(Option<DeviceEvent>),
            Interrupt(Result<(), smol::channel::RecvError>),
        }

        unfold(
            (self, Some(interrupts), Stop::Running, false),
            |(mut running, mut interrupts, mut stop, mut drained)| async move {
                loop {
                    let next = if drained {
                        match running.receiver.try_recv() {
                            Ok(event) => Next::Event(Some(event)),
                            Err(_) => return None,
                        }
                    } else if let Some(interrupt_receiver) = &interrupts {
                        let events = std::pin::pin!(running.next());
                        let interrupt = std::pin::pin!(interrupt_receiver.recv());
                        match select(events, interrupt).await {
                            Either::Left((event, _)) => Next::Event(event),
                            Either::Right((result, _)) => Next::Interrupt(result),
                        }
                    } else {
                        Next::Event(running.next().await)
                    };

                    match next {
                        Next::Interrupt(Err(_)) => interrupts = None,
                        Next::Interrupt(Ok(())) => match stop {
                            Stop::Running => {
                                if running.request(StopRequest::Terminate) {
                                    stop = Stop::Terminating;
                                }
                            }
                            Stop::Terminating => {
                                if running.request(StopRequest::Kill) {
                                    stop = Stop::Killing;
                                }
                            }
                            Stop::Killing => {}
                        },
                        Next::Event(event) => {
                            let terminal = event.as_ref().is_none_or(DeviceEvent::is_terminal);
                            let event = event.map(|event| {
                                if stop == Stop::Running {
                                    event
                                } else {
                                    match event {
                                        DeviceEvent::Exited(_) => DeviceEvent::Stopped,
                                        DeviceEvent::Crashed(crash)
                                            if crash.cause.ends_a_requested_stop() =>
                                        {
                                            DeviceEvent::Stopped
                                        }
                                        other => other,
                                    }
                                }
                            });
                            if terminal {
                                drained = true;
                            }
                            if let Some(event) = event {
                                return Some((event, (running, interrupts, stop, drained)));
                            }
                        }
                    }
                }
            },
        )
    }

    /// Retain a value for the lifetime of the `Running` instance.
    pub fn retain<T: Send + 'static>(&mut self, value: T) {
        self.on_drop.push(Box::new(move || {
            drop(value);
        }));
    }

    /// Detach the running instance, preventing the app from being killed on drop.
    ///
    /// This is useful for long-running apps like the preview support app that should
    /// stay running after the CLI command completes.
    pub fn detach(self: Pin<&mut Self>) {
        let this = self.get_mut();
        // Detach keeps every retained resource alive, so the hooks are
        // forgotten rather than dropped: dropping a retained RAII guard (like
        // the `adb forward` teardown) fires its `Drop` here, which is exactly
        // the cleanup detach exists to prevent. The control sender is forgotten
        // the same way — a detached app keeps running.
        for hook in this.on_drop.drain(..) {
            std::mem::forget(hook);
        }
        if let Some(control) = this.control.take() {
            std::mem::forget(control);
        }
    }
}

impl Stream for Running {
    type Item = DeviceEvent;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let result = this.receiver.as_mut().poll_next(cx);
        if matches!(&result, std::task::Poll::Ready(Some(event)) if event.is_terminal())
            || matches!(result, std::task::Poll::Ready(None))
        {
            this.ended = true;
        }
        result
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.ended && self.request(StopRequest::Kill) && !std::thread::panicking() {
            tracing::error!(
                "A running app was dropped without shutdown; its kill was requested but not awaited"
            );
        }
        for hook in self.on_drop.drain(..) {
            hook();
        }
    }
}

/// Errors that can occur when running an application on a device
#[derive(Debug, thiserror::Error)]
pub enum FailToRun {
    /// Invalid artifact provided.
    #[error("Invalid artifact")]
    InvalidArtifact,

    /// Failed to install the application on the device.
    #[error("Failed to install application on device: {0}")]
    Install(eyre::Report),

    /// Failed to launch the device.
    #[error("Failed to launch device: {0}")]
    Launch(eyre::Report),
    /// Failed to run the application on the device.
    #[error("Failed to run application on device: {0}")]
    Run(eyre::Report),

    /// Failed to package the artifacts.
    #[error("Failed to package the artifacts: {0}")]
    Package(eyre::Report),

    /// Failed to build the project.
    #[error("Failed to build the project: {0}")]
    Build(eyre::Report),

    /// Application crashed.
    #[error("Application crashed: {0}")]
    Crashed(String),
}

/// A clean application exit observed by the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationExit {
    reason: ApplicationExitReason,
}

impl ApplicationExit {
    /// The application process finished with a successful process status.
    #[must_use]
    pub const fn completed() -> Self {
        Self {
            reason: ApplicationExitReason::Completed,
        }
    }

    /// A GUI application window or process closed without crash evidence.
    #[must_use]
    pub const fn user_closed() -> Self {
        Self {
            reason: ApplicationExitReason::UserClosed,
        }
    }

    /// Human-readable message for terminal status output.
    #[must_use]
    pub const fn terminal_message(self) -> &'static str {
        match self.reason {
            ApplicationExitReason::Completed => "Application exited",
            ApplicationExitReason::UserClosed => "Application closed",
        }
    }

    /// Return the classified clean-exit reason.
    #[must_use]
    pub const fn reason(self) -> ApplicationExitReason {
        self.reason
    }
}

/// Reason attached to a clean application exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationExitReason {
    /// The launched process returned a successful exit status.
    Completed,
    /// The GUI app was closed and no crash report or panic log was found.
    UserClosed,
}

/// Events emitted by a running application on a device
#[derive(Debug)]
pub enum DeviceEvent {
    /// Application has started
    Started,
    /// Application has stopped by CLI
    Stopped,
    /// Standard output from the application
    Stdout {
        /// The output message
        message: String,
    },

    /// Standard error from the application
    Stderr {
        /// The error message
        message: String,
    },
    /// Standard log from the application
    Log {
        /// The log level
        level: tracing::Level,
        /// The log message
        message: String,
    },

    /// The CLI's own monitor of the run failed at something, such as a stop
    /// step overrunning its deadline. It is not terminal: the monitor still
    /// delivers its terminal event afterwards, and `water run` fails once
    /// the run ends.
    MonitorError {
        /// What the monitor failed at.
        message: String,
    },

    /// Clean exit of the application.
    Exited(ApplicationExit),

    /// Application crashed.
    Crashed(Crash),
}

impl DeviceEvent {
    const fn is_terminal(&self) -> bool {
        matches!(self, Self::Exited(_) | Self::Crashed(_) | Self::Stopped)
    }
}

/// An application crash observed by a device monitor.
#[derive(Debug, Clone)]
pub struct Crash {
    /// What ended the application.
    pub cause: CrashCause,
    /// How the process ended.
    pub process_end: Option<ProcessEnd>,
    /// The crash report the operating system wrote for a crash whose cause
    /// names it already, such as a panic.
    pub report: Option<Box<CrashReport>>,
}

/// How a process ended: the signal that killed it or the code it exited with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessEnd {
    /// The process was killed by a signal.
    Signal(i32),
    /// The process exited with a status code.
    ExitCode(i32),
}

#[cfg(unix)]
const fn signal_name(signal: i32) -> Option<&'static str> {
    match signal {
        nix::libc::SIGABRT => Some("SIGABRT"),
        nix::libc::SIGSEGV => Some("SIGSEGV"),
        _ => None,
    }
}

#[cfg(not(unix))]
const fn signal_name(_signal: i32) -> Option<&'static str> {
    None
}

impl std::fmt::Display for ProcessEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Signal(signal) => match signal_name(*signal) {
                Some(name) => write!(f, "signal {signal} ({name})"),
                None => write!(f, "signal {signal}"),
            },
            Self::ExitCode(code) => write!(f, "exit code {code}"),
        }
    }
}

impl Crash {
    /// A crash with the given cause and no crash report.
    #[must_use]
    pub const fn new(cause: CrashCause) -> Self {
        Self {
            cause,
            process_end: None,
            report: None,
        }
    }

    /// Supplemental crash details for a panic report.
    #[must_use]
    pub fn panic_note(&self) -> Option<String> {
        let mut details = Vec::new();
        if let Some(end) = self.process_end {
            details.push(format!("process terminated with {end}"));
        }
        if let Some(report) = &self.report {
            details.push(report.summary().to_owned());
        }
        (!details.is_empty()).then(|| details.join("; "))
    }
}

impl std::fmt::Display for Crash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.cause, f)?;
        if let Some(end) = self.process_end {
            write!(f, "\n  process terminated with {end}")?;
        }
        if let Some(report) = &self.report {
            write!(f, "\n\nCrash report: {}", report.log_path().display())?;
        }
        Ok(())
    }
}

/// What ended a crashed application.
#[derive(Debug, Clone)]
pub enum CrashCause {
    /// A Rust panic captured from the app's output or logs.
    Panic(PanicInfo),
    /// The process was ended by this signal.
    Signal(i32),
    /// The process exited with this non-zero code.
    ExitCode(i32),
    /// The operating system wrote a crash report for the process.
    Report(Box<CrashReport>),
    /// A platform-native crash description, such as an Android crash log.
    Native(String),
}

impl CrashCause {
    /// Whether this outcome is how a process ends when the CLI stops it: a
    /// termination signal or an exit code. Panics, crash reports, native
    /// crashes and other signals remain crashes after a stop.
    const fn ends_a_requested_stop(&self) -> bool {
        match self {
            Self::ExitCode(_) => true,
            #[cfg(unix)]
            Self::Signal(nix::libc::SIGTERM | nix::libc::SIGKILL | nix::libc::SIGINT) => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for CrashCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Panic(panic) => {
                write!(f, "Panic: {}", panic.payload)?;
                if let Some(location) = &panic.location {
                    write!(f, "\n  at {location}")?;
                }
                Ok(())
            }
            Self::Signal(signal) => match signal_name(*signal) {
                Some(name) => write!(f, "Process crashed ({name})"),
                None => write!(f, "Terminated by signal {signal}"),
            },
            Self::ExitCode(code) => write!(f, "Exit code: {code}"),
            Self::Report(report) => std::fmt::Display::fmt(report, f),
            Self::Native(description) => f.write_str(description),
        }
    }
}

/// A Rust panic reported by a running application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanicInfo {
    /// The panic message payload.
    pub payload: String,
    /// The source location where the panic occurred, as `file:line:column`.
    pub location: Option<String>,
}

/// Represents the kind of device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// Simulator device
    Simulator,
    /// Physical device
    Physical,
}

/// Represents the state of a device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Device is booted and ready
    Booted,
    /// Device is shutdown
    Shutdown,
    /// Device is disconnected (e.g., physical device unplugged)
    Disconnected,
}

// =============================================================================
// macOS-specific crash detection and logging
// =============================================================================

#[cfg(target_os = "macos")]
use smol::{
    Timer,
    io::{AsyncBufReadExt, BufReader},
    process::{Command, Stdio},
    spawn,
    stream::StreamExt,
};

#[cfg(target_os = "macos")]
struct MacosLogStream {
    task: smol::Task<DrainEnd>,
    panic_rx: Receiver<PanicInfo>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrainEnd {
    Marker,
    StreamClosed,
    ConsumerGone,
}

/// Start streaming logs from a `WaterUI` app on macOS.
///
/// Uses `log stream` with a predicate to filter by the `WaterUI` subsystem (`dev.waterui`).
/// This captures all tracing output from the Rust code via `tracing_oslog`.
///
/// The `log stream` process is returned beside the stream: it is spawned with
/// `kill_on_drop` and the reader task only holds its stdout, so whoever owns
/// the handle owns the process's lifetime. The caller retains it in the
/// [`Running`] so the stream ends with the run — the app exits, the user
/// cancels, the CLI receives `SIGTERM` — instead of outliving it as an orphan
/// on launchd, filtering for a process that no longer exists.
#[cfg(target_os = "macos")]
fn start_log_stream(
    host: &Host,
    sender: Sender<DeviceEvent>,
    log_level: Option<LogLevel>,
    pid: u32,
    end_marker: &str,
) -> Result<(MacosLogStream, smol::process::Child), FailToRun> {
    // Bounded channel with capacity 1 acts as oneshot - only first panic is captured
    let (panic_tx, panic_rx) = smol::channel::bounded::<PanicInfo>(1);

    // Always stream at default level to capture errors/faults, even if user didn't request logs
    let stream_level = log_level.map_or("default", |l| l.to_apple_level());

    // The marker clause admits the run's own end-of-entries marker: the
    // supervisor writes it through `logger` after the child exits. Treating
    // its arrival as proof that all earlier app entries were delivered is
    // the empirical logd ordering assumption documented below.
    let predicate = format!(
        "(processID == {pid} AND subsystem == \"dev.waterui\") OR \
         (process == \"logger\" AND eventMessage CONTAINS \"{end_marker}\")"
    );

    let mut log_cmd = host.command_in_own_process_group("log");
    log_cmd
        .arg("stream")
        .arg("--predicate")
        .arg(&predicate)
        .arg("--level")
        .arg(stream_level)
        .arg("--style")
        .arg("compact")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut log_child = log_cmd.spawn().map_err(|error| {
        FailToRun::Launch(eyre::eyre!("Failed to start macOS log stream: {error}"))
    })?;
    let stdout = log_child
        .stdout
        .take()
        .expect("stdout is piped for the macOS log stream");

    // The stream only forwards entries written after it attaches to logd, and
    // a fast first paint can beat the attach. The persisted store is replayed
    // once shortly after, but that replay does not reliably recover entries
    // written before the attach (#2080). Duplicates are harmless: the
    // consumer takes the first matching marker.
    replay_log_history(
        host.clone(),
        predicate,
        sender.clone(),
        panic_tx.clone(),
        log_level,
    );

    let end_marker = end_marker.to_string();
    let task = spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(Ok(line)) = lines.next().await {
            if line.starts_with("Filtering") || line.starts_with("Timestamp") {
                continue;
            }

            // The supervisor's end-of-entries marker: the empirical logd
            // ordering assumption means earlier app entries precede it, so
            // the reader is done when it arrives — before the stream is
            // dropped and unconsumed tail lines are lost.
            if line.contains(&end_marker) {
                return DrainEnd::Marker;
            }

            if line.contains("panic.payload=")
                && let Some(info) = extract_panic_info_from_log(&line)
            {
                let _ = panic_tx.try_send(info);
            }

            if log_level.is_some() {
                let level = if line.contains(" F ") || line.contains(" E ") {
                    tracing::Level::ERROR
                } else if line.contains(" W ") {
                    tracing::Level::WARN
                } else if line.contains(" D ") {
                    tracing::Level::DEBUG
                } else {
                    tracing::Level::INFO
                };

                if sender
                    .try_send(DeviceEvent::Log {
                        level,
                        message: line,
                    })
                    .is_err()
                {
                    return DrainEnd::ConsumerGone;
                }
            }
        }
        DrainEnd::StreamClosed
    });

    Ok((MacosLogStream { task, panic_rx }, log_child))
}

/// Forward `log show` output for the recent window into the same event path as
/// the live stream, a few seconds after the stream starts. Reads the persisted
/// store, so it recovers entries emitted before the stream attached to logd.
#[cfg(target_os = "macos")]
fn replay_log_history(
    host: Host,
    predicate: String,
    sender: Sender<DeviceEvent>,
    panic_tx: Sender<PanicInfo>,
    log_level: Option<LogLevel>,
) {
    spawn(async move {
        Timer::after(Duration::from_secs(4)).await;
        let Ok(output) = host
            .command("log")
            .args(["show", "--last", "2m", "--predicate", &predicate])
            .args(["--style", "compact"])
            .output()
            .await
        else {
            return;
        };
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.starts_with("Filtering") || line.starts_with("Timestamp") {
                continue;
            }
            if line.contains("panic.payload=")
                && let Some(info) = extract_panic_info_from_log(line)
            {
                let _ = panic_tx.try_send(info);
            }
            if log_level.is_some() {
                let level = if line.contains(" F ") || line.contains(" E ") {
                    tracing::Level::ERROR
                } else if line.contains(" W ") {
                    tracing::Level::WARN
                } else if line.contains(" D ") {
                    tracing::Level::DEBUG
                } else {
                    tracing::Level::INFO
                };
                let _ = sender.try_send(DeviceEvent::Log {
                    level,
                    message: line.to_string(),
                });
            }
        }
    })
    .detach();
}

/// Extract panic information from a log line containing panic.payload and panic.location fields.
#[cfg(target_os = "macos")]
fn extract_panic_info_from_log(line: &str) -> Option<PanicInfo> {
    let mut payload = None;
    let mut location = None;

    // Extract panic.payload="..."
    if let Some(start) = line.find("panic.payload=\"") {
        let start = start + 15;
        if let Some(end) = line[start..].find('"') {
            payload = Some(line[start..start + end].to_string());
        }
    }

    // Extract panic.location="..."
    if let Some(start) = line.find("panic.location=\"") {
        let start = start + 16;
        if let Some(end) = line[start..].find('"') {
            location = Some(line[start..start + end].to_string());
        }
    }

    payload.map(|p| PanicInfo {
        payload: p,
        location,
    })
}

/// Fetch recent panic logs from macOS unified logging system.
///
/// Uses `log show` to retrieve logs that contain panic info.
/// Returns the panic message if found, along with location and payload.
#[cfg(target_os = "macos")]
async fn fetch_recent_panic_logs(
    host: &Host,
    started_at: Instant,
    pid: Option<u32>,
) -> Option<PanicInfo> {
    let last = started_at.elapsed() + Duration::from_secs(2);
    let last_arg = format!("{}s", last.as_secs().max(5));

    let predicate = pid.map_or_else(
        || "subsystem == \"dev.waterui\" AND eventMessage CONTAINS \"panic\"".to_string(),
        |pid| {
            format!(
                "processID == {pid} AND subsystem == \"dev.waterui\" AND eventMessage CONTAINS \"panic\""
            )
        },
    );

    let output = host
        .output(
            "log",
            [
                "show",
                "--predicate",
                predicate.as_str(),
                "--style",
                "compact",
                "--last",
                last_arg.as_str(),
            ],
        )
        .await
        .ok()?;

    let stdout = String::from_utf8(output.stdout).ok()?;

    for line in stdout.lines() {
        if line.starts_with("Filtering") || line.starts_with("Timestamp") || line.is_empty() {
            continue;
        }

        let mut location = None;
        let mut payload = None;

        if let Some(loc_start) = line.find("panic.location=\"") {
            let start = loc_start + 16;
            if let Some(end) = line[start..].find('"') {
                location = Some(&line[start..start + end]);
            }
        }

        if let Some(pay_start) = line.find("panic.payload=\"") {
            let start = pay_start + 15;
            if let Some(end) = line[start..].find('"') {
                payload = Some(&line[start..start + end]);
            }
        }

        if payload.is_some() || location.is_some() {
            return Some(PanicInfo {
                payload: payload.unwrap_or_default().to_string(),
                location: location.map(str::to_string),
            });
        }
    }

    None
}

// =============================================================================
// Local Device
// =============================================================================

/// Local device representing the current machine.
///
/// This is a shared device that works with ANY backend:
/// - Apple backend: runs the executable inside a macOS `.app` bundle
/// - GTK4 backend: runs cargo binaries directly
///
/// The artifact type determines how it's executed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Local;

impl Device for Local {
    fn name(&self) -> &'static str {
        "Local Machine"
    }

    fn launch(&self, _host: &Host) -> impl Future<Output = eyre::Result<()>> + Send {
        // No-op - local machine is always "launched"
        std::future::ready(Ok(()))
    }

    async fn run(
        &self,
        host: &Host,
        artifact: Artifact,
        options: RunOptions,
    ) -> Result<Running, FailToRun> {
        let artifact_path = artifact.path();

        // Dispatch based on artifact type
        match artifact_path.extension().and_then(|e| e.to_str()) {
            Some("app") => {
                // macOS .app bundle - supervise its real executable
                run_macos_app(host, artifact, options).await
            }
            _ => {
                // Binary executable - run directly
                run_binary(host, &artifact, &options)
            }
        }
    }

    fn scan(_host: &Host) -> impl Future<Output = eyre::Result<Vec<Self>>> + Send {
        // Local machine is always available - just return a single instance
        std::future::ready(Ok(vec![Self]))
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacosProcess {
    pid: u32,
    command: String,
}

#[cfg(target_os = "macos")]
async fn list_macos_processes(host: &Host) -> Result<Vec<MacosProcess>, FailToRun> {
    let output = host
        .output("ps", ["-axo", "pid=,command="])
        .await
        .map_err(|e| FailToRun::Launch(eyre::eyre!("Failed to list local processes: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(FailToRun::Launch(eyre::eyre!(
            "Failed to list local processes with ps: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }

        let mut fields = trimmed.splitn(2, char::is_whitespace);
        let Some(pid_str) = fields.next() else {
            continue;
        };
        let Some(command) = fields.next() else {
            continue;
        };

        let pid = pid_str.parse::<u32>().map_err(|e| {
            FailToRun::Launch(eyre::eyre!(
                "Failed to parse process id '{pid_str}' from ps output: {e}"
            ))
        })?;
        processes.push(MacosProcess {
            pid,
            command: command.trim_start().to_string(),
        });
    }

    Ok(processes)
}

#[cfg(target_os = "macos")]
fn command_runs_executable(command: &str, executable_path: &Path) -> bool {
    let executable = executable_path.to_string_lossy();
    command == executable || command.starts_with(&format!("{executable} "))
}

#[cfg(target_os = "macos")]
async fn read_macos_bundle_identifier(app_path: &Path) -> Result<String, FailToRun> {
    let plist_path = app_path.join("Contents").join("Info.plist");
    smol::unblock({
        let plist_path = plist_path.clone();
        move || -> eyre::Result<String> {
            let plist = plist::Value::from_file(&plist_path).map_err(|error| {
                eyre::eyre!(
                    "Failed to read bundle Info.plist at '{}': {error}",
                    plist_path.display()
                )
            })?;
            let dictionary = plist.into_dictionary().ok_or_else(|| {
                eyre::eyre!(
                    "Bundle Info.plist at '{}' must contain a dictionary root",
                    plist_path.display()
                )
            })?;
            dictionary
                .get("CFBundleIdentifier")
                .and_then(plist::Value::as_string)
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    eyre::eyre!(
                        "Bundle Info.plist at '{}' is missing CFBundleIdentifier",
                        plist_path.display()
                    )
                })
        }
    })
    .await
    .map_err(FailToRun::Launch)
}

#[cfg(target_os = "macos")]
fn command_app_bundle_path_for_executable(command: &str, executable_name: &str) -> Option<PathBuf> {
    const BUNDLE_SUFFIX: &str = ".app";
    const EXECUTABLE_MARKER: &str = ".app/Contents/MacOS/";

    let command = command.trim_start();
    if !command.starts_with('/') {
        return None;
    }
    let marker_start = command.find(EXECUTABLE_MARKER)?;
    let executable_start = marker_start + EXECUTABLE_MARKER.len();
    let executable_end = executable_start.checked_add(executable_name.len())?;
    if !command[executable_start..].starts_with(executable_name) {
        return None;
    }
    if command
        .as_bytes()
        .get(executable_end)
        .is_some_and(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
    {
        return None;
    }

    let app_end = marker_start + BUNDLE_SUFFIX.len();
    Some(PathBuf::from(&command[..app_end]))
}

#[cfg(target_os = "macos")]
async fn list_conflicting_macos_app_pids(
    host: &Host,
    launch: &MacosBundleLaunchContext,
) -> Result<Vec<u32>, FailToRun> {
    let executable_name = launch
        .executable_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            FailToRun::Launch(eyre::eyre!(
                "Failed to determine executable name for '{}'",
                launch.executable_path.display()
            ))
        })?;

    let mut pids = BTreeSet::new();
    for process in list_macos_processes(host).await? {
        if command_runs_executable(&process.command, &launch.executable_path) {
            pids.insert(process.pid);
            continue;
        }

        let Some(app_path) =
            command_app_bundle_path_for_executable(&process.command, executable_name)
        else {
            continue;
        };
        // A running process whose bundle can no longer be identified (its
        // build directory was deleted after launch) cannot be an instance of
        // the bundle being launched; it must not fail this launch.
        match read_macos_bundle_identifier(&app_path).await {
            Ok(bundle_id) if bundle_id == launch.bundle_id => {
                pids.insert(process.pid);
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    pid = process.pid,
                    path = %app_path.display(),
                    "Skipping running app with unreadable bundle: {error:?}"
                );
            }
        }
    }

    Ok(pids.into_iter().collect())
}

#[cfg(target_os = "macos")]
fn quiet_kill_command(host: &Host, signal: &str, pid: &str) -> Command {
    let mut command = host.command("kill");
    command
        .arg(signal)
        .arg(pid)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(target_os = "macos")]
async fn is_pid_alive(host: &Host, pid: u32) -> bool {
    let pid = pid.to_string();
    quiet_kill_command(host, "-0", &pid)
        .status()
        .await
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "macos")]
async fn terminate_pids(host: &Host, pids: &[u32]) -> Result<(), FailToRun> {
    if pids.is_empty() {
        return Ok(());
    }

    for &pid in pids {
        let pid = pid.to_string();
        let status = quiet_kill_command(host, "-TERM", &pid)
            .status()
            .await
            .map_err(|e| {
                FailToRun::Launch(eyre::eyre!(
                    "Failed to terminate existing app process {pid}: {e}"
                ))
            })?;
        if !status.success() {
            return Err(FailToRun::Launch(eyre::eyre!(
                "Failed to terminate existing app process {pid} before relaunch"
            )));
        }
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let mut alive = false;
        for &pid in pids {
            if is_pid_alive(host, pid).await {
                alive = true;
                break;
            }
        }
        if !alive {
            return Ok(());
        }
        Timer::after(Duration::from_millis(80)).await;
    }

    Err(FailToRun::Launch(eyre::eyre!(
        "Timed out waiting for previous app instance(s) to terminate before relaunch"
    )))
}

#[cfg(target_os = "macos")]
pub(crate) async fn resolve_macos_bundle_executable_path(
    artifact_path: &Path,
) -> Result<PathBuf, FailToRun> {
    let plist_path = artifact_path.join("Contents").join("Info.plist");
    let executable_name = smol::unblock({
        let plist_path = plist_path.clone();
        move || -> eyre::Result<String> {
            let plist = plist::Value::from_file(&plist_path).map_err(|error| {
                eyre::eyre!(
                    "Failed to read bundle Info.plist at '{}': {error}",
                    plist_path.display()
                )
            })?;
            let dictionary = plist.into_dictionary().ok_or_else(|| {
                eyre::eyre!(
                    "Bundle Info.plist at '{}' must contain a dictionary root",
                    plist_path.display()
                )
            })?;
            let executable = dictionary
                .get("CFBundleExecutable")
                .and_then(plist::Value::as_string)
                .ok_or_else(|| {
                    eyre::eyre!(
                        "Bundle Info.plist at '{}' is missing CFBundleExecutable",
                        plist_path.display()
                    )
                })?;
            Ok(executable.to_string())
        }
    })
    .await
    .map_err(FailToRun::Launch)?;

    Ok(artifact_path
        .join("Contents")
        .join("MacOS")
        .join(executable_name))
}

#[cfg(target_os = "macos")]
struct MacosBundleLaunchContext {
    bundle_id: String,
    artifact_path: PathBuf,
    executable_path: PathBuf,
}

#[cfg(target_os = "macos")]
async fn prepare_macos_bundle_launch(
    artifact: Artifact,
) -> Result<MacosBundleLaunchContext, FailToRun> {
    let artifact_path = artifact.path().to_path_buf();
    let executable_path = resolve_macos_bundle_executable_path(&artifact_path).await?;

    Ok(MacosBundleLaunchContext {
        bundle_id: artifact.bundle_id().to_string(),
        artifact_path,
        executable_path,
    })
}

/// Run a macOS `.app` bundle by spawning its executable directly.
///
/// `open -W` cannot supervise the app: `open` exits 0 once the launched
/// process goes away regardless of how it died, and the app's stderr is
/// handed to `LaunchServices` instead of the caller. Spawning
/// `Contents/MacOS/<executable>` keeps the child supervised here — its exit
/// status decides the run's, and its stderr reaches the user — while the
/// process still runs inside its bundle, so its identity, resources, and
/// unified-logging stream are unchanged. App logs are captured from unified
/// logging by PID.
#[cfg(target_os = "macos")]
async fn run_macos_app(
    host: &Host,
    artifact: Artifact,
    options: RunOptions,
) -> Result<Running, FailToRun> {
    run_macos_app_with_grace(host, artifact, options, termination_grace()).await
}

#[cfg(target_os = "macos")]
async fn run_macos_app_with_grace(
    host: &Host,
    artifact: Artifact,
    options: RunOptions,
    grace: std::time::Duration,
) -> Result<Running, FailToRun> {
    use tracing::info;

    let launch = prepare_macos_bundle_launch(artifact).await?;
    let started_at = Instant::now();

    if options.replace_existing_macos_app_instances() {
        let existing_pids = list_conflicting_macos_app_pids(host, &launch).await?;
        terminate_pids(host, &existing_pids).await?;
    }

    info!("Launching app on macOS: {}", launch.artifact_path.display());
    // The app leads its own process group: a terminal `Ctrl-C` must reach
    // this process alone so the monitor controls its termination.
    let mut command = host.command_in_own_process_group(&launch.executable_path);
    for (key, value) in options.env_vars() {
        command.env(key, value);
    }
    // Match the environment `open` gave the app: no inherited stdin and `/`
    // as the working directory.
    command
        .stdin(Stdio::null())
        .current_dir("/")
        .kill_on_drop(true);
    let app_log_file = configure_app_stdio(&mut command, options.app_log_file())?;
    let child = command.spawn().map_err(|error| {
        FailToRun::Launch(eyre::eyre!(
            "Failed to launch '{}': {error}",
            launch.executable_path.display()
        ))
    })?;
    let app_pid = child.id();
    let (mut running, sender, control) = Running::new();
    if let Some(log_file) = app_log_file {
        spawn_app_log_file_follower(log_file, sender.clone());
    }
    // Unique per run: `log stream` admits this text through the marker
    // clause of its predicate, and the exit monitor writes it after the
    // child exits to end the log reader only once every earlier entry the
    // app wrote has been delivered.
    let end_marker = format!(
        "waterui-log-drain-{app_pid}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |span| span.as_nanos())
    );
    let (log_stream, log_child) = start_log_stream(
        host,
        sender.clone(),
        options.log_level(),
        app_pid,
        &end_marker,
    )?;
    running.retain(log_child);
    let monitor = ChildMonitor::new(child, sender.clone(), control, grace);
    spawn_macos_app_exit_monitor(
        host, monitor, log_stream, sender, started_at, app_pid, end_marker,
    );

    Ok(running)
}

/// Route a spawned macOS app's stdout and stderr.
///
/// With a log file, both streams append to it (created, with parents): a file
/// is the right channel for an app designed to outlive this command, where a
/// pipe whose reader has exited raises SIGPIPE on unix (water-rs/cli#197).
/// Without one, the streams keep the pipes the [`ChildMonitor`] forwards while
/// the app is supervised.
///
/// Returns the log file's path when the streams were redirected to it.
#[cfg(target_os = "macos")]
fn configure_app_stdio(
    command: &mut Command,
    app_log_file: Option<&Path>,
) -> Result<Option<PathBuf>, FailToRun> {
    let Some(path) = app_log_file else {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        return Ok(None);
    };
    let open_log = || {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    };
    let stdout = open_log().map_err(|error| {
        FailToRun::Launch(eyre::eyre!(
            "Failed to open the app log file {}: {error}",
            path.display()
        ))
    })?;
    let stderr = open_log().map_err(|error| {
        FailToRun::Launch(eyre::eyre!(
            "Failed to open the app log file {}: {error}",
            path.display()
        ))
    })?;
    command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    Ok(Some(path.to_path_buf()))
}

/// Follow the file a spawned app logs to and forward new lines as
/// [`DeviceEvent::Log`].
///
/// The file is the app's log channel for the rest of its life — a detached
/// support app keeps writing long after this command exits — but this run
/// still streams what lands while it runs. A task, not a `tail` child: a child
/// retained on the [`Running`] would be forgotten by [`Running::detach`] and
/// orphaned past the command's exit.
#[cfg(target_os = "macos")]
fn spawn_app_log_file_follower(path: PathBuf, sender: Sender<DeviceEvent>) {
    use smol::io::{AsyncReadExt, AsyncSeekExt};
    spawn(async move {
        let mut file = match smol::fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(path = %path.display(), "Cannot follow the app log file: {error}");
                return;
            }
        };
        // Only lines written during this run matter.
        if let Err(error) = file.seek(std::io::SeekFrom::End(0)).await {
            tracing::warn!(path = %path.display(), "Cannot seek the app log file: {error}");
            return;
        }
        let mut pending = String::new();
        loop {
            let mut chunk = [0u8; 8192];
            match file.read(&mut chunk).await {
                Ok(0) => {
                    if sender.is_closed() {
                        return;
                    }
                    Timer::after(Duration::from_millis(200)).await;
                }
                Ok(read) => {
                    pending.push_str(&String::from_utf8_lossy(&chunk[..read]));
                    while let Some(newline) = pending.find('\n') {
                        let line = pending[..newline].trim_end().to_string();
                        pending.drain(..=newline);
                        if !line.is_empty()
                            && sender
                                .try_send(DeviceEvent::Log {
                                    level: parse_log_level(&line),
                                    message: line,
                                })
                                .is_err()
                        {
                            return;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(path = %path.display(), "App log file read failed: {error}");
                    return;
                }
            }
        }
    })
    .detach();
}

/// Run a macOS .app bundle on non-macOS platforms (not supported).
#[cfg(not(target_os = "macos"))]
fn run_macos_app(
    _host: &Host,
    _artifact: Artifact,
    _options: RunOptions,
) -> impl std::future::Future<Output = Result<Running, FailToRun>> {
    std::future::ready(Err(FailToRun::InvalidArtifact)) // .app bundles only work on macOS
}

/// Run a binary executable directly.
///
/// Captures stdout/stderr and extracts panic messages from stderr.
fn run_binary(
    host: &Host,
    artifact: &Artifact,
    options: &RunOptions,
) -> Result<Running, FailToRun> {
    run_binary_with_grace(host, artifact, options, termination_grace())
}

fn run_binary_with_grace(
    host: &Host,
    artifact: &Artifact,
    options: &RunOptions,
    grace: std::time::Duration,
) -> Result<Running, FailToRun> {
    let binary_path = artifact.path();
    if !binary_path.exists() {
        return Err(FailToRun::InvalidArtifact);
    }

    let child = spawn_local_child(host, binary_path, options)?;
    let (running, sender, control) = Running::new();
    let monitor = ChildMonitor::new(child, sender.clone(), control, grace);
    spawn_binary_exit_monitor(monitor, sender);

    Ok(running)
}

const fn termination_grace() -> std::time::Duration {
    #[cfg(unix)]
    {
        TERMINATION_GRACE_PERIOD
    }
    #[cfg(not(unix))]
    {
        std::time::Duration::ZERO
    }
}

fn spawn_local_child(
    host: &Host,
    executable_path: &Path,
    options: &RunOptions,
) -> Result<smol::process::Child, FailToRun> {
    use smol::process::Stdio;

    // On unix the child leads its own process group: a terminal `Ctrl-C`
    // reaches this CLI alone, and `water` forwards one termination signal
    // itself. Windows has no process-group signal routing — the console
    // delivers `Ctrl-C` to every attached process, so the child shares the
    // console's group.
    #[cfg(unix)]
    let mut cmd = host.command_in_own_process_group(executable_path);
    #[cfg(not(unix))]
    let mut cmd = host.command(executable_path);
    for (key, value) in options.env_vars() {
        cmd.env(key, value);
    }

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    cmd.spawn().map_err(|error| {
        FailToRun::Launch(eyre::eyre!(
            "Failed to launch '{}': {error}",
            executable_path.display()
        ))
    })
}

fn spawn_stdout_forwarder(
    stdout: smol::process::ChildStdout,
    sender: Sender<DeviceEvent>,
) -> smol::Task<()> {
    use smol::io::{AsyncBufReadExt, BufReader};
    use smol::spawn;
    use smol::stream::StreamExt;

    spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Some(result) = lines.next().await {
            let Ok(line) = result else { break };
            if sender
                .try_send(DeviceEvent::Log {
                    level: parse_log_level(&line),
                    message: line,
                })
                .is_err()
            {
                break;
            }
        }
    })
}

fn spawn_stderr_forwarder(
    stderr: smol::process::ChildStderr,
    sender: Sender<DeviceEvent>,
    panic_tx: Sender<PanicInfo>,
) -> smol::Task<()> {
    use smol::io::{AsyncBufReadExt, BufReader};
    use smol::spawn;
    use smol::stream::StreamExt;

    spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        let mut panic_lines = Vec::new();
        let mut capturing_panic = false;

        while let Some(result) = lines.next().await {
            let Ok(line) = result else { break };

            if starts_panic_capture(&line) {
                capturing_panic = true;
                panic_lines.clear();
            }

            if capturing_panic {
                panic_lines.push(line.clone());
                if should_flush_panic_capture(&panic_lines, &line) {
                    capturing_panic = false;
                    try_send_panic(&panic_tx, &panic_lines);
                }
            }

            if sender
                .try_send(DeviceEvent::Stderr { message: line })
                .is_err()
            {
                break;
            }
        }

        if capturing_panic && !panic_lines.is_empty() {
            try_send_panic(&panic_tx, &panic_lines);
        }
    })
}

fn starts_panic_capture(line: &str) -> bool {
    line.contains("panicked at") || line.starts_with("thread '") && line.contains("panic")
}

fn should_flush_panic_capture(panic_lines: &[String], line: &str) -> bool {
    panic_lines.len() > 10 || panic_lines.len() > 2 && line.trim().is_empty()
}

fn try_send_panic(panic_tx: &Sender<PanicInfo>, panic_lines: &[String]) {
    if let Some(panic) = extract_panic(panic_lines) {
        let _ = panic_tx.try_send(panic);
    }
}

/// The longest `water run` waits for a stopped child to exit on its own
/// before escalating to `SIGKILL`.
///
/// The one termination signal the cancel action sends reaches the app's
/// termination hooks, and those hooks are user code — persisting state,
/// flushing logs — that needs real time to finish; too short a bound would
/// cut a legitimate hook off mid-write. It is still a user-facing stop: an
/// app that never answers cannot hold the CLI forever. Five seconds leaves
/// a reasonable hook room to run while keeping `Ctrl-C` responsive.
///
/// Unix-only: Windows has no termination signal and uses a zero grace period,
/// so the caller kills once.
#[cfg(unix)]
pub(crate) const TERMINATION_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);

/// How the grace period of a child sent its termination signal ended.
pub(crate) enum TerminationOutcome {
    /// The child exited within the grace period.
    Exited(std::io::Result<std::process::ExitStatus>),
    /// The grace period ran out; already reported as a [`DeviceEvent::MonitorError`].
    Overran,
    /// The supervisor asked for a kill before the grace period ran out.
    KillRequested,
}

/// How [`until_deadline_or_kill`] ended.
enum Raced<T> {
    Done(T),
    DeadlinePassed,
    KillRequested,
}

/// Await `work` for at most `deadline` while still reading the run's control
/// requests: a repeated `Terminate` is ignored, a closed control channel stops
/// being read, and `Kill` ends the wait.
async fn until_deadline_or_kill<F: std::future::Future>(
    work: F,
    control: &mut Option<Receiver<StopRequest>>,
    deadline: std::time::Duration,
) -> Raced<F::Output> {
    use futures_util::future::{Either, select};

    let mut work = std::pin::pin!(work);
    let mut timer = std::pin::pin!(smol::Timer::after(deadline));
    loop {
        if let Some(receiver) = control.as_mut() {
            let request = std::pin::pin!(receiver.recv());
            match select(work.as_mut(), select(timer.as_mut(), request)).await {
                Either::Left((result, _)) => return Raced::Done(result),
                Either::Right((Either::Left((_, _)), _)) => return Raced::DeadlinePassed,
                Either::Right((Either::Right((Ok(StopRequest::Kill), _)), _)) => {
                    return Raced::KillRequested;
                }
                Either::Right((Either::Right((Err(_), _)), _)) => *control = None,
                Either::Right((Either::Right((Ok(StopRequest::Terminate), _)), _)) => {}
            }
        } else {
            match select(work.as_mut(), timer.as_mut()).await {
                Either::Left((result, _)) => return Raced::Done(result),
                Either::Right((_, _)) => return Raced::DeadlinePassed,
            }
        }
    }
}

/// Wait up to `grace` for a child that was sent its termination signal,
/// still reading the run's control requests. A repeated `Terminate` is
/// ignored, a closed control channel stops being read, `Overran` reports the
/// error, and `KillRequested` does not.
pub(crate) async fn await_termination(
    child: &mut smol::process::Child,
    control: &mut Option<Receiver<StopRequest>>,
    grace: std::time::Duration,
    sender: &Sender<DeviceEvent>,
) -> TerminationOutcome {
    match until_deadline_or_kill(child.status(), control, grace).await {
        Raced::Done(status) => TerminationOutcome::Exited(status),
        Raced::DeadlinePassed => {
            report_monitor_error(
                sender,
                format!(
                    "The app did not exit within the {grace:?} termination grace period; killing it"
                ),
            );
            TerminationOutcome::Overran
        }
        Raced::KillRequested => TerminationOutcome::KillRequested,
    }
}

/// The longest the run's output may take to reach end-of-file once the app's
/// process is reaped, whether it exited on its own or was stopped. The pipes
/// close when the app exits unless a process it spawned still holds them; such
/// a process is not this run's app, so its output does not hold back the
/// terminal event.
pub(crate) const OUTPUT_JOIN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// How joining the run's output ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OutputJoin {
    /// The output reached end-of-file.
    Finished,
    /// A process outside the run still held the output at the deadline.
    OutlivedByOthers,
    /// The supervisor asked for a kill while the output was joining.
    KillRequested,
}

pub(crate) async fn join_output(
    output: impl std::future::Future<Output = ()>,
    control: &mut Option<Receiver<StopRequest>>,
) -> OutputJoin {
    match until_deadline_or_kill(output, control, OUTPUT_JOIN_DEADLINE).await {
        Raced::Done(()) => OutputJoin::Finished,
        Raced::DeadlinePassed => {
            tracing::warn!(
                "Output from processes that outlive the app is no longer forwarded: the app's output was still open {OUTPUT_JOIN_DEADLINE:?} after it exited"
            );
            OutputJoin::OutlivedByOthers
        }
        Raced::KillRequested => OutputJoin::KillRequested,
    }
}

/// Await `future` for at most `deadline`; `None` means it overran.
pub(crate) async fn within<T>(
    deadline: std::time::Duration,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    use futures_util::future::{Either, select};

    let future = std::pin::pin!(future);
    let timer = std::pin::pin!(smol::Timer::after(deadline));
    match select(future, timer).await {
        Either::Left((result, _)) => Some(result),
        Either::Right((_, _)) => None,
    }
}

/// Report a monitor failure to the run's consumer as a
/// [`DeviceEvent::MonitorError`], which is its single report: the consumer
/// prints it.
pub(crate) fn report_monitor_error(sender: &Sender<DeviceEvent>, message: String) {
    let _ = sender.try_send(DeviceEvent::MonitorError { message });
}

struct ChildMonitor {
    child: smol::process::Child,
    sender: Sender<DeviceEvent>,
    stdout_task: Option<smol::Task<()>>,
    stderr_task: Option<smol::Task<()>>,
    panic_rx: Receiver<PanicInfo>,
    control: Option<Receiver<StopRequest>>,
    termination_grace: std::time::Duration,
}

impl ChildMonitor {
    fn new(
        mut child: smol::process::Child,
        sender: Sender<DeviceEvent>,
        control: Receiver<StopRequest>,
        termination_grace: std::time::Duration,
    ) -> Self {
        let (panic_tx, panic_rx) = smol::channel::unbounded::<PanicInfo>();
        let stdout_task = child
            .stdout
            .take()
            .map(|stdout| spawn_stdout_forwarder(stdout, sender.clone()));
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| spawn_stderr_forwarder(stderr, sender.clone(), panic_tx));

        Self {
            child,
            sender,
            stdout_task,
            stderr_task,
            panic_rx,
            control: Some(control),
            termination_grace,
        }
    }

    async fn wait(mut self) -> ChildExit {
        use futures_util::future::{Either, select};

        #[cfg(target_os = "macos")]
        let mut stopped = false;
        let status = loop {
            let Some(control) = &mut self.control else {
                break self.child.status().await;
            };
            let child_status = std::pin::pin!(self.child.status());
            let request = std::pin::pin!(control.recv());
            match select(child_status, request).await {
                Either::Left((status, _)) => break status,
                Either::Right((Err(_), _)) => self.control = None,
                Either::Right((Ok(StopRequest::Terminate), _)) => {
                    #[cfg(target_os = "macos")]
                    {
                        stopped = true;
                    }
                    #[cfg(unix)]
                    self.terminate();
                    break self.wait_after_terminate().await;
                }
                Either::Right((Ok(StopRequest::Kill), _)) => {
                    #[cfg(target_os = "macos")]
                    {
                        stopped = true;
                    }
                    self.kill();
                    break self.child.status().await;
                }
            }
        };

        let stdout_task = self.stdout_task.take();
        let stderr_task = self.stderr_task.take();
        let forwarders = async move {
            if let Some(task) = stdout_task {
                task.await;
            }
            if let Some(task) = stderr_task {
                task.await;
            }
        };
        let _ = join_output(forwarders, &mut self.control).await;

        ChildExit {
            status,
            panic: latest_panic(&self.panic_rx),
            #[cfg(target_os = "macos")]
            stopped,
            control: self.control,
        }
    }

    async fn wait_after_terminate(&mut self) -> std::io::Result<std::process::ExitStatus> {
        if self.termination_grace.is_zero() {
            self.kill();
            return self.child.status().await;
        }

        match await_termination(
            &mut self.child,
            &mut self.control,
            self.termination_grace,
            &self.sender,
        )
        .await
        {
            TerminationOutcome::Exited(status) => status,
            TerminationOutcome::Overran | TerminationOutcome::KillRequested => {
                self.kill();
                self.child.status().await
            }
        }
    }

    #[cfg(unix)]
    fn terminate(&self) {
        // Child::id remains valid until this Child has been waited on; the
        // monitor owns it and is the only code that may reap this PID.
        let Ok(raw_pid) = i32::try_from(self.child.id()) else {
            report_monitor_error(
                &self.sender,
                "Child PID does not fit in nix::unistd::Pid".to_string(),
            );
            return;
        };
        if let Err(error) = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(raw_pid),
            nix::sys::signal::Signal::SIGTERM,
        ) {
            report_monitor_error(
                &self.sender,
                format!("Could not send SIGTERM to child: {error}"),
            );
        }
    }

    fn kill(&mut self) {
        if let Err(error) = self.child.kill() {
            report_monitor_error(&self.sender, format!("Could not kill child: {error}"));
        }
    }
}

struct ChildExit {
    status: std::io::Result<std::process::ExitStatus>,
    panic: Option<PanicInfo>,
    /// Whether the monitor itself delivered a terminate or kill to the child.
    #[cfg(target_os = "macos")]
    stopped: bool,
    /// The run's control receiver, held until the terminal event is sent so
    /// the supervisor's requests keep being delivered while the run ends.
    control: Option<Receiver<StopRequest>>,
}

fn spawn_binary_exit_monitor(monitor: ChildMonitor, sender: Sender<DeviceEvent>) {
    use smol::spawn;

    spawn(async move {
        let ChildExit {
            status,
            panic,
            control,
            ..
        } = monitor.wait().await;
        emit_process_exit_event(&sender, status, panic, ApplicationExit::completed());
        drop(control);
    })
    .detach();
}

/// Write the run's end-of-entries marker into the unified log.
///
/// `log stream`'s predicate admits this text through its marker clause.
/// Relying on its arrival as the end-of-entries signal assumes logd delivers
/// entries to a stream subscriber in receipt order across processes. This is
/// empirical — observed, not documented by Apple — and the drain deadline
/// bounds the cost if the assumption is ever wrong.
#[cfg(target_os = "macos")]
async fn write_log_end_marker(host: &Host, marker: &str) -> Result<(), String> {
    let output = host
        .command_in_own_process_group("logger")
        .arg(marker)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "`logger` exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Maximum time to wait for the unified log stream to deliver the drain marker.
#[cfg(target_os = "macos")]
const LOG_DRAIN_DEADLINE: Duration = Duration::from_secs(3);

/// Let the log reader finish on the marker written after the child's
/// exit rather than dropping it mid-delivery.
///
/// The child exiting does not mean `log stream` already delivered the
/// app's last entries — a shutdown hook's output can still be in flight.
/// Awaiting the task lets it consume through the marker; this relies on the
/// empirical logd ordering assumption documented by [`write_log_end_marker`].
/// It also ends when `log stream` itself dies (the reader's stdout hits EOF).
/// Writing the marker and reading up to it share one [`LOG_DRAIN_DEADLINE`].
#[cfg(target_os = "macos")]
async fn drain_log_stream(
    host: &Host,
    log_stream: MacosLogStream,
    marker: &str,
    sender: &Sender<DeviceEvent>,
) -> Receiver<PanicInfo> {
    enum Drain {
        Ended(DrainEnd),
        MarkerFailed(String),
    }

    let MacosLogStream { mut task, panic_rx } = log_stream;
    let drain = within(LOG_DRAIN_DEADLINE, async {
        if let Err(error) = write_log_end_marker(host, marker).await {
            return Drain::MarkerFailed(error);
        }
        Drain::Ended((&mut task).await)
    })
    .await;
    match drain {
        Some(Drain::Ended(DrainEnd::Marker | DrainEnd::ConsumerGone)) => {}
        Some(Drain::Ended(DrainEnd::StreamClosed)) => {
            report_monitor_error(
                sender,
                "macOS log stream ended before the drain marker".to_string(),
            );
        }
        Some(Drain::MarkerFailed(error)) => {
            report_monitor_error(
                sender,
                format!(
                    "macOS log drain failed: {error}; app log lines written after exit may be missing"
                ),
            );
            task.cancel().await;
        }
        None => {
            report_monitor_error(
                sender,
                format!(
                    "macOS log drain failed: timed out after {LOG_DRAIN_DEADLINE:?}; app log lines written after exit may be missing"
                ),
            );
            task.cancel().await;
        }
    }
    panic_rx
}

#[cfg(target_os = "macos")]
fn spawn_macos_app_exit_monitor(
    host: &Host,
    monitor: ChildMonitor,
    log_stream: MacosLogStream,
    sender: Sender<DeviceEvent>,
    started_at: Instant,
    pid: u32,
    end_marker: String,
) {
    let host = host.clone();
    spawn(async move {
        let ChildExit {
            status,
            panic,
            stopped,
            control,
        } = monitor.wait().await;
        let panic_rx = drain_log_stream(&host, log_stream, &end_marker, &sender).await;
        let mut panic = panic.or_else(|| latest_panic(&panic_rx));

        // A stop this monitor delivered is not a crash: the panic search
        // only runs for an exit nobody asked for.
        if panic.is_none()
            && !stopped
            && matches!(&status, Ok(exit_status) if !exit_status.success())
        {
            match within(
                PANIC_LOG_QUERY_DEADLINE,
                fetch_recent_panic_logs(&host, started_at, Some(pid)),
            )
            .await
            {
                Some(found) => panic = found,
                None => report_monitor_error(
                    &sender,
                    format!(
                        "The macOS panic log query timed out after {PANIC_LOG_QUERY_DEADLINE:?}; a panic may be unreported"
                    ),
                ),
            }
        }

        emit_process_exit_event(&sender, status, panic, ApplicationExit::user_closed());
        drop(control);
    })
    .detach();
}

fn latest_panic(panic_rx: &Receiver<PanicInfo>) -> Option<PanicInfo> {
    let mut latest = None;
    while let Ok(panic) = panic_rx.try_recv() {
        latest = Some(panic);
    }
    latest
}

fn emit_process_exit_event(
    sender: &Sender<DeviceEvent>,
    status: std::io::Result<std::process::ExitStatus>,
    panic: Option<PanicInfo>,
    successful_exit: ApplicationExit,
) {
    let cause = match status {
        Ok(exit_status) if exit_status.success() => {
            let _ = sender.try_send(DeviceEvent::Exited(successful_exit));
            return;
        }
        Ok(exit_status) => process_crash_cause(exit_status, panic),
        Err(error) => Crash::new(CrashCause::Native(format!("Process error: {error}"))),
    };
    let _ = sender.try_send(DeviceEvent::Crashed(cause));
}

fn process_crash_cause(exit_status: std::process::ExitStatus, panic: Option<PanicInfo>) -> Crash {
    let process_end = {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;

            exit_status.signal().map_or_else(
                || ProcessEnd::ExitCode(exit_status.code().unwrap_or(-1)),
                ProcessEnd::Signal,
            )
        }
        #[cfg(not(unix))]
        {
            ProcessEnd::ExitCode(exit_status.code().unwrap_or(-1))
        }
    };
    // A signal or exit code cause already says how the process ended.
    panic.map_or_else(
        || {
            Crash::new(match process_end {
                ProcessEnd::Signal(signal) => CrashCause::Signal(signal),
                ProcessEnd::ExitCode(code) => CrashCause::ExitCode(code),
            })
        },
        |panic| Crash {
            cause: CrashCause::Panic(panic),
            process_end: Some(process_end),
            report: None,
        },
    )
}

/// Extract panic message from captured stderr lines.
pub(crate) fn extract_panic(lines: &[String]) -> Option<PanicInfo> {
    for (line_index, line) in lines.iter().enumerate() {
        // Format: "thread 'main' panicked at 'message', file.rs:123:45"
        // Or: "thread 'main' panicked at file.rs:123:45:\nmessage"
        if let Some(idx) = line.find("panicked at") {
            let after = &line[idx + 11..].trim_start();

            // Try to extract message in quotes: panicked at 'message'
            if after.starts_with('\'')
                && let Some(end) = after[1..].find('\'')
            {
                let message = &after[1..=end];
                // Also try to get location
                let location = after[end + 2..].trim_start_matches(", ").trim();
                return Some(PanicInfo {
                    payload: message.to_string(),
                    location: (!location.is_empty()).then(|| location.to_string()),
                });
            }

            // Try newer format: panicked at file.rs:123:45:
            // Message is on the next line
            if after.ends_with(':') {
                let location = after.trim_end_matches(':');
                for next_line in lines.iter().skip(line_index + 1) {
                    let msg = next_line.trim();
                    if !msg.is_empty()
                        && !msg.starts_with("note:")
                        && !msg.starts_with("stack backtrace:")
                    {
                        return Some(PanicInfo {
                            payload: msg.to_string(),
                            location: Some(location.to_string()),
                        });
                    }
                }
                let message_end = idx + "panicked".len();
                return Some(PanicInfo {
                    payload: line[..message_end].trim().to_string(),
                    location: Some(location.to_string()),
                });
            }

            if let Some((location, message)) = split_single_line_panic_location(after) {
                return Some(PanicInfo {
                    payload: message.to_string(),
                    location: Some(location.to_string()),
                });
            }
            return Some(PanicInfo {
                payload: after.to_string(),
                location: None,
            });
        }
    }
    None
}

fn split_single_line_panic_location(after: &str) -> Option<(&str, &str)> {
    after.match_indices(": ").find_map(|(separator, _)| {
        let location = &after[..separator];
        let mut parts = location.rsplitn(3, ':');
        parts.next()?.parse::<usize>().ok()?;
        parts.next()?.parse::<usize>().ok()?;
        let file = parts.next()?;
        (!file.is_empty()).then_some((location, &after[separator + 2..]))
    })
}

/// Parse log level from a line of output.
fn parse_log_level(line: &str) -> tracing::Level {
    let line_lower = line.to_lowercase();
    if line_lower.contains("error") || line_lower.contains("fatal") || line_lower.contains("panic")
    {
        tracing::Level::ERROR
    } else if line_lower.contains("warn") {
        tracing::Level::WARN
    } else if line_lower.contains("debug") {
        tracing::Level::DEBUG
    } else if line_lower.contains("trace") {
        tracing::Level::TRACE
    } else {
        tracing::Level::INFO
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitStatus;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use smol::channel::unbounded;

    #[cfg(unix)]
    use super::test_support::{ProcessGroupGuard, term_ignoring_fixture};
    use super::{
        ApplicationExit, ApplicationExitReason, Crash, CrashCause, DeviceEvent, PanicInfo, Running,
        emit_process_exit_event, extract_panic, parse_log_level,
    };
    #[cfg(unix)]
    use super::{ProcessEnd, process_crash_cause};
    #[cfg(target_os = "macos")]
    use super::{command_app_bundle_path_for_executable, command_runs_executable};

    #[cfg(unix)]
    fn successful_exit_status() -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(0)
    }

    #[cfg(windows)]
    fn successful_exit_status() -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(0)
    }

    #[cfg(unix)]
    fn failing_exit_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn failing_exit_status(code: u32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(code)
    }

    #[test]
    fn application_exit_messages_are_reason_specific() {
        assert_eq!(
            ApplicationExit::completed().reason(),
            ApplicationExitReason::Completed
        );
        assert_eq!(
            ApplicationExit::completed().terminal_message(),
            "Application exited"
        );
        assert_eq!(
            ApplicationExit::user_closed().reason(),
            ApplicationExitReason::UserClosed
        );
        assert_eq!(
            ApplicationExit::user_closed().terminal_message(),
            "Application closed"
        );
    }

    #[test]
    fn successful_binary_status_emits_completed_exit() {
        let (sender, receiver) = unbounded();
        emit_process_exit_event(
            &sender,
            Ok(successful_exit_status()),
            None,
            ApplicationExit::completed(),
        );

        let event = receiver
            .try_recv()
            .expect("successful status should emit an event");
        let DeviceEvent::Exited(exit) = event else {
            panic!("successful status should emit a clean exit");
        };
        assert_eq!(exit.reason(), ApplicationExitReason::Completed);
    }

    #[test]
    fn successful_gui_status_emits_user_closed_exit() {
        let (sender, receiver) = unbounded();
        emit_process_exit_event(
            &sender,
            Ok(successful_exit_status()),
            None,
            ApplicationExit::user_closed(),
        );

        let event = receiver
            .try_recv()
            .expect("successful status should emit an event");
        let DeviceEvent::Exited(exit) = event else {
            panic!("successful status should emit a clean exit");
        };
        assert_eq!(exit.reason(), ApplicationExitReason::UserClosed);
    }

    #[test]
    fn failing_binary_status_with_a_panic_emits_the_panic() {
        let (sender, receiver) = unbounded();
        let panic = PanicInfo {
            payload: "backend panic".to_string(),
            location: Some("src/lib.rs:3:5".to_string()),
        };
        emit_process_exit_event(
            &sender,
            Ok(failing_exit_status(7)),
            Some(panic.clone()),
            ApplicationExit::completed(),
        );

        let event = receiver
            .try_recv()
            .expect("failing status should emit an event");
        assert!(matches!(
            event,
            DeviceEvent::Crashed(Crash { cause: CrashCause::Panic(reported), .. }) if reported == panic
        ));
    }

    #[test]
    fn failing_binary_status_without_a_panic_emits_its_exit_code() {
        let (sender, receiver) = unbounded();
        emit_process_exit_event(
            &sender,
            Ok(failing_exit_status(7)),
            None,
            ApplicationExit::completed(),
        );

        let event = receiver
            .try_recv()
            .expect("failing status should emit an event");
        assert!(matches!(
            event,
            DeviceEvent::Crashed(Crash {
                cause: CrashCause::ExitCode(7),
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn crash_display_includes_process_end() {
        let crash = Crash {
            cause: CrashCause::ExitCode(101),
            process_end: Some(ProcessEnd::Signal(nix::libc::SIGABRT)),
            report: None,
        };
        assert!(
            crash
                .to_string()
                .contains("process terminated with signal 6 (SIGABRT)")
        );
    }

    #[cfg(unix)]
    #[test]
    fn panic_crash_retains_signal_process_end() {
        use std::os::unix::process::ExitStatusExt;

        let panic = PanicInfo {
            payload: "backend panic".to_string(),
            location: None,
        };
        let crash = process_crash_cause(
            ExitStatus::from_raw(nix::libc::SIGABRT),
            Some(panic.clone()),
        );
        assert!(matches!(crash.cause, CrashCause::Panic(reported) if reported == panic));
        assert_eq!(
            crash.process_end,
            Some(ProcessEnd::Signal(nix::libc::SIGABRT))
        );
    }

    #[cfg(unix)]
    #[test]
    fn signal_crash_with_report_displays_both_details() {
        let crash = Crash {
            cause: CrashCause::Signal(nix::libc::SIGABRT),
            process_end: Some(ProcessEnd::Signal(nix::libc::SIGABRT)),
            report: Some(Box::new(super::CrashReport::new(
                jiff::Timestamp::now(),
                "test device",
                "test-device",
                "test.app",
                std::path::PathBuf::from("/tmp/crash.ips"),
                "test crash summary",
            ))),
        };
        let formatted = crash.to_string();
        assert!(formatted.contains("Process crashed (SIGABRT)"));
        assert!(formatted.contains("process terminated with signal 6 (SIGABRT)"));
        assert!(formatted.contains("Crash report: /tmp/crash.ips"));
        assert_eq!(
            crash.panic_note().as_deref(),
            Some("process terminated with signal 6 (SIGABRT); test crash summary")
        );
    }

    #[test]
    fn extract_panic_splits_single_line_location() {
        let lines = ["thread 'main' panicked at src/main.rs:5:5: boom".to_string()];
        let panic = extract_panic(&lines).expect("extract the panic");
        assert_eq!(panic.payload, "boom");
        assert_eq!(panic.location.as_deref(), Some("src/main.rs:5:5"));
    }

    #[test]
    fn extract_panic_reads_after_the_matching_line() {
        let lines = [
            "previous stderr line".to_string(),
            "thread 'worker' panicked at src/lib.rs:8:3:".to_string(),
            "note: panic formatting detail".to_string(),
            "the actual panic message".to_string(),
        ];
        let panic = extract_panic(&lines).expect("extract the panic");
        assert_eq!(panic.payload, "the actual panic message");
        assert_eq!(panic.location.as_deref(), Some("src/lib.rs:8:3"));
    }

    #[test]
    fn extract_panic_without_following_message_omits_location_from_payload() {
        let lines = ["thread 'main' panicked at src/main.rs:5:5:".to_string()];
        let panic = extract_panic(&lines).expect("extract the panic");
        assert_eq!(panic.payload, "thread 'main' panicked");
        assert_eq!(panic.location.as_deref(), Some("src/main.rs:5:5"));
    }

    #[test]
    fn parse_log_level_detects_panic_as_error() {
        assert_eq!(
            parse_log_level("thread panicked at app.rs"),
            tracing::Level::ERROR
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_command_extracts_app_path_with_spaces() {
        let app_path = command_app_bundle_path_for_executable(
            "/tmp/water build/My App.app/Contents/MacOS/my-app --flag",
            "my-app",
        )
        .expect("app path should be extracted");
        assert_eq!(
            app_path,
            std::path::PathBuf::from("/tmp/water build/My App.app")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_command_rejects_nonmatching_executable_prefix() {
        assert!(
            command_app_bundle_path_for_executable(
                "/tmp/My App.app/Contents/MacOS/my-app-helper",
                "my-app",
            )
            .is_none()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_command_matches_exact_executable_path() {
        let executable = std::path::Path::new("/tmp/My App.app/Contents/MacOS/my-app");
        assert!(command_runs_executable(
            "/tmp/My App.app/Contents/MacOS/my-app --flag",
            executable,
        ));
    }

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn dropping_a_running_fires_retained_guards() {
        let fired = Arc::new(AtomicBool::new(false));
        let (mut running, _sender, control) = Running::new();
        running.retain(DropProbe(fired.clone()));
        drop(control);
        drop(running);
        assert!(fired.load(Ordering::SeqCst));
    }

    #[test]
    fn dropping_an_unsupervised_running_queues_a_kill_without_waiting() {
        let (mut running, _sender, control) = Running::new();
        let fired = Arc::new(AtomicBool::new(false));
        running.retain(DropProbe(fired.clone()));

        drop(running);

        assert_eq!(control.try_recv(), Ok(super::StopRequest::Kill));
        assert!(
            fired.load(Ordering::SeqCst),
            "drop releases retained resources without waiting for the monitor"
        );
    }

    #[test]
    fn shutdown_awaits_the_monitors_terminal_event() {
        struct AckProbe(Arc<AtomicBool>, Arc<AtomicBool>);

        impl Drop for AckProbe {
            fn drop(&mut self) {
                self.1
                    .store(self.0.load(Ordering::SeqCst), Ordering::SeqCst);
            }
        }

        let (mut running, sender, control) = Running::new();
        let acknowledged = Arc::new(AtomicBool::new(false));
        let hook_observed_ack = Arc::new(AtomicBool::new(false));
        running.retain(AckProbe(acknowledged.clone(), hook_observed_ack.clone()));
        let monitor = async {
            assert_eq!(
                control.recv().await,
                Ok(super::StopRequest::Terminate),
                "shutdown delivers the requested stop"
            );
            sender
                .send(DeviceEvent::Log {
                    level: tracing::Level::INFO,
                    message: "on-terminate".to_string(),
                })
                .await
                .expect("monitor forwards the shutdown output");
            acknowledged.store(true, Ordering::SeqCst);
            sender
                .send(DeviceEvent::Exited(ApplicationExit::completed()))
                .await
                .expect("monitor acknowledges termination");
        };

        smol::block_on(futures_util::future::join(
            running.shutdown(super::StopRequest::Terminate),
            monitor,
        ));

        assert!(
            hook_observed_ack.load(Ordering::SeqCst),
            "retained resources outlive the monitor's terminal event"
        );
    }

    #[test]
    fn detach_keeps_retained_guards_from_firing() {
        // A retained RAII guard — the `adb forward` teardown is one — must
        // survive detach: the detached app outlives the session and keeps
        // serving through the forwarded ports.
        let fired = Arc::new(AtomicBool::new(false));
        let (mut running, _sender, control) = Running::new();
        running.retain(DropProbe(fired.clone()));
        let mut running = Box::pin(running);
        running.as_mut().detach();
        drop(control);
        drop(running);
        assert!(!fired.load(Ordering::SeqCst));
    }

    /// The preview support app outlives `water preview`; a pipe whose reader
    /// has exited turns the app's next write into SIGPIPE (water-rs/cli#197).
    /// With an app log file set, the spawned child must carry no pipes.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_app_stdio_lands_in_the_log_file_not_pipes() {
        use futures_util::future::{Either, select};
        use std::time::Duration;

        smol::block_on(async {
            let machine = crate::toolchain::testing::TestMachine::new();
            let host = machine.host(Vec::<(String, String)>::new());
            let script = machine.root().join("emit");
            std::fs::write(
                &script,
                "#!/bin/sh\nprintf 'out-line\\n'\nprintf 'err-line\\n' >&2\n",
            )
            .expect("write emitter script");
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                    .expect("mark emitter executable");
            }
            let log = machine.root().join("logs/support-app.log");

            let mut command = host.command(&script);
            let redirected = super::configure_app_stdio(&mut command, Some(&log))
                .expect("configure app log-file stdio");
            assert_eq!(redirected.as_deref(), Some(log.as_path()));
            let mut child = command.spawn().expect("spawn the emitter");
            assert!(
                child.stdout.is_none() && child.stderr.is_none(),
                "a log-file app's stdio must be files, not pipes"
            );
            let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(5)));
            let status = match select(Box::pin(child.status()), deadline).await {
                Either::Left((status, _)) => status.expect("await the emitter"),
                Either::Right(_) => panic!("stdio emitter exceeded five seconds"),
            };
            assert!(status.success());
            let contents = std::fs::read_to_string(&log).expect("read the app log");
            assert!(contents.contains("out-line") && contents.contains("err-line"));

            // Supervised apps (no log file) keep the pipes the monitor
            // forwards.
            let mut piped = host.command(&script);
            super::configure_app_stdio(&mut piped, None).expect("configure piped stdio");
            let mut piped_child = piped.spawn().expect("spawn the piped emitter");
            assert!(piped_child.stdout.is_some() && piped_child.stderr.is_some());
            let _ = piped_child.kill();
            let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(5)));
            match select(Box::pin(piped_child.status()), deadline).await {
                Either::Left((status, _)) => {
                    let _ = status.expect("reap the piped emitter");
                }
                Either::Right(_) => panic!("piped emitter exceeded five seconds"),
            }
        });
    }

    /// Everything a signal-counting fixture `.app` needs: the file its
    /// `SIGTERM` trap appends one `t` to per delivery, the fifo whose read
    /// side the app blocks on until a "done" line arrives, and the
    /// artifact to hand `run_macos_app`.
    #[cfg(target_os = "macos")]
    struct SignalCountingApp {
        marker: std::path::PathBuf,
        fifo: std::path::PathBuf,
        artifact: super::Artifact,
    }

    /// Build the fixture under `machine`. The executable traps `SIGTERM`
    /// and appends one `t` to `marker` on every delivery — it does not
    /// exit on the signal — then blocks reading `fifo` until the test
    /// writes the done line.
    #[cfg(target_os = "macos")]
    fn signal_counting_app(
        machine: &crate::toolchain::testing::TestMachine,
        exit_on_term: bool,
    ) -> SignalCountingApp {
        use std::os::unix::fs::PermissionsExt as _;

        let marker = machine.root().join("term-count");
        let fifo = machine.root().join("done-fifo");
        machine.file(
            "Fixture.app/Contents/Info.plist",
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
                "<plist version=\"1.0\"><dict>",
                "<key>CFBundleExecutable</key><string>fixture-app</string>",
                "</dict></plist>",
            ),
        );
        let runner = machine.file(
            "Fixture.app/Contents/MacOS/fixture-app",
            &format!(
                "#!/bin/sh\n\
                 echo pid=$$ >&2\n\
                 trap 'echo t >> \"{marker}\"; echo term-seen >&2; {on_term}' TERM\n\
                 echo trap-armed >&2\n\
                 while ! IFS= read -r _done < \"{fifo}\"; do :; done\n\
                 echo hook-ran >&2\n\
                 exit 42\n",
                marker = marker.display(),
                fifo = fifo.display(),
                on_term = if exit_on_term { "exit 42" } else { ":" },
            ),
        );
        std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o755))
            .expect("mark the fixture executable runnable");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU)
            .expect("mkfifo the done-line fifo");

        SignalCountingApp {
            marker,
            fifo,
            artifact: super::Artifact::new(
                "dev.waterui.fixture",
                machine.root().join("Fixture.app"),
            ),
        }
    }

    #[test]
    fn join_output_ends_on_a_kill_request() {
        let (control_sender, control_receiver) = unbounded();
        control_sender
            .try_send(super::StopRequest::Kill)
            .expect("queue kill request");
        let mut control = Some(control_receiver);

        assert_eq!(
            smol::block_on(super::join_output(
                std::future::pending::<()>(),
                &mut control
            )),
            super::OutputJoin::KillRequested
        );
    }

    #[test]
    fn join_output_ignores_terminate_then_finishes() {
        let (control_sender, control_receiver) = unbounded();
        control_sender
            .try_send(super::StopRequest::Terminate)
            .expect("queue terminate request");
        let mut control = Some(control_receiver);
        let mut polls = 0;
        let output = futures_util::future::poll_fn(|_| {
            polls += 1;
            if polls == 1 {
                std::task::Poll::Pending
            } else {
                std::task::Poll::Ready(())
            }
        });

        assert_eq!(
            smol::block_on(super::join_output(output, &mut control)),
            super::OutputJoin::Finished
        );
    }

    #[test]
    fn join_output_gives_up_on_output_held_by_others() {
        assert_eq!(
            smol::block_on(super::join_output(std::future::pending::<()>(), &mut None)),
            super::OutputJoin::OutlivedByOthers
        );
    }

    #[cfg(unix)]
    #[test]
    fn natural_exit_ends_the_run_while_a_grandchild_holds_its_output() {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;
        use std::{os::unix::fs::PermissionsExt as _, time::Duration};

        let machine = crate::toolchain::testing::TestMachine::new();
        let fifo = machine.root().join("grandchild-fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU)
            .expect("mkfifo the grandchild's blocking fifo");
        let script = machine.file(
            "grandchild-holds-output",
            &format!(
                "#!/bin/sh\n\
                 echo pid=$$ >&2\n\
                 ( while ! IFS= read -r _done < \"{}\"; do :; done ) &\n\
                 echo started >&2\n\
                 exit 0\n",
                fifo.display()
            ),
        );
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");

        let host = machine.host(Vec::<(String, String)>::new());
        let running = super::run_binary_with_grace(
            &host,
            &super::Artifact::new("dev.waterui.fixture", script),
            &super::RunOptions::new(),
            Duration::from_secs(3600),
        )
        .expect("launch the grandchild fixture");
        let (_interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let app_group = ProcessGroupGuard::default();
        let event_group = app_group;
        let exercise = async move {
            let mut events = std::pin::pin!(running.supervise(interrupt_rx));
            let mut started = false;
            let mut exited = false;
            let mut crashed = false;
            let mut monitor_error = false;
            let mut pid = None;
            while let Some(event) = events.next().await {
                match event {
                    DeviceEvent::Stderr { message } => {
                        if let Some(found) = message
                            .strip_prefix("pid=")
                            .and_then(|pid| pid.parse::<i32>().ok())
                        {
                            let found_pid = nix::unistd::Pid::from_raw(found);
                            event_group.set(found_pid);
                            pid = Some(found_pid);
                        }
                        if message.contains("started") {
                            started = true;
                        }
                    }
                    DeviceEvent::Exited(_) => {
                        exited = true;
                        break;
                    }
                    DeviceEvent::Crashed(_) => {
                        crashed = true;
                        break;
                    }
                    DeviceEvent::MonitorError { .. } => monitor_error = true,
                    _ => {}
                }
            }
            (started, exited, crashed, monitor_error, pid)
        };
        let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(30)));
        let (started, exited, crashed, monitor_error, pid) = smol::block_on(async {
            match select(Box::pin(exercise), deadline).await {
                Either::Left((result, _)) => result,
                Either::Right(_) => panic!("natural exit exceeded 30 seconds"),
            }
        });

        assert!(started, "fixture reports it started before exiting");
        assert!(exited, "the natural exit is reported as Exited");
        assert!(!crashed, "a clean app exit is not reported as Crashed");
        assert!(
            !monitor_error,
            "a held-open output pipe is not a monitor error"
        );
        assert!(pid.is_some(), "fixture prints its pid");
    }

    #[cfg(unix)]
    #[test]
    fn second_interrupt_kills_a_child_that_ignores_term() {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;
        use std::time::Duration;

        let machine = crate::toolchain::testing::TestMachine::new();
        let script = term_ignoring_fixture(&machine);
        let host = machine.host(Vec::<(String, String)>::new());
        let running = super::run_binary_with_grace(
            &host,
            &super::Artifact::new("dev.waterui.fixture", script),
            &super::RunOptions::new(),
            Duration::from_secs(3600),
        )
        .expect("launch the TERM-ignoring fixture");
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let app_group = ProcessGroupGuard::default();
        let group_guard = app_group;
        let exercise = async move {
            let mut events = std::pin::pin!(running.supervise(interrupt_rx));
            let mut armed = false;
            let mut term_seen = false;
            let mut stopped = false;
            let mut monitor_error = false;
            let mut pid = None;
            while let Some(event) = events.next().await {
                match event {
                    DeviceEvent::Stderr { message } => {
                        if let Some(found) = message
                            .strip_prefix("pid=")
                            .and_then(|pid| pid.parse::<i32>().ok())
                        {
                            pid = Some(nix::unistd::Pid::from_raw(found));
                            group_guard.set(nix::unistd::Pid::from_raw(found));
                        }
                        if !armed && message.contains("trap-armed") {
                            armed = true;
                            interrupt_tx.try_send(()).expect("request termination");
                        }
                        if !term_seen && message.contains("term-seen") {
                            term_seen = true;
                            interrupt_tx.try_send(()).expect("request immediate kill");
                        }
                    }
                    DeviceEvent::MonitorError { .. } => monitor_error = true,
                    DeviceEvent::Stopped => {
                        stopped = true;
                        break;
                    }
                    DeviceEvent::Exited(_) | DeviceEvent::Crashed(_) => break,
                    _ => {}
                }
            }
            (armed, term_seen, stopped, monitor_error, pid)
        };
        let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(30)));
        let (armed, term_seen, stopped, monitor_error, pid) = smol::block_on(async {
            match select(Box::pin(exercise), deadline).await {
                Either::Left((result, _)) => result,
                Either::Right(_) => panic!("TERM escalation exceeded 30 seconds"),
            }
        });
        assert!(armed, "fixture trap arms before the first interrupt");
        assert!(term_seen, "fixture receives SIGTERM and remains alive");
        assert!(stopped, "second interrupt waits for the monitor's kill ack");
        assert!(
            !monitor_error,
            "a kill the user asked for is not a monitor error"
        );
        let pid = pid.expect("fixture prints its pid");
        assert_eq!(
            nix::sys::signal::kill(pid, None),
            Err(nix::errno::Errno::ESRCH),
            "Stopped follows the confirmed child exit and reap"
        );
    }

    /// A child that outlives the termination grace is killed, and the
    /// overrun reaches the run as a monitor error before `Stopped`.
    #[cfg(unix)]
    #[test]
    fn grace_overrun_reports_an_error_then_kills() {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;
        use std::time::Duration;

        let machine = crate::toolchain::testing::TestMachine::new();
        let script = term_ignoring_fixture(&machine);
        let host = machine.host(Vec::<(String, String)>::new());
        let running = super::run_binary_with_grace(
            &host,
            &super::Artifact::new("dev.waterui.fixture", script),
            &super::RunOptions::new(),
            Duration::from_millis(200),
        )
        .expect("launch the TERM-ignoring fixture");
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let group_guard = ProcessGroupGuard::default();
        let exercise = async move {
            let mut events = std::pin::pin!(running.supervise(interrupt_rx));
            let mut overrun = None;
            let mut stopped = false;
            let mut pid = None;
            while let Some(event) = events.next().await {
                match event {
                    DeviceEvent::Stderr { message } => {
                        if let Some(found) = message
                            .strip_prefix("pid=")
                            .and_then(|pid| pid.parse::<i32>().ok())
                        {
                            pid = Some(nix::unistd::Pid::from_raw(found));
                            group_guard.set(nix::unistd::Pid::from_raw(found));
                        }
                        if message.contains("trap-armed") {
                            interrupt_tx.try_send(()).expect("request termination");
                        }
                    }
                    DeviceEvent::MonitorError { message } => overrun = Some(message),
                    DeviceEvent::Stopped => {
                        stopped = true;
                        break;
                    }
                    DeviceEvent::Exited(_) | DeviceEvent::Crashed(_) => break,
                    _ => {}
                }
            }
            (overrun, stopped, pid)
        };
        let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(30)));
        let (overrun, stopped, pid) = smol::block_on(async {
            match select(Box::pin(exercise), deadline).await {
                Either::Left((result, _)) => result,
                Either::Right(_) => panic!("grace overrun exceeded 30 seconds"),
            }
        });
        let overrun = overrun.expect("the grace overrun is reported before Stopped");
        assert!(
            overrun.contains("did not exit within the 200ms termination grace period"),
            "the error names the grace overrun: {overrun}"
        );
        assert!(stopped, "the overrun still ends in Stopped");
        let pid = pid.expect("fixture prints its pid");
        assert_eq!(
            nix::sys::signal::kill(pid, None),
            Err(nix::errno::Errno::ESRCH),
            "the monitor killed and reaped the child after the grace"
        );
    }

    /// `Ctrl-C` semantics through the real `run_macos_app`: a fabricated
    /// `.app` whose executable counts every `SIGTERM` it receives proves
    /// the app gets exactly one signal, that the monitor waits on the
    /// real exit — the app stays alive, blocked on its done line, until
    /// the test releases it — and that the run keeps streaming shutdown
    /// output until the monitor reports `Stopped`.
    #[cfg(target_os = "macos")]
    #[test]
    fn stopping_a_macos_app_signals_once_then_reports_stopped() {
        use futures_util::future::{Either, select};
        use std::time::Duration;

        let machine = crate::toolchain::testing::TestMachine::new();
        machine.install("log");
        machine.install("logger");
        let host = machine.host(Vec::<(String, String)>::new());
        let fixture = signal_counting_app(&machine, false);
        let fifo = fixture.fifo.clone();
        let marker = fixture.marker.clone();

        let mut options = super::RunOptions::new();
        options.set_replace_existing_macos_app_instances(false);

        let launch = super::run_macos_app_with_grace(
            &host,
            fixture.artifact,
            options,
            Duration::from_secs(3600),
        );
        let launch_deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(30)));
        let running = smol::block_on(async {
            match select(Box::pin(launch), launch_deadline).await {
                Either::Left((running, _)) => running.expect("the fixture app must launch"),
                Either::Right(_) => panic!("macOS app launch exceeded 30 seconds"),
            }
        });
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let app_group = ProcessGroupGuard::default();
        let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(30)));
        let (armed, term_seen, saw_hook_line, stopped) = smol::block_on(async {
            match select(
                Box::pin(exercise_macos_app_stop(
                    running,
                    interrupt_tx,
                    interrupt_rx,
                    fifo,
                    app_group,
                )),
                deadline,
            )
            .await
            {
                Either::Left((result, _)) => result,
                Either::Right(_) => panic!("macOS app termination exceeded 30 seconds"),
            }
        });

        assert!(armed, "the fixture's trap must arm before the run ends");
        assert!(term_seen, "the app's one SIGTERM must be observed");
        assert!(saw_hook_line, "the app's shutdown output must still stream");
        assert!(stopped, "a stopped run must end on a stopped event");
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap_or_default(),
            "t\n",
            "exactly one termination signal may reach the app"
        );
    }

    #[cfg(target_os = "macos")]
    async fn exercise_macos_app_stop(
        running: Running,
        interrupt_tx: async_channel::Sender<()>,
        interrupt_rx: async_channel::Receiver<()>,
        fifo: std::path::PathBuf,
        app_group: ProcessGroupGuard,
    ) -> (bool, bool, bool, bool) {
        use futures_util::FutureExt as _;
        use smol::stream::StreamExt as _;

        let mut events = Box::pin(running.supervise(interrupt_rx));
        let mut armed = false;
        let mut term_seen = false;
        let mut saw_hook_line = false;
        let mut stopped = false;
        while let Some(event) = events.next().await {
            match event {
                super::DeviceEvent::Stderr { message } => {
                    if let Some(pid) = message
                        .strip_prefix("pid=")
                        .and_then(|pid| pid.parse::<i32>().ok())
                    {
                        app_group.set(nix::unistd::Pid::from_raw(pid));
                    }
                    if !armed && message.contains("trap-armed") {
                        armed = true;
                        interrupt_tx.try_send(()).expect("request termination");
                    }
                    if !term_seen && message.contains("term-seen") {
                        term_seen = true;
                        loop {
                            match events.next().now_or_never() {
                                None => break,
                                Some(Some(event)) => assert!(
                                    !matches!(
                                        event,
                                        super::DeviceEvent::Stopped
                                            | super::DeviceEvent::Exited(_)
                                            | super::DeviceEvent::Crashed(_)
                                    ),
                                    "the app remains alive while it waits on its done line"
                                ),
                                Some(None) => {
                                    panic!("the app stream ended before the done line")
                                }
                            }
                        }
                        let (fifo_tx, fifo_rx) = std::sync::mpsc::channel();
                        let fifo_path = fifo.clone();
                        std::thread::spawn(move || {
                            let _ = fifo_tx.send(std::fs::write(fifo_path, "done\n"));
                        });
                        fifo_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .expect("fifo write is bounded")
                            .expect("the live app's fifo read must take the done line");
                    }
                    if message.contains("hook-ran") {
                        saw_hook_line = true;
                    }
                }
                super::DeviceEvent::Stopped => {
                    stopped = true;
                    break;
                }
                super::DeviceEvent::Exited(_) | super::DeviceEvent::Crashed(_) => break,
                _ => {}
            }
        }
        (armed, term_seen, saw_hook_line, stopped)
    }

    /// A terminal-style SIGINT is delivered only to water; it forwards one
    /// SIGTERM and drains the app's `os_log` termination hook before Stopped.
    #[cfg(target_os = "macos")]
    const SIGINT_ROLE_ENV: &str = "WATERUI_1934_FIXTURE_ROLE";
    #[cfg(target_os = "macos")]
    const SIGINT_BUNDLE_ENV: &str = "WATERUI_1934_FIXTURE_BUNDLE";
    #[cfg(target_os = "macos")]
    const SIGINT_MARKER_ENV: &str = "WATERUI_1934_FIXTURE_MARKER";
    /// The fifo both fixture roles report their events on, one line each.
    #[cfg(target_os = "macos")]
    const SIGINT_EVENTS_ENV: &str = "WATERUI_1934_FIXTURE_EVENTS";

    #[cfg(target_os = "macos")]
    fn open_sigint_events(current: &crate::toolchain::Host) -> std::fs::File {
        let events = current
            .env_string(SIGINT_EVENTS_ENV)
            .expect("fixture events fifo path");
        std::fs::OpenOptions::new()
            .write(true)
            .open(events)
            .expect("open the fixture events fifo")
    }

    #[cfg(target_os = "macos")]
    fn send_sigint_event(events: &mut std::fs::File, line: &str) {
        use std::io::Write as _;

        events
            .write_all(format!("{line}\n").as_bytes())
            .expect("write a fixture event");
    }
    #[cfg(target_os = "macos")]
    const SIGINT_TEST_NAME: &str =
        "workflows::device::tests::sigint_to_water_process_group_drains_on_terminate_logs";
    #[cfg(target_os = "macos")]
    const SIGINT_TEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

    #[cfg(target_os = "macos")]
    fn run_sigint_fixture_role(current: &crate::toolchain::Host) -> bool {
        match current.env_string(SIGINT_ROLE_ENV).as_deref() {
            Some("app") => {
                run_sigint_app_role(current);
                true
            }
            Some("water") => run_sigint_water_role(current),
            _ => false,
        }
    }

    #[cfg(target_os = "macos")]
    fn run_sigint_app_role(current: &crate::toolchain::Host) {
        use futures_util::future::{Either, select};
        use tracing_subscriber::prelude::*;

        tracing_subscriber::registry()
            .with(tracing_oslog::OsLogger::new("dev.waterui", "default"))
            .try_init()
            .expect("install the fixture's os_log subscriber");
        let marker = current
            .env_string(SIGINT_MARKER_ENV)
            .expect("fixture marker path");
        let (signal_tx, signal_rx) = async_channel::unbounded();
        ctrlc::set_handler(move || {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&marker)
                .and_then(|mut marker| {
                    use std::io::Write as _;
                    marker.write_all(b"t\n")
                });
            let _ = signal_tx.try_send(());
        })
        .expect("install the app's SIGTERM handler");
        let mut events = open_sigint_events(current);
        send_sigint_event(&mut events, &format!("pid={}", std::process::id()));
        send_sigint_event(&mut events, "trap-armed");
        tracing::info!("fixture-app-ready");
        let signal = std::pin::pin!(signal_rx.recv());
        let deadline = std::pin::pin!(smol::Timer::after(SIGINT_TEST_DEADLINE));
        smol::block_on(async {
            match select(signal, deadline).await {
                Either::Left((Ok(()), _)) => {}
                Either::Left((Err(error), _)) => panic!("signal channel closed: {error}"),
                Either::Right(_) => panic!("app did not receive SIGTERM within 60 seconds"),
            }
        });
        tracing::info!("on-terminate-hook-ran");
        std::process::exit(0);
    }

    #[cfg(target_os = "macos")]
    fn run_sigint_water_role(current: &crate::toolchain::Host) -> bool {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;

        let host = current.clone();
        let mut events = open_sigint_events(current);
        let events_path = current
            .env_string(SIGINT_EVENTS_ENV)
            .expect("fixture events fifo path");
        let bundle = current
            .env_string(SIGINT_BUNDLE_ENV)
            .expect("fixture bundle path");
        let marker = current
            .env_string(SIGINT_MARKER_ENV)
            .expect("fixture marker path");
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        ctrlc::set_handler(move || {
            let _ = interrupt_tx.try_send(());
        })
        .expect("install the supervisor's SIGINT handler");
        smol::block_on(async {
            let mut options = super::RunOptions::new();
            options.set_log_level(super::LogLevel::Debug);
            options.set_replace_existing_macos_app_instances(false);
            options.insert_env_var(SIGINT_ROLE_ENV.to_string(), "app".to_string());
            options.insert_env_var(SIGINT_MARKER_ENV.to_string(), marker);
            options.insert_env_var(SIGINT_EVENTS_ENV.to_string(), events_path);
            let launch = super::run_macos_app(
                &host,
                super::Artifact::new("dev.waterui.fixture", bundle.into()),
                options,
            );
            let launch_deadline = std::pin::pin!(smol::Timer::after(SIGINT_TEST_DEADLINE));
            let running = match select(Box::pin(launch), launch_deadline).await {
                Either::Left((running, _)) => running.expect("fixture app launches"),
                Either::Right(_) => panic!("fixture app launch exceeded 60 seconds"),
            };
            let run = async {
                let mut stream = std::pin::pin!(running.supervise(interrupt_rx));
                while let Some(event) = stream.next().await {
                    match event {
                        super::DeviceEvent::Log { message, .. } => {
                            send_sigint_event(&mut events, &format!("log:{message}"));
                        }
                        super::DeviceEvent::Stopped => send_sigint_event(&mut events, "stopped"),
                        _ => {}
                    }
                }
            };
            let deadline = std::pin::pin!(smol::Timer::after(SIGINT_TEST_DEADLINE));
            match select(Box::pin(run), deadline).await {
                Either::Left(((), _)) => {}
                Either::Right(_) => panic!("supervised app exceeded 60 seconds"),
            }
        });
        true
    }

    #[cfg(target_os = "macos")]
    struct SigintLogFixture {
        _machine: crate::toolchain::testing::TestMachine,
        marker: std::path::PathBuf,
        events: std::path::PathBuf,
        host: crate::toolchain::Host,
    }

    #[cfg(target_os = "macos")]
    fn prepare_sigint_log_fixture(current: &crate::toolchain::Host) -> SigintLogFixture {
        use std::os::unix::fs::PermissionsExt as _;

        let machine = crate::toolchain::testing::TestMachine::new();
        // The supervisor runs the real macOS tools its app monitor needs,
        // resolved on this machine and linked into the declared PATH.
        for tool in ["log", "logger", "kill"] {
            let real = smol::block_on(current.which(tool)).expect("macOS provides the tool");
            std::os::unix::fs::symlink(real, machine.bin().join(tool))
                .expect("link the real tool into the fixture PATH");
        }
        let marker = machine.root().join("term-count");
        let events = machine.root().join("events");
        nix::unistd::mkfifo(&events, nix::sys::stat::Mode::S_IRWXU)
            .expect("create the fixture events fifo");
        let bundle = machine.dir("Fixture.app");
        machine.file(
            "Fixture.app/Contents/Info.plist",
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
                "<plist version=\"1.0\"><dict>",
                "<key>CFBundleExecutable</key><string>fixture-app</string>",
                "</dict></plist>",
            ),
        );
        let app = machine.file(
            "Fixture.app/Contents/MacOS/fixture-app",
            &format!(
                "#!/bin/sh\nexec \"{}\" {SIGINT_TEST_NAME} --exact --nocapture\n",
                crate::toolchain::Host::current_exe()
                    .expect("test executable path")
                    .display()
            ),
        );
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755))
            .expect("make fixture app executable");
        let host = machine.host([
            (SIGINT_ROLE_ENV, "water"),
            (
                SIGINT_BUNDLE_ENV,
                bundle.to_str().expect("bundle path is UTF-8"),
            ),
            (
                SIGINT_MARKER_ENV,
                marker.to_str().expect("marker path is UTF-8"),
            ),
            (
                SIGINT_EVENTS_ENV,
                events.to_str().expect("events path is UTF-8"),
            ),
        ]);
        SigintLogFixture {
            _machine: machine,
            marker,
            events,
            host,
        }
    }

    #[cfg(target_os = "macos")]
    fn spawn_sigint_water_helper(
        host: &crate::toolchain::Host,
        events: &std::path::Path,
    ) -> (
        std::process::Child,
        std::sync::mpsc::Receiver<Option<String>>,
        std::thread::JoinHandle<()>,
        ProcessGroupGuard,
    ) {
        use std::{
            io::{BufRead as _, BufReader},
            os::unix::process::CommandExt as _,
            process::Stdio,
            sync::mpsc,
            thread,
        };

        let child = host
            .std_command(crate::toolchain::Host::current_exe().expect("test executable path"))
            .args([SIGINT_TEST_NAME, "--exact", "--nocapture"])
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn water helper");
        let helper_group = ProcessGroupGuard::default();
        helper_group.set(nix::unistd::Pid::from_raw(
            i32::try_from(child.id()).expect("helper pid fits in i32"),
        ));
        let (line_tx, line_rx) = mpsc::channel();
        let events = events.to_path_buf();
        let reader = thread::spawn(move || {
            // Opening a fifo's read end waits for its first writer, so it
            // happens here, under the caller's bounded receive.
            let events = std::fs::File::open(events).expect("open the fixture events fifo");
            for line in BufReader::new(events).lines().map_while(Result::ok) {
                if line_tx.send(Some(line)).is_err() {
                    return;
                }
            }
            let _ = line_tx.send(None);
        });
        (child, line_rx, reader, helper_group)
    }

    #[cfg(target_os = "macos")]
    fn verify_sigint_shutdown(
        mut child: std::process::Child,
        line_rx: &std::sync::mpsc::Receiver<Option<String>>,
        reader: std::thread::JoinHandle<()>,
        marker: &std::path::Path,
        _helper_group: ProcessGroupGuard,
        app_group: &ProcessGroupGuard,
    ) {
        use std::{sync::mpsc, thread, time::Instant};

        let readiness_deadline = Instant::now() + SIGINT_TEST_DEADLINE;
        let mut armed = false;
        let mut app_pid_seen = false;
        let mut app_ready = false;
        // Wait until `log stream` is attached, shown by the app's readiness
        // entry arriving: an entry written before the attach can be lost,
        // since the delayed `log show` replay does not reliably recover it
        // (#2080).
        while !(armed && app_pid_seen && app_ready) {
            let Some(line) = line_rx
                .recv_timeout(readiness_deadline.saturating_duration_since(Instant::now()))
                .expect("fixture readiness is bounded")
            else {
                break;
            };
            if let Some(pid) = line
                .strip_prefix("pid=")
                .and_then(|pid| pid.parse::<i32>().ok())
            {
                app_group.set(nix::unistd::Pid::from_raw(pid));
                app_pid_seen = true;
            }
            if line.contains("trap-armed") {
                armed = true;
            }
            if line.starts_with("log:") && line.contains("fixture-app-ready") {
                app_ready = true;
            }
        }
        assert!(
            app_pid_seen,
            "the fixture app reports its pid for process-group cleanup"
        );
        assert!(
            armed,
            "the fixture must arm its trap before SIGINT goes out"
        );
        assert!(
            app_ready,
            "the fixture app's os_log readiness event is forwarded"
        );

        nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(i32::try_from(child.id()).expect("helper pid fits in i32")),
            nix::sys::signal::Signal::SIGINT,
        )
        .expect("SIGINT the supervisor's process group");

        let output_deadline = Instant::now() + SIGINT_TEST_DEADLINE;
        let mut saw_hook_before_stopped = false;
        let mut saw_stopped = false;
        while let Some(line) = line_rx
            .recv_timeout(output_deadline.saturating_duration_since(Instant::now()))
            .expect("supervisor output is bounded")
        {
            if line.starts_with("log:") && line.contains("on-terminate-hook-ran") && !saw_stopped {
                saw_hook_before_stopped = true;
            }
            if line == "stopped" {
                saw_stopped = true;
            }
        }
        reader.join().expect("helper output reader finishes");
        let (status_tx, status_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = status_tx.send(child.wait());
        });
        let status = status_rx
            .recv_timeout(SIGINT_TEST_DEADLINE)
            .expect("helper exit is bounded")
            .expect("wait for helper");
        assert!(status.success(), "the supervised helper exits cleanly");
        assert!(
            saw_hook_before_stopped,
            "termination logs drain before Stopped"
        );
        assert!(saw_stopped, "the monitor acknowledges termination");
        assert_eq!(
            std::fs::read_to_string(marker).unwrap_or_default(),
            "t\n",
            "the app receives exactly one SIGTERM"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sigint_to_water_process_group_drains_on_terminate_logs() {
        let current = crate::toolchain::Host::current();
        if run_sigint_fixture_role(&current) {
            return;
        }

        let fixture = prepare_sigint_log_fixture(&current);
        let (child, line_rx, reader, helper_group) =
            spawn_sigint_water_helper(&fixture.host, &fixture.events);
        let app_group = ProcessGroupGuard::default();
        verify_sigint_shutdown(
            child,
            &line_rx,
            reader,
            &fixture.marker,
            helper_group,
            &app_group,
        );
    }

    /// The monitor remains the acknowledgment authority even after kill is requested.
    #[cfg(unix)]
    #[test]
    fn supervise_stops_then_escalates_on_a_second_interrupt() {
        use futures_util::FutureExt as _;
        use smol::stream::StreamExt as _;

        let (running, sender, control) = Running::new();
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let mut events = Box::pin(running.supervise(interrupt_rx));

        smol::block_on(async {
            assert!(
                matches!(events.next().await, Some(super::DeviceEvent::Started)),
                "the run's first event is Started"
            );

            interrupt_tx.try_send(()).expect("fire the interrupt");
            assert!(events.next().now_or_never().is_none());
            assert!(
                matches!(control.try_recv(), Ok(super::StopRequest::Terminate)),
                "the first interrupt requests graceful termination"
            );

            sender
                .send(super::DeviceEvent::Log {
                    level: tracing::Level::INFO,
                    message: "shutdown line".to_string(),
                })
                .await
                .expect("queue a shutdown event");
            assert!(
                matches!(events.next().await, Some(super::DeviceEvent::Log { .. })),
                "shutdown output streams while termination is pending"
            );

            interrupt_tx
                .try_send(())
                .expect("fire the second interrupt");
            assert!(
                events.next().now_or_never().is_none(),
                "the stream stays alive until the monitor acknowledges termination"
            );
            assert!(
                matches!(control.try_recv(), Ok(super::StopRequest::Kill)),
                "the second interrupt sends the monitor's kill request"
            );
            sender
                .send(super::DeviceEvent::Crashed(Crash::new(CrashCause::Signal(
                    nix::sys::signal::Signal::SIGKILL as i32,
                ))))
                .await
                .expect("monitor acknowledges the kill");
            assert!(matches!(
                events.next().await,
                Some(super::DeviceEvent::Stopped)
            ));
            assert!(events.next().await.is_none());
        });
    }

    #[test]
    fn supervise_surfaces_a_panic_after_a_stop() {
        use futures_util::FutureExt as _;
        use smol::stream::StreamExt as _;

        let (running, sender, control) = Running::new();
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let mut events = Box::pin(running.supervise(interrupt_rx));

        smol::block_on(async {
            assert!(matches!(events.next().await, Some(DeviceEvent::Started)));
            interrupt_tx.try_send(()).expect("fire the interrupt");
            assert!(events.next().now_or_never().is_none());
            assert!(matches!(
                control.try_recv(),
                Ok(super::StopRequest::Terminate)
            ));
            let panic = PanicInfo {
                payload: "boom".to_string(),
                location: None,
            };
            sender
                .send(DeviceEvent::Crashed(Crash::new(CrashCause::Panic(
                    panic.clone(),
                ))))
                .await
                .expect("monitor reports panic");
            assert!(matches!(
                events.next().await,
                Some(DeviceEvent::Crashed(Crash { cause: CrashCause::Panic(reported), .. }))
                    if reported == panic
            ));
            assert!(events.next().await.is_none());
        });
    }

    #[cfg(unix)]
    #[test]
    fn supervise_surfaces_a_native_signal_crash_after_a_stop() {
        use futures_util::FutureExt as _;
        use smol::stream::StreamExt as _;

        let (running, sender, control) = Running::new();
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let mut events = Box::pin(running.supervise(interrupt_rx));

        smol::block_on(async {
            assert!(matches!(events.next().await, Some(DeviceEvent::Started)));
            interrupt_tx.try_send(()).expect("fire the interrupt");
            assert!(events.next().now_or_never().is_none());
            assert!(matches!(
                control.try_recv(),
                Ok(super::StopRequest::Terminate)
            ));
            let segv = nix::sys::signal::Signal::SIGSEGV as i32;
            sender
                .send(DeviceEvent::Crashed(Crash::new(CrashCause::Signal(segv))))
                .await
                .expect("monitor reports the crash");
            assert!(matches!(
                events.next().await,
                Some(DeviceEvent::Crashed(Crash { cause: CrashCause::Signal(signal), .. }))
                    if signal == segv
            ));
            assert!(events.next().await.is_none());
        });
    }

    #[test]
    fn supervise_keeps_its_stop_state_when_the_request_is_undelivered() {
        use futures_util::FutureExt as _;
        use smol::stream::StreamExt as _;

        let (running, sender, control) = Running::new();
        drop(control);
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let mut events = Box::pin(running.supervise(interrupt_rx));

        smol::block_on(async {
            assert!(matches!(events.next().await, Some(DeviceEvent::Started)));
            interrupt_tx.try_send(()).expect("fire the interrupt");
            assert!(events.next().now_or_never().is_none());
            sender
                .send(DeviceEvent::Crashed(Crash::new(CrashCause::ExitCode(1))))
                .await
                .expect("monitor reports the exit");
            assert!(matches!(
                events.next().await,
                Some(DeviceEvent::Crashed(Crash {
                    cause: CrashCause::ExitCode(1),
                    ..
                }))
            ));
            assert!(events.next().await.is_none());
        });
    }

    #[test]
    fn supervise_maps_exit_after_stop_to_stopped() {
        use futures_util::FutureExt as _;
        use smol::stream::StreamExt as _;

        let (running, sender, control) = Running::new();
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let mut events = Box::pin(running.supervise(interrupt_rx));

        smol::block_on(async {
            assert!(matches!(events.next().await, Some(DeviceEvent::Started)));
            interrupt_tx.try_send(()).expect("fire the interrupt");
            assert!(events.next().now_or_never().is_none());
            assert!(matches!(
                control.try_recv(),
                Ok(super::StopRequest::Terminate)
            ));
            sender
                .send(DeviceEvent::Exited(ApplicationExit::completed()))
                .await
                .expect("monitor reports process exit");
            assert!(matches!(events.next().await, Some(DeviceEvent::Stopped)));
            assert!(events.next().await.is_none());
        });
    }

    #[test]
    fn supervise_preserves_exit_without_a_stop_request() {
        use smol::stream::StreamExt as _;

        let (running, sender, _control) = Running::new();
        let (_interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let mut events = Box::pin(running.supervise(interrupt_rx));

        smol::block_on(async {
            assert!(matches!(events.next().await, Some(DeviceEvent::Started)));
            sender
                .send(DeviceEvent::Exited(ApplicationExit::completed()))
                .await
                .expect("monitor reports process exit");
            assert!(matches!(
                events.next().await,
                Some(DeviceEvent::Exited(exit))
                    if exit.reason() == ApplicationExitReason::Completed
            ));
            assert!(events.next().await.is_none());
        });
    }

    /// A terminal event does not cut off events already queued behind it:
    /// shutdown output written alongside the exit still reaches the
    /// consumer before the stream ends.
    #[test]
    fn supervise_yields_queued_events_after_a_terminal_event() {
        use smol::stream::StreamExt as _;

        let (running, sender, _control) = Running::new();
        let (_interrupt_tx, interrupt_rx) = async_channel::unbounded();
        let mut events = Box::pin(running.supervise(interrupt_rx));

        smol::block_on(async {
            events.next().await; // Started
            sender
                .try_send(super::DeviceEvent::Stopped)
                .expect("queue the terminal event");
            sender
                .try_send(super::DeviceEvent::Log {
                    level: tracing::Level::INFO,
                    message: "trailing line".to_string(),
                })
                .expect("queue a trailing event");

            assert!(matches!(
                events.next().await,
                Some(super::DeviceEvent::Stopped)
            ));
            assert!(
                matches!(events.next().await, Some(super::DeviceEvent::Log { .. })),
                "events queued behind the terminal event still yield"
            );
            assert!(events.next().await.is_none(), "then the stream ends");
        });
    }
}
