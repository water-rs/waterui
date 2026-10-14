//! Inspector app launcher and session management.

use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;

use eyre::{Context as _, Result, bail, eyre};
use futures_util::future::{Either, select};
use waterui_inspector_protocol::transport::{read_frame, write_frame};
use waterui_inspector_protocol::{
    InspectorBuildIdentity, InspectorClientMessage, InspectorServerMessage,
};

use crate::device::{Artifact, Device, DeviceEvent, Local, RunOptions, Running, StopRequest};
use crate::platform::{NativeOs, TargetPlatform};
use crate::workflows::inspector::client;

use super::forward::AndroidForward;

/// How long starting the prepared client may take — long enough for a cold
/// `open` of a freshly built bundle, short enough to fail instead of hang
/// with a forward held.
const LAUNCH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// An endpoint's `--identity` answer is one frame; anything longer is a
/// stuck endpoint, not a slow one.
const IDENTITY_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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
    /// Start the client on the connection `start_inspector` opened.
    ///
    /// The `Local.run` itself is bounded: a launch that stalls would hold
    /// an acquired forward forever, so it fails and the connection's
    /// forward is closed.
    ///
    /// # Errors
    /// Returns an error if the client fails to start; an acquired forward
    /// is closed, and its removal failure is carried alongside the launch
    /// failure.
    async fn launch_on(
        self,
        connection: TargetConnection,
        token: String,
    ) -> Result<InspectorSession> {
        let TargetConnection { addr, forward } = connection;

        let mut run_options = RunOptions::new();
        run_options.insert_env_var(
            "WATERUI_INSPECTOR_TARGET_ADDR".to_string(),
            addr.to_string(),
        );
        run_options.insert_env_var("WATERUI_INSPECTOR_TOKEN".to_string(), token);

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

/// Resolve, build or fetch, and package the inspector client for the
/// identity the endpoint reported.
///
/// Nothing that could leak is acquired — no forward, no credential, no
/// process — so cancelling preparation abandons nothing. The result
/// launches through [`start_inspector`].
///
/// # Errors
/// Returns an error if the inspected project's `WaterUI` source cannot be
/// resolved, the client cannot be built or fetched, or the project and the
/// endpoint disagree about versions or revisions.
pub async fn prepare_inspector(
    host: &crate::toolchain::Host,
    project_path: &Path,
    identity: &InspectorBuildIdentity,
    progress: Option<crate::build::BuildProgress>,
) -> Result<PreparedInspector> {
    let artifact = client::prepare_client(host, project_path, identity, progress).await?;
    Ok(PreparedInspector {
        platform: client_platform(),
        host: host.clone(),
        artifact,
    })
}

/// The endpoint connection a session owns until it is done: the target's
/// address plus the `adb forward` an Android target needs, removed exactly
/// when the session or a failed start closes.
struct TargetConnection {
    addr: SocketAddr,
    forward: Option<AndroidForward>,
}

impl TargetConnection {
    /// Dial the endpoint as `target` describes — an `AndroidDevice` opens
    /// its `adb forward` first, bounded by the forward's own deadline.
    async fn open(host: &crate::toolchain::Host, target: &InspectorTarget) -> Result<Self> {
        match target {
            InspectorTarget::Direct(addr) => Ok(Self {
                addr: *addr,
                forward: None,
            }),
            InspectorTarget::AndroidDevice { serial, port } => {
                let forward = AndroidForward::open(host, serial, *port).await?;
                Ok(Self {
                    addr: forward.local_addr(),
                    forward: Some(forward),
                })
            }
        }
    }

    /// Remove the forward an Android open registered — a no-op for a
    /// direct endpoint.
    async fn close(self) -> Result<()> {
        if let Some(forward) = self.forward {
            forward.close().await?;
        }
        Ok(())
    }
}

/// Ask the endpoint which build it is — the identity the prepared client
/// must answer `--identity` with.
///
/// One frame, one answer, bounded: a silent endpoint is a stuck endpoint,
/// not a slow one. The token is authenticated by the endpoint; a refusal
/// surfaces as the endpoint's `Error` reply.
async fn query_identity(
    addr: SocketAddr,
    token: &str,
    timeout: std::time::Duration,
) -> Result<InspectorBuildIdentity> {
    let query = async {
        let mut stream = smol::net::TcpStream::connect(addr)
            .await
            .wrap_err_with(|| format!("could not reach the inspector endpoint at {addr}"))?;
        write_frame(
            &mut stream,
            &InspectorClientMessage::Identify {
                token: token.to_owned(),
            },
        )
        .await
        .wrap_err("failed to send the identity query")?;
        match read_frame::<_, InspectorServerMessage>(&mut stream)
            .await
            .wrap_err("failed to read the identity reply")?
        {
            InspectorServerMessage::Identity { identity } => Ok(identity),
            InspectorServerMessage::Error { message } => {
                bail!("the endpoint refused the identity query: {message}")
            }
            other => Err(eyre!("unexpected reply to the identity query: {other:?}")),
        }
    };
    let query = std::pin::pin!(query);
    let timer = std::pin::pin!(smol::Timer::after(timeout));
    match select(query, timer).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => bail!("the endpoint at {addr} did not answer within {timeout:?}"),
    }
}

