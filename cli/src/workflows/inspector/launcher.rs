//! Inspector app launcher and session management.

use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;

use eyre::{Context as _, Result};

use crate::device::{Artifact, Device, DeviceEvent, Local, RunOptions, Running, StopRequest};
use crate::platform::{NativeOs, TargetPlatform};

use super::forward::AndroidForward;

/// How long starting the prepared client may take — long enough for a cold
/// `open` of a freshly built bundle, short enough to fail instead of hang
/// with a forward held.
const LAUNCH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// Await `launch` for at most `deadline`.
///
/// The losing future is dropped before the caller runs cleanup — the
/// in-flight `Local.run` releases its process handles rather than living
/// on as a detached spawn.
///
/// # Errors
/// Returns the launch's own error, or a timeout error naming the deadline.
async fn bounded_client_launch(
    launch: impl std::future::Future<Output = Result<Running, crate::device::FailToRun>>,
    deadline: std::time::Duration,
) -> Result<Running> {
    use futures_util::future::{Either, select};

    let launch = Box::pin(launch);
    let timer = std::pin::pin!(smol::Timer::after(deadline));
    match select(launch, timer).await {
        Either::Left((result, _)) => {
            result.map_err(|e| eyre::eyre!("Failed to run inspector app: {e}"))
        }
        Either::Right(_) => Err(eyre::eyre!(
            "Timed out launching the inspector client after {deadline:?}"
        )),
    }
}

/// Where the endpoint the inspector attaches to lives.
///
/// The inspector client always runs on the developer's machine; this only
/// says how the server inside the inspected application is reached —
/// platform and device selection describe the inspected target, never where
/// the client launches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorTarget {
    /// An endpoint this machine can dial directly: an application running
    /// here, or any address the network reaches (a simulator's loopback, a
    /// routable device address, a port already forwarded).
    Direct(SocketAddr),
    /// An endpoint on an Android device, reached through an `adb forward`
    /// the session opens.
    AndroidDevice {
        /// The device's adb serial.
        serial: String,
        /// The port the application listens on, as the device sees it.
        port: u16,
    },
}

/// Launch options for Inspector app.
#[derive(Clone)]
pub struct InspectorLaunchOptions {
    /// The endpoint of the application to inspect.
    pub target: InspectorTarget,
    /// One-time token used by runtime endpoint and inspector app.
    pub token: String,
}

// The token is a credential: it travels to the endpoint, never into
// diagnostics.
impl std::fmt::Debug for InspectorLaunchOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectorLaunchOptions")
            .field("target", &self.target)
            .field("token", &"[redacted]")
            .finish()
    }
}

/// Inspector session state.
///
/// A session always owns what it started: the client process and the `adb`
/// forward it is dialling through. There is no detached lifetime — closing
/// the client ends the session and removes the registration.
#[derive(Debug)]
pub struct InspectorSession {
    /// The desktop platform the client was built for — always this host's
    /// own, never the inspected target's.
    pub platform: TargetPlatform,
    /// The address the inspector application dials.
    addr: SocketAddr,
    /// The `adb forward` that makes a device endpoint reachable here, for
    /// the sessions that opened one.
    forward: Option<AndroidForward>,
    /// The launched client's monitor.
    running: Pin<Box<Running>>,
}

impl InspectorSession {
    /// The address the inspector application dials — the endpoint itself,
    /// or the local end of the `adb forward` onto a device's endpoint.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Shutdown the client and remove the session's `adb forward`.
    ///
    /// # Errors
    /// Returns an error if the `adb forward` cannot be removed.
    pub async fn shutdown(self) -> Result<()> {
        let Self {
            forward, running, ..
        } = self;
        Pin::into_inner(running).shutdown(StopRequest::Kill).await;
        if let Some(forward) = forward {
            forward.close().await?;
        }
        Ok(())
    }

    /// Run the client to its end: every device event goes through
    /// `on_event`, the first interrupt asks the monitor to stop the client,
    /// and the session's own `adb forward` is removed once the monitored
    /// child is down. Nothing is detached or forgotten.
    ///
    /// # Errors
    /// Returns the first failure the callback or the monitor reported, and
    /// a forward-removal failure alongside it — never silently dropped.
    pub async fn supervise(
        self,
        interrupts: smol::channel::Receiver<()>,
        mut on_event: impl FnMut(DeviceEvent) -> Result<()>,
    ) -> Result<()> {
        use smol::stream::StreamExt as _;

        let Self {
            forward, running, ..
        } = self;
        let mut events = std::pin::pin!(Pin::into_inner(running).supervise(interrupts));
        let mut first_error: Option<eyre::Report> = None;
        while let Some(event) = events.next().await {
            let monitor_error = match &event {
                DeviceEvent::MonitorError { message } => Some(eyre::eyre!(
                    "the inspector client's monitor reported: {message}"
                )),
                _ => None,
            };
            if let Err(error) = on_event(event) {
                first_error.get_or_insert(error);
            }
            if let Some(error) = monitor_error {
                first_error.get_or_insert(error);
            }
        }

        let removal = match forward {
            Some(forward) => forward.close().await.err(),
            None => None,
        };
        if let Some(close_error) = removal {
            return match first_error {
                Some(error) => Err(error).wrap_err(format!(
                    "Also failed to remove the inspector's forwarding: {close_error:#}"
                )),
                None => Err(close_error),
            };
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// An inspector client ready to launch — the client's own source build or
/// the project's matching release download, resolved and verified by
/// [`crate::workflows::inspector::client`].
///
/// No credential is stored and no forward or process is acquired yet, so
/// cancelling here abandons nothing.
#[derive(Debug)]
pub struct PreparedInspector {
    /// The desktop platform the client was built for — this host's own.
    platform: TargetPlatform,
    /// The host that builds and spawns the client.
    host: crate::toolchain::Host,
    /// The packaged client.
    artifact: Artifact,
}

impl PreparedInspector {
    /// Acquire the endpoint's forward and start the client.
    ///
    /// The `Local.run` itself is bounded: a launch that stalls would hold
    /// an acquired forward forever, so it fails and the forward is closed.
    ///
    /// # Errors
    /// Returns an error if the forward cannot be opened or the client fails
    /// to start; a forward already acquired is closed, and its removal
    /// failure is carried alongside the launch failure.
    pub async fn launch(self, options: InspectorLaunchOptions) -> Result<InspectorSession> {
        let (addr, forward) = match &options.target {
            InspectorTarget::Direct(addr) => (*addr, None),
            InspectorTarget::AndroidDevice { serial, port } => {
                let forward = AndroidForward::open(&self.host, serial, *port).await?;
                (forward.local_addr(), Some(forward))
            }
        };

        let mut run_options = RunOptions::new();
        run_options.insert_env_var(
            "WATERUI_INSPECTOR_TARGET_ADDR".to_string(),
            addr.to_string(),
        );
        run_options.insert_env_var("WATERUI_INSPECTOR_TOKEN".to_string(), options.token);

        let running = bounded_client_launch(
            Local.run(&self.host, self.artifact, run_options),
            LAUNCH_DEADLINE,
        )
        .await;
        let running = match running {
            Ok(running) => running,
            Err(error) => {
                // A launch that fails after the forward was opened must not
                // leak the registration the adb server is holding — and a
                // removal that fails is itself an error the caller needs,
                // not a warning swallowed beside the real one.
                if let Some(forward) = forward
                    && let Err(close_error) = forward.close().await
                {
                    return Err(error).wrap_err(format!(
                        "Also failed to remove the inspector's forwarding: {close_error:#}"
                    ));
                }
                return Err(error);
            }
        };

        Ok(InspectorSession {
            platform: self.platform,
            addr,
            forward,
            running: Box::pin(running),
        })
    }
}

/// The platform the inspector client builds for: the developer machine's
/// own OS, always — the inspected target's platform never enters into it.
///
/// This derives from [`NativeOs::running_host`] rather than the [`Host`]
/// argument: a `Host` is a spawn/environment seam — it carries `PATH`, the
/// working directory, and app roots, and every process it launches is a
/// `std::process::Command` in this machine's process space. It has no OS
/// field and cannot describe a remote execution context, so the only OS a
/// `Host` can ever realize a client on is the one this binary runs on —
/// which `running_host()` reports. The `Host` still governs how the client
/// is built and spawned; it just cannot change where.
const fn client_platform() -> TargetPlatform {
    NativeOs::running_host().target_platform()
}

/// Resolve, build or fetch, and package the inspector client.
///
/// Nothing that could leak is acquired — no forward, no credential, no
/// process — so cancelling preparation abandons nothing. The result
/// launches through [`PreparedInspector::launch`].
///
/// # Errors
/// Returns an error if the inspected project's `WaterUI` source cannot be
/// resolved, or the client cannot be built or fetched.
pub async fn prepare_inspector(
    host: &crate::toolchain::Host,
    project_path: &Path,
    progress: Option<crate::build::BuildProgress>,
) -> Result<PreparedInspector> {
    let artifact = super::client::prepare_client(host, project_path, progress).await?;
    Ok(PreparedInspector {
        platform: client_platform(),
        host: host.clone(),
        artifact,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::sync::Arc;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicBool, Ordering};

    #[cfg(unix)]
    use crate::toolchain::testing::TestMachine;

    use super::TargetPlatform;

    /// The token is a credential: `Debug` names the target without
    /// spelling the secret.
    #[test]
    fn the_launch_options_debug_never_shows_the_token() {
        let options = super::InspectorLaunchOptions {
            target: super::InspectorTarget::Direct(
                "127.0.0.1:23106".parse().expect("a valid address"),
            ),
            token: String::from("s3cret-launch-token"),
        };
        let rendered = format!("{options:?}");
        assert!(!rendered.contains("s3cret-launch-token"), "{rendered}");
        assert!(rendered.contains("target"), "{rendered}");
        assert!(rendered.contains("23106"), "{rendered}");
    }

    /// The client always runs on the developer machine's own OS — one of
    /// the three supported desktop platforms Hydrolysis builds natively,
    /// never the inspected target's platform.
    #[test]
    fn the_client_platform_is_the_hosts_desktop_platform() {
        assert!(matches!(
            super::client_platform(),
            TargetPlatform::MacOS | TargetPlatform::Linux | TargetPlatform::Windows
        ));
    }

    /// An `InspectorSession` stood up from a fake `Running` plus a real
    /// `AndroidForward` against the fake adb — the session's own port, and
    /// a sibling registration the session must never touch.
    #[cfg(unix)]
    fn session_fixture(
        machine: &TestMachine,
        host: &crate::toolchain::Host,
    ) -> (
        super::InspectorSession,
        super::super::forward::AndroidForward,
        smol::channel::Sender<crate::device::DeviceEvent>,
        smol::channel::Receiver<crate::device::StopRequest>,
    ) {
        machine.respond("ADB_FORWARD_PORT", "49321\n");
        let own = smol::block_on(super::super::forward::AndroidForward::open(
            host,
            "pixel-9-pro:5555",
            23106,
        ))
        .expect("the session's forward opens");
        machine.respond("ADB_FORWARD_PORT", "49322\n");
        let sibling = smol::block_on(super::super::forward::AndroidForward::open(
            host,
            "pixel-9-pro:5555",
            23106,
        ))
        .expect("the sibling's forward opens");
        let (running, events, control) = crate::device::Running::new();
        let session = super::InspectorSession {
            platform: super::client_platform(),
            addr: own.local_addr(),
            forward: Some(own),
            running: Box::pin(running),
        };
        (session, sibling, events, control)
    }

    /// The adb argv log — readable lines, never a swallowed I/O error.
    #[cfg(unix)]
    fn adb_argv(log: &std::path::Path) -> String {
        match std::fs::read_to_string(log) {
            Ok(argv) => argv,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("read the fake adb argv log: {error}"),
        }
    }

    /// A bounded `Local.run` that loses the race drops its future before
    /// the caller's cleanup runs — the in-flight spawn releases its handles.
    #[cfg(unix)]
    #[test]
    fn a_timed_out_launch_drops_the_in_flight_run() {
        struct Probe(Arc<AtomicBool>);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let probe = dropped.clone();
        let pending = async move {
            // The probe rides inside the future: it drops only when the
            // future itself does.
            let _probe = Probe(probe);
            std::future::pending::<Result<crate::device::Running, crate::device::FailToRun>>().await
        };

        let error = smol::block_on(super::bounded_client_launch(
            pending,
            std::time::Duration::from_millis(20),
        ))
        .expect_err("a never-completing run times out");
        assert!(error.to_string().contains("Timed out"), "{error}");
        assert!(
            dropped.load(Ordering::SeqCst),
            "the losing launch future must be dropped before return"
        );
    }

    /// A client that exits on its own still has its forward removed by the
    /// supervising session — the registration outlives nothing.
    #[cfg(unix)]
    #[test]
    fn a_natural_exit_removes_only_the_sessions_forward() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);
        let (session, sibling, events, _control) = session_fixture(&machine, &host);
        let (_interrupts_tx, interrupts) = smol::channel::unbounded::<()>();

        events
            .try_send(crate::device::DeviceEvent::Exited(
                crate::device::ApplicationExit::completed(),
            ))
            .expect("the terminal event queues");
        smol::block_on(session.supervise(interrupts, |_| Ok(()))).expect("a clean exit cleans up");

        // The session removed exactly its own registration — the sibling's
        // is still on the server until it closes itself.
        let argv = adb_argv(&log);
        assert!(
            argv.contains("forward --remove tcp:49321"),
            "the session removed its own forward: {argv}"
        );
        assert!(!argv.contains("forward --remove tcp:49322"), "{argv}");
        assert!(!argv.contains("--remove-all"), "{argv}");

        smol::block_on(sibling.close()).expect("the sibling closes itself");
        assert!(adb_argv(&log).contains("forward --remove tcp:49322"));
    }

    /// Ctrl-C is the client's own shutdown: the monitor gets Terminate, the
    /// acknowledged Stopped ends the session, and its forward is removed.
    #[cfg(unix)]
    #[test]
    fn a_ctrl_c_terminates_the_client_then_removes_the_forward() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);
        let (session, sibling, events, control) = session_fixture(&machine, &host);
        let (interrupts_tx, interrupts) = smol::channel::unbounded::<()>();

        let work = smol::future::or(
            async {
                // Ctrl-C first, the monitor's acknowledgement second — the
                // Terminate request must reach it.
                interrupts_tx.try_send(()).expect("interrupt queues");
                match control.recv().await {
                    Ok(crate::device::StopRequest::Terminate) => {}
                    other => panic!("expected Terminate, got {other:?}"),
                }
                events
                    .try_send(crate::device::DeviceEvent::Stopped)
                    .expect("the monitor acknowledges");
            },
            async {
                smol::Timer::after(std::time::Duration::from_secs(2)).await;
                panic!("supervision did not finish within two seconds");
            },
        );
        smol::block_on(async {
            smol::future::zip(work, session.supervise(interrupts, |_| Ok(())))
                .await
                .1
        })
        .expect("the interrupted session cleans up");

        let argv = adb_argv(&log);
        assert!(
            argv.contains("forward --remove tcp:49321"),
            "the session removed its own forward: {argv}"
        );
        assert!(!argv.contains("forward --remove tcp:49322"), "{argv}");
        assert!(!argv.contains("--remove-all"), "{argv}");
        let _ = sibling;
    }