/// Race a preparation against interrupts.
///
/// Both futures are owned and pinned inside the helper: an interrupt that
/// wins the select drops the owned preparation *here*, before the caller
/// runs connection cleanup — never left live past this function's return.
/// An interrupt already queued is answered without polling the
/// preparation at all.
///
/// Returns `Some(result)` when the preparation finished first, `None` on
/// interrupt.
async fn prepare_until_interrupt(
    preparation: impl std::future::Future<Output = Result<PreparedInspector>>,
    interrupts: &smol::channel::Receiver<()>,
) -> Option<Result<PreparedInspector>> {
    if interrupts.try_recv().is_ok() {
        return None;
    }
    let mut preparation = Box::pin(preparation);
    let mut interrupt = Box::pin(interrupts.recv());
    match select(preparation.as_mut(), interrupt.as_mut()).await {
        Either::Left((result, _)) => Some(result),
        Either::Right(_) => {
            // Explicitly drop the losing owned future inside the helper —
            // a caller-side cleanup awaits only after this point.
            drop(preparation);
            drop(interrupt);
            None
        }
    }
}

/// Query the endpoint, prepare the matching client, and start the session.
///
/// The endpoint connection — including an Android session's `adb forward`
/// — stays owned outside every interruptible future: an interruption drops
/// the in-flight query or preparation inside `prepare_until_interrupt`
/// first, then the connection closes its own forward.
///
/// # Errors
/// Returns an error if the endpoint cannot be queried, the client cannot
/// be prepared, or cleanup fails. Returns `Ok(None)` when an interrupt
/// arrived before the session existed.
pub async fn start_inspector(
    host: &crate::toolchain::Host,
    project_path: &Path,
    options: InspectorLaunchOptions,
    progress: Option<crate::build::BuildProgress>,
    interrupts: smol::channel::Receiver<()>,
) -> Result<Option<InspectorSession>> {
    // Opening the forward is itself bounded by the forward's deadline — it
    // is never dropped mid-open by the interrupt race below.
    let connection = TargetConnection::open(host, &options.target).await?;

    // Copies the work future owns — `connection` and `options` stay free to
    // move into the session or the cleanup path.
    let addr = connection.addr;
    let token = options.token.clone();
    let prepared = match prepare_until_interrupt(
        async {
            let identity = query_identity(addr, &token, IDENTITY_QUERY_TIMEOUT).await?;
            prepare_inspector(host, project_path, &identity, progress).await
        },
        &interrupts,
    )
    .await
    {
        Some(Ok(prepared)) => prepared,
        Some(Err(error)) => {
            return Err(match connection.close().await {
                Ok(()) => error,
                Err(close_error) => error.wrap_err(format!(
                    "Also failed to close the endpoint connection: {close_error:#}"
                )),
            });
        }
        // The interrupted preparation is already dropped — only the
        // connection's own forward remains to remove.
        None => {
            if let Err(close_error) = connection.close().await {
                return Err(eyre!(
                    "interrupted; also failed to close the endpoint connection: {close_error:#}"
                ));
            }
            return Ok(None);
        }
    };

    prepared
        .launch_on(connection, options.token)
        .await
        .map(Some)
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
            host: host.clone(),
            artifact: crate::device::Artifact::new(
                "dev.waterui.inspector",
                PathBuf::from("/nonexistent/inspector.app"),
            ),
        };
        let forward = smol::block_on(super::super::forward::AndroidForward::open(
            &host,
            "pixel-9-pro:5555",
            23106,
        ))
        .expect("the session's forward opens");
        let error = smol::block_on(prepared.launch_on(
            super::TargetConnection {
                addr: forward.local_addr(),
                forward: Some(forward),
            },
            String::from("token"),
        ))
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
            host: host.clone(),
            artifact: crate::device::Artifact::new(
                "dev.waterui.inspector",
                PathBuf::from("/nonexistent/inspector.app"),
            ),
        };
        let forward = smol::block_on(super::super::forward::AndroidForward::open(
            &host,
            "pixel-9-pro:5555",
            23106,
        ))
        .expect("the session's forward opens");
        let error = smol::block_on(prepared.launch_on(
            super::TargetConnection {
                addr: forward.local_addr(),
                forward: Some(forward),
            },
            String::from("token"),
        ))
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

    /// The whole start on a direct endpoint: the query answers, the
    /// matching released client is fetched and verified, the launch
    /// spawns it, and its natural exit ends the session cleanly.
    #[cfg(unix)]
    #[test]
    fn a_direct_start_launches_the_prepared_session() {
        use sha2::Digest as _;
        use waterui_inspector_protocol::InspectorProtocolInfo;

        let machine = TestMachine::new();
        let identity = super::InspectorBuildIdentity {
            framework_version: String::from("0.5.2"),
            protocol: InspectorProtocolInfo {
                version: String::from("0.5.1"),
                build_commit: None,
            },
        };
        // A valid release archive on the fixture host: the binary is a
        // script that exits immediately — its natural exit ends the
        // session the launch produced.
        let script = include_bytes!("testdata/fake_client.sh");
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let directory = zip::write::SimpleFileOptions::default();
            let package = super::client::CLIENT_PACKAGE;
            zip.add_directory(format!("{package}/"), directory)
                .expect("dir");
            zip.start_file(
                format!("{package}/{}", super::client::binary_name()),
                directory.unix_permissions(0o755),
            )
            .expect("binary entry");
            std::io::Write::write_all(&mut zip, script).expect("binary bytes");
            zip.finish().expect("seal");
        }
        let archive = buffer.into_inner();
        let digest = hex::encode(sha2::Sha256::digest(&archive));
        let name = format!("waterui-inspector-{}.zip", target_lexicon::Triple::host());
        let version = &identity.framework_version;
        let asset_url =
            format!("https://github.com/water-rs/waterui/releases/download/v{version}/{name}");
        let api = format!("https://api.github.com/repos/water-rs/waterui/releases/tags/v{version}");
        let mut release = serde_json::json!({ "tag_name": format!("v{version}") });
        release["assets"] = serde_json::json!([{
            "name": name,
            "browser_download_url": asset_url,
            "digest": format!("sha256:{digest}"),
            "size": archive.len(),
        }]);
        // The fake client answers `--identity` correctly; without its
        // expected identity the verification would fail.
        let identity_file = machine.file(
            "responses/identity.json",
            &serde_json::to_string(&identity).expect("identity json"),
        );
        let host = machine
            .host([("WATERUI_FAKE_IDENTITY", identity_file.as_os_str())])
            .with_http_response(
                &api,
                zenwave::Response::new(zenwave::Body::from(release.to_string())),
            )
            .with_http_response(
                &asset_url,
                zenwave::Response::new(zenwave::Body::from(archive)),
            );

        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let addr = listener.local_addr().expect("address");
            let _server = smol::spawn(answer_one_identify(
                listener,
                Some(super::InspectorServerMessage::Identity {
                    identity: identity.clone(),
                }),
            ));
            let (_interrupts_tx, interrupts) = smol::channel::unbounded::<()>();
            let options = super::InspectorLaunchOptions {
                target: super::InspectorTarget::Direct(addr),
                token: String::from("token"),
            };
            let session =
                super::start_inspector(&host, machine.root(), options, None, interrupts.clone())
                    .await
                    .expect("the session starts")
                    .expect("a session exists");
            // The fixture binary exits on its own — supervise ends.
            session
                .supervise(interrupts, |_| Ok(()))
                .await
                .expect("the session ends cleanly");
        });
    }

    /// A one-shot endpoint fixture: accepts one connection, captures the
    /// `Identify` it was sent, replies `reply` (or holds silently for the
    /// timeout tests), then exits.
    #[cfg(unix)]
    async fn answer_one_identify(
        listener: smol::net::TcpListener,
        reply: Option<super::InspectorServerMessage>,
    ) -> Option<String> {
        let (mut stream, _) = listener.accept().await.expect("endpoint accepts");
        let message = super::read_frame::<_, super::InspectorClientMessage>(&mut stream)
            .await
            .expect("endpoint reads a frame");
        let token = match message {
            super::InspectorClientMessage::Identify { token } => Some(token),
            _ => None,
        };
        if let Some(reply) = reply {
            super::write_frame(&mut stream, &reply)
                .await
                .expect("endpoint replies");
        } else {
            // Hold the connection without answering — the querier's own
            // timeout or the interrupt decides.
            smol::Timer::after(std::time::Duration::from_secs(60)).await;
        }
        token
    }

    /// The fixture's own port + an `Identity` reply it returns.
    #[cfg(unix)]
    fn released_identity() -> super::InspectorBuildIdentity {
        waterui_inspector_protocol::InspectorBuildIdentity {
            framework_version: String::from("0.5.2"),
            protocol: waterui_inspector_protocol::InspectorProtocolInfo {
                version: String::from("0.5.1"),
                build_commit: None,
            },
        }
    }

    /// `query_identity` sends the session's token and returns the identity
    /// the endpoint answered with.
    #[cfg(unix)]
    #[test]
    fn query_identity_returns_the_endpoints_identity() {
        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let addr = listener.local_addr().expect("address");
            let reply = super::InspectorServerMessage::Identity {
                identity: released_identity(),
            };
            let server = smol::spawn(answer_one_identify(listener, Some(reply)));
            let identity =
                super::query_identity(addr, "session-token", std::time::Duration::from_secs(5))
                    .await
                    .expect("the endpoint answers");
            assert_eq!(identity, released_identity());
            assert_eq!(server.await.as_deref(), Some("session-token"));
        });
    }

    /// An endpoint's `Error` reply surfaces as the refusal it is; any other
    /// message is an unexpected-reply error.
    #[cfg(unix)]
    #[test]
    fn query_identity_surfaces_a_refusal() {
        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let addr = listener.local_addr().expect("address");
            let server = smol::spawn(answer_one_identify(
                listener,
                Some(super::InspectorServerMessage::Error {
                    message: String::from("invalid inspector token"),
                }),
            ));
            let error = super::query_identity(addr, "wrong", std::time::Duration::from_secs(5))
                .await
                .expect_err("a refused query fails");
            assert!(
                error.to_string().contains("invalid inspector token"),
                "{error}"
            );
            server.await;
        });
    }

    /// A `Welcome` is not an `Identity` — the querier refuses it rather
    /// than guessing.
    #[cfg(unix)]
    #[test]
    fn query_identity_refuses_an_unexpected_reply() {
        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let addr = listener.local_addr().expect("address");
            let server = smol::spawn(answer_one_identify(
                listener,
                Some(super::InspectorServerMessage::Welcome {
                    identity: released_identity(),
                    target: waterui_inspector_protocol::TargetInfo {
                        pid: 0,
                        name: String::from("fixture"),
                        backend: String::from("test"),
                    },
                    available: waterui_inspector_protocol::ChannelSet::default_subscription(),
                }),
            ));
            let error = super::query_identity(addr, "token", std::time::Duration::from_secs(5))
                .await
                .expect_err("a Welcome is not an Identity");
            assert!(error.to_string().contains("unexpected"), "{error}");
            server.await;
        });
    }

    /// A silent endpoint fails the query at the deadline — the socket is
    /// dropped, not held.
    #[cfg(unix)]
    #[test]
    fn a_silent_endpoint_times_the_query_out() {
        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let addr = listener.local_addr().expect("address");
            let _server = smol::spawn(answer_one_identify(listener, None));
            let error = super::query_identity(addr, "token", std::time::Duration::from_millis(20))
                .await
                .expect_err("a silent endpoint times out");
            assert!(error.to_string().contains("did not answer"), "{error}");
        });
    }

    /// A pre-queued interrupt returns `None` before the endpoint is
    /// queried — no poll of the preparation, so nothing connects.
    #[cfg(unix)]
    #[test]
    fn a_prequeued_interrupt_never_queries() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let (interrupts_tx, interrupts) = smol::channel::bounded::<()>(1);
        interrupts_tx.try_send(()).expect("the interrupt queues");
        // No listener exists — a query would fail on connect; `Ok(None)`
        // proves the preparation was never polled.
        let options = super::InspectorLaunchOptions {
            target: super::InspectorTarget::Direct("127.0.0.1:1".parse().expect("addr")),
            token: String::from("token"),
        };
        let result = smol::block_on(super::start_inspector(
            &host,
            machine.root(),
            options,
            None,
            interrupts,
        ));
        assert!(matches!(result, Ok(None)), "{result:?}");
    }

    /// An interrupted preparation drops the owned future inside the
    /// helper — the probe rides the preparation's drop, not any later
    /// caller-side cleanup.
    #[cfg(unix)]
    #[test]
    fn an_interrupted_preparation_is_dropped_before_cleanup() {
        struct Probe(Arc<AtomicBool>);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let probe = dropped.clone();
        smol::block_on(async move {
            let (started_tx, started_rx) = smol::channel::bounded::<()>(1);
            let pending = async move {
                let _probe = Probe(probe);
                let _ = started_tx.try_send(());
                std::future::pending::<eyre::Result<super::PreparedInspector>>().await
            };
            let (interrupts_tx, interrupts) = smol::channel::unbounded::<()>();
            let helper =
                smol::spawn(
                    async move { super::prepare_until_interrupt(pending, &interrupts).await },
                );
            // The interrupt lands only after the preparation is in flight —
            // this is a mid-flight cancel, not a queued one.
            started_rx.recv().await.expect("the preparation started");
            interrupts_tx.try_send(()).expect("interrupt");
            let result = helper.await;
            assert!(result.is_none(), "{result:?}");
            assert!(
                dropped.load(Ordering::SeqCst),
                "the losing preparation must be dropped before the helper returns"
            );
        });
    }

    /// An Android start interrupted *after the query reached the endpoint*
    /// drops the in-flight query, removes its own forward, and returns
    /// `None` — a sibling forward is untouched.
    #[cfg(unix)]
    #[test]
    fn an_interrupted_start_closes_only_its_own_forward() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);

        smol::block_on(async {
            // The fixture accepts the query, reports it, then holds — the
            // interrupt lands while the query is in flight.
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let port = listener.local_addr().expect("address").port();
            let (seen_tx, seen_rx) = smol::channel::bounded::<()>(1);
            let _server = smol::spawn(async move {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let message = super::read_frame::<_, super::InspectorClientMessage>(&mut stream)
                    .await
                    .expect("a frame");
                assert!(
                    matches!(message, super::InspectorClientMessage::Identify { .. }),
                    "the query sent an Identify"
                );
                let _ = seen_tx.try_send(());
                smol::Timer::after(std::time::Duration::from_secs(60)).await;
            });

            // The sibling's forward takes an unrelated bound port.
            let sibling_listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("sibling listener");
            let sibling_port = sibling_listener.local_addr().expect("address").port();
            machine.respond("ADB_FORWARD_PORT", &format!("{sibling_port}\n"));
            let sibling =
                super::super::forward::AndroidForward::open(&host, "pixel-9-pro:5555", 23106)
                    .await
                    .expect("the sibling forward opens");

            machine.respond("ADB_FORWARD_PORT", &format!("{port}\n"));
            let (interrupts_tx, interrupts) = smol::channel::unbounded::<()>();
            let options = super::InspectorLaunchOptions {
                target: super::InspectorTarget::AndroidDevice {
                    serial: String::from("pixel-9-pro:5555"),
                    port: 23106,
                },
                token: String::from("token"),
            };
            // The interrupt only lands once the endpoint actually holds
            // the session's Identify — this is an in-flight cancel, not a
            // queued one.
            let trigger = async {
                seen_rx.recv().await.expect("the endpoint saw the query");
                interrupts_tx.try_send(()).expect("interrupt");
            };
            let (result, ()) = smol::future::zip(
                super::start_inspector(&host, machine.root(), options, None, interrupts),
                trigger,
            )
            .await;
            assert!(matches!(result, Ok(None)), "{result:?}");

            let argv = std::fs::read_to_string(&log).expect("the adb argv log");
            assert!(
                argv.contains(&format!("forward --remove tcp:{port}")),
                "the session removed its own forward: {argv}"
            );
            assert!(
                !argv.contains(&format!("forward --remove tcp:{sibling_port}")),
                "the sibling is untouched: {argv}"
            );
            drop(sibling);
        });
    }

    /// A silent endpoint fails the start at the query deadline and the
    /// forward it opened is removed.
    #[cfg(unix)]
    #[test]
    fn a_silent_endpoint_fails_the_start_and_cleans_up() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);

        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let port = listener.local_addr().expect("address").port();
            let _server = smol::spawn(answer_one_identify(listener, None));
            machine.respond("ADB_FORWARD_PORT", &format!("{port}\n"));
            let (_interrupts_tx, interrupts) = smol::channel::unbounded::<()>();
            let options = super::InspectorLaunchOptions {
                target: super::InspectorTarget::AndroidDevice {
                    serial: String::from("pixel-9-pro:5555"),
                    port: 23106,
                },
                token: String::from("token"),
            };
            let error = super::start_inspector(&host, machine.root(), options, None, interrupts)
                .await
                .expect_err("a silent endpoint fails the start");
            assert!(error.to_string().contains("did not answer"), "{error}");
        });

        let argv = std::fs::read_to_string(&log).expect("the adb argv log");
        assert!(
            argv.contains("forward --remove tcp:"),
            "the failed start removed its forward: {argv}"
        );
    }

    /// An endpoint that refuses the `Identify` ends the start with the
    /// refusal — and the forward it opened is removed.
    #[cfg(unix)]
    #[test]
    fn a_refused_identity_closes_the_forward() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);

        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let port = listener.local_addr().expect("address").port();
            let _server = smol::spawn(answer_one_identify(
                listener,
                Some(super::InspectorServerMessage::Error {
                    message: String::from("invalid inspector token"),
                }),
            ));
            machine.respond("ADB_FORWARD_PORT", &format!("{port}\n"));
            let (_interrupts_tx, interrupts) = smol::channel::unbounded::<()>();
            let options = super::InspectorLaunchOptions {
                target: super::InspectorTarget::AndroidDevice {
                    serial: String::from("pixel-9-pro:5555"),
                    port: 23106,
                },
                token: String::from("wrong"),
            };
            let error = super::start_inspector(&host, machine.root(), options, None, interrupts)
                .await
                .expect_err("a refused identity fails the start");
            assert!(
                error.to_string().contains("invalid inspector token"),
                "{error}"
            );
        });

        let argv = std::fs::read_to_string(&log).expect("the adb argv log");
        assert!(
            argv.contains("forward --remove tcp:"),
            "the failed start removed its forward: {argv}"
        );
    }

    /// A preparation that fails after the query answered — a checkout
    /// revision the project cannot resolve — removes the forward and
    /// carries the failure.
    #[cfg(unix)]
    #[test]
    fn a_failed_preparation_closes_the_forward() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);

        smol::block_on(async {
            let listener = smol::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let port = listener.local_addr().expect("address").port();
            let checkout_identity = super::InspectorBuildIdentity {
                framework_version: String::from("0.5.2"),
                protocol: waterui_inspector_protocol::InspectorProtocolInfo {
                    version: String::from("0.5.1"),
                    build_commit: Some("a".repeat(40)),
                },
            };
            let _server = smol::spawn(answer_one_identify(
                listener,
                Some(super::InspectorServerMessage::Identity {
                    identity: checkout_identity,
                }),
            ));
            machine.respond("ADB_FORWARD_PORT", &format!("{port}\n"));
            let (_interrupts_tx, interrupts) = smol::channel::unbounded::<()>();
            let options = super::InspectorLaunchOptions {
                target: super::InspectorTarget::AndroidDevice {
                    serial: String::from("pixel-9-pro:5555"),
                    port: 23106,
                },
                token: String::from("token"),
            };
            // A project path that resolves nothing fails preparation.
            let missing = machine.root().join("nonexistent-project");
            let error = super::start_inspector(&host, &missing, options, None, interrupts)
                .await
                .expect_err("a failed preparation fails the start");
            assert_ne!(error.to_string(), "");
        });

        let argv = std::fs::read_to_string(&log).expect("the adb argv log");
        assert!(
            argv.contains("forward --remove tcp:"),
            "the failed start removed its forward: {argv}"
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
        let identity = waterui_inspector_protocol::InspectorBuildIdentity {
            framework_version: String::from("0.5.2"),
            protocol: waterui_inspector_protocol::InspectorProtocolInfo {
                version: String::from("0.5.1"),
                build_commit: Some("a".repeat(40)),
            },
        };
        let error = smol::block_on(super::prepare_inspector(&host, &missing, &identity, None))
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