    /// A removal that fails after the client has already ended is reported
    /// as the cleanup error itself — the session does not drop it.
    #[cfg(unix)]
    #[test]
    fn a_natural_exit_still_reports_a_failed_removal() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
            (
                "WATERUI_FAKE_ADB_FORWARD_REMOVE_STATUS",
                std::ffi::OsStr::new("1"),
            ),
        ]);
        let (session, sibling, events, _control) = session_fixture(&machine, &host);
        let (_interrupts_tx, interrupts) = smol::channel::unbounded::<()>();

        events
            .try_send(crate::device::DeviceEvent::Exited(
                crate::device::ApplicationExit::completed(),
            ))
            .expect("the terminal event queues");
        let error = smol::block_on(session.supervise(interrupts, |_| Ok(())))
            .expect_err("the removal failure surfaces");
        // The failing removal never touched the sibling's registration.
        let argv = adb_argv(&log);
        assert!(!argv.contains("forward --remove tcp:49322"), "{argv}");
        drop(sibling);
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("Failed to remove the inspector's adb forward tcp:49321"),
            "the removal diagnostic names the owned port: {rendered}"
        );
    }

    /// A callback failure does not cut the drain short: the remaining
    /// events still flow, the forward is still removed, and a removal
    /// failure rides along beside the first error.
    #[cfg(unix)]
    #[test]
    fn a_callback_failure_keeps_draining_then_reports_both_failures() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
            (
                "WATERUI_FAKE_ADB_FORWARD_REMOVE_STATUS",
                std::ffi::OsStr::new("1"),
            ),
        ]);
        let (session, sibling, events, _control) = session_fixture(&machine, &host);
        let (_interrupts_tx, interrupts) = smol::channel::unbounded::<()>();

        // `Running::new` queues `Started` itself: that is the event the
        // callback fails on; the terminal event and the output queued
        // behind it must still be delivered before supervise ends.
        events
            .try_send(crate::device::DeviceEvent::Exited(
                crate::device::ApplicationExit::completed(),
            ))
            .expect("the terminal event queues");
        events
            .try_send(crate::device::DeviceEvent::Stdout {
                message: String::from("shutdown output"),
            })
            .expect("the trailing output queues");

        let mut received = Vec::new();
        let error = smol::block_on(session.supervise(interrupts, |event| match event {
            crate::device::DeviceEvent::Started => {
                received.push("started");
                Err(eyre::eyre!("the primary failure"))
            }
            crate::device::DeviceEvent::Exited(_) => {
                received.push("exited");
                Ok(())
            }
            crate::device::DeviceEvent::Stdout { .. } => {
                received.push("stdout");
                Ok(())
            }
            _ => Ok(()),
        }))
        .expect_err("the callback failure surfaces");
        drop(sibling);

        assert_eq!(received, ["started", "exited", "stdout"], "{received:?}");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("the primary failure"), "{rendered}");
        assert!(
            rendered.contains("Also failed to remove the inspector's forwarding"),
            "the removal failure is carried alongside: {rendered}"
        );
    }

    /// A launch that fails after the `adb forward` was opened removes
    /// the registration — the adb server is not left holding a local
    /// port for an inspector that never started.
    #[cfg(unix)]
    #[test]
    fn a_launch_failure_removes_the_forward() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);
        machine.respond("ADB_FORWARD_PORT", "49321\n");

        // A prepared client whose artifact cannot run fails the launch
        // after the forward opened.
        let prepared = super::PreparedInspector {
            platform: super::client_platform(),
            host,
            artifact: crate::device::Artifact::new(
                "dev.waterui.inspector",
                PathBuf::from("/nonexistent/inspector.app"),
            ),
        };
        let options = super::InspectorLaunchOptions {
            target: super::InspectorTarget::AndroidDevice {
                serial: String::from("pixel-9-pro:5555"),
                port: 23106,
            },
            token: String::from("token"),
        };
        let error = smol::block_on(prepared.launch(options))
            .expect_err("a missing client binary cannot launch");

        let argv = std::fs::read_to_string(&log).expect("the adb argv log");
        assert!(
            argv.contains("forward --remove tcp:49321"),
            "the failed launch must remove its forward: {argv}"
        );
        let _ = error;
    }

    /// A removal that fails is itself an error — the primary launch
    /// failure carries it through, never a warning swallowed beside the
    /// real diagnosis.
    #[cfg(unix)]
    #[test]
    fn a_failed_removal_travels_with_the_launch_error() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
            (
                "WATERUI_FAKE_ADB_FORWARD_REMOVE_STATUS",
                std::ffi::OsStr::new("1"),
            ),
        ]);
        machine.respond("ADB_FORWARD_PORT", "49321\n");

        let prepared = super::PreparedInspector {
            platform: super::client_platform(),
            host,
            artifact: crate::device::Artifact::new(
                "dev.waterui.inspector",
                PathBuf::from("/nonexistent/inspector.app"),
            ),
        };
        let options = super::InspectorLaunchOptions {
            target: super::InspectorTarget::AndroidDevice {
                serial: String::from("pixel-9-pro:5555"),
                port: 23106,
            },
            token: String::from("token"),
        };
        let error = smol::block_on(prepared.launch(options))
            .expect_err("a missing client binary cannot launch");

        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("Failed to run inspector app"),
            "the launch failure is carried: {rendered}"
        );
        assert!(
            rendered.contains("Also failed to remove the inspector's forwarding"),
            "the removal failure is carried: {rendered}"
        );
    }

    /// Preparation that fails — here, an inspected project that does not
    /// resolve — acquires nothing: the fake adb log holds no forward at
    /// all, because none was opened.
    #[cfg(unix)]
    #[test]
    fn a_failed_preparation_never_opens_a_forward() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);

        let missing = machine.root().join("nonexistent-project");
        let error = smol::block_on(super::prepare_inspector(&host, &missing, None))
            .expect_err("a missing project cannot prepare");
        let _ = error;

        let argv = match std::fs::read_to_string(&log) {
            Ok(argv) => argv,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("read the fake adb argv log: {error}"),
        };
        assert!(
            !argv.contains("forward"),
            "preparation acquires nothing: {argv}"
        );
    }
}
