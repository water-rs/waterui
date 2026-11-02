//! Inspector app launcher and session management.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use eyre::{Context as _, Result, bail};
use tracing::info;

use crate::build::{BuildOptions, BuildProfile, BuildProgress};
use crate::device::{Artifact, Device, DeviceEvent, Local, RunOptions, Running, StopRequest};
use crate::hydrolysis::platform::{build_hydrolysis, package_hydrolysis};
use crate::platform::{NativeOs, PackageOptions, TargetPlatform};
use crate::project::Project;
use crate::runtime_compat::runtime_profile_tag;
use crate::runtime_fingerprint::{compute_runtime_fingerprint, runtime_package_identity};
use crate::support_app;
use crate::templates::TemplateContext;

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

const INSPECTOR_TEMPLATE_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
const INSPECTOR_METADATA_FILE: &str = ".waterui-inspector-signature";
/// Bumped whenever `scaffold_inspector_app` changes what it generates beyond
/// the templated files (manifest edits, permissions, the serving set), which
/// the template fingerprint does not cover.
const INSPECTOR_SCAFFOLD_GENERATION: u32 = 3;

#[derive(Debug, Clone)]
struct InspectorRequirements {
    waterui_path: Option<PathBuf>,
    runtime_fingerprint: String,
}

/// Where the endpoint the inspector attaches to lives.
///
/// The inspector client always runs on the developer's machine; this only
/// says how the server inside the inspected application is reached —
/// platform and device selection describe the inspected target, never where
/// the client launches.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    target: InspectorTarget,
    /// One-time token used by runtime endpoint and inspector app.
    token: String,
}

impl InspectorLaunchOptions {
    /// Options pointing the launched app at the inspector on `target`,
    /// authenticating with `token`.
    #[must_use]
    pub const fn new(target: InspectorTarget, token: String) -> Self {
        Self { target, token }
    }

    /// The endpoint of the application to inspect.
    #[must_use]
    pub const fn target(&self) -> &InspectorTarget {
        &self.target
    }

    /// One-time token used by runtime endpoint and inspector app.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
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

/// An inspector client ready to launch.
///
/// Its support project is resolved, scaffolded, built and packaged — no
/// credential stored, no forward or process acquired yet, so cancelling
/// here abandons nothing.
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

/// Every triple the inspector support app can build for — the serving set
/// of its manifest's portable-desktop table. The client only ever launches
/// on a developer machine, so this is the same set every generated manifest
/// serves for a desktop run: an Android or iOS triple here would be an
/// on-device client coming back.
fn inspector_targets() -> Vec<target_lexicon::Triple> {
    crate::platform::native_target_triples()
}

/// Resolve, scaffold, build and package the inspector client.
///
/// Nothing that could leak is acquired — no forward, no credential, no
/// process — so cancelling preparation abandons nothing. The result
/// launches through [`PreparedInspector::launch`].
///
/// # Errors
/// Returns an error if the inspected project's requirements cannot be
/// resolved, the support project cannot be scaffolded, or the client fails
/// to build or package.
pub async fn prepare_inspector(
    host: &crate::toolchain::Host,
    project_path: &Path,
    progress: Option<BuildProgress>,
) -> Result<PreparedInspector> {
    let platform = client_platform();
    let requirements = resolve_inspector_requirements(host, project_path).await?;
    let inspector_app_path = inspector_support_path(host)?;
    ensure_inspector_support_app(host, &inspector_app_path, &requirements).await?;

    let project = crate::hydrolysis::backend::open_ready(host, &inspector_app_path)
        .await
        .wrap_err("Failed to open inspector support app project")?;

    let mut build_options = BuildOptions::development(BuildProfile::Debug);
    if let Some(progress) = &progress {
        build_options = build_options.with_progress(progress.clone());
    }
    let mut package_options = PackageOptions::development();
    if let Some(progress) = progress {
        package_options = package_options.with_progress(progress);
    }

    info!("Building the inspector client for this machine...");
    let built = build_hydrolysis(&project, platform, build_options)
        .await
        .map_err(|e| eyre::eyre!("Failed to build inspector app: {e}"))?;
    let artifact = package_hydrolysis(&project, platform, package_options, Some(&built))
        .await
        .map_err(|e| eyre::eyre!("Failed to package inspector app: {e}"))?;

    Ok(PreparedInspector {
        platform,
        // The project that builds and spawns the client owns the context
        // (cwd, environment, app roots) the run inherits — never the
        // caller's host value.
        host: project.host().clone(),
        artifact,
    })
}

fn inspector_support_path(host: &crate::toolchain::Host) -> Result<PathBuf> {
    support_app::support_app_path(host, "inspector_support")
}

async fn ensure_inspector_support_app(
    host: &crate::toolchain::Host,
    path: &Path,
    requirements: &InspectorRequirements,
) -> Result<()> {
    let desired_signature = inspector_signature(requirements);
    let scaffold_path = path.to_path_buf();
    let scaffold_requirements = requirements.clone();
    support_app::ensure_support_app(
        path,
        INSPECTOR_METADATA_FILE,
        &desired_signature,
        "inspector support",
        move || async move {
            scaffold_inspector_app(host, &scaffold_path, &scaffold_requirements).await
        },
    )
    .await
}

async fn scaffold_inspector_app(
    host: &crate::toolchain::Host,
    path: &Path,
    requirements: &InspectorRequirements,
) -> Result<()> {
    let waterui_path = requirements.waterui_path.clone();

    let options = inspector_create_options(waterui_path.clone());
    let expected_packages = inspector_project_packages(&options);

    let project = Project::create(host, path, options)
        .await
        .map_err(|e| eyre::eyre!("Failed to create inspector app: {e}"))?;

    let framework = project.resolved_framework().await?;
    let project_packages = project
        .project_packages(framework, &inspector_targets())
        .await?;
    // `inspector_signature` fingerprints the set derived from the display
    // name before the project exists; the manifest must carry that same set,
    // or the stored signature would describe a manifest that was not written.
    eyre::ensure!(
        project_packages == expected_packages,
        "the inspector support app resolved its own packages as {project_packages:?}, \
         but its signature fingerprints {expected_packages:?}"
    );
    let ctx = TemplateContext::for_support_app(
        host,
        crate::templates::SupportAppIdentity {
            display_name: "WaterUI Inspector".to_string(),
            crate_name: project.crate_name().clone(),
            bundle_identifier: crate::project_types::BundleIdentifier::try_from(
                "dev.waterui.inspector",
            )
            .expect("inspector support bundle identifier must be valid"),
        },
        waterui_path,
        framework,
        false,
        project.local_sources(),
    )
    .with_project_packages(project_packages);

    crate::templates::inspector::scaffold(project.root(), &ctx)
        .await
        .wrap_err("Failed to scaffold embedded inspector app template")?;

    info!("Inspector app scaffolded at {}", path.display());
    Ok(())
}

/// The `water create` options the inspector support app is scaffolded from.
fn inspector_create_options(waterui_path: Option<PathBuf>) -> crate::project::CreateOptions {
    crate::project::CreateOptions::new(
        "WaterUI Inspector".to_string(),
        crate::project_types::BundleIdentifier::try_from("dev.waterui.inspector")
            .expect("inspector support bundle identifier must be valid"),
        String::new(),
    )
    .with_waterui_path(waterui_path)
}

/// The inspector support app's own packages: its graph is its app crate plus
/// the framework, so the set is the crate `water create` derives from the
/// display name. `scaffold_inspector_app` checks the written set against it.
fn inspector_project_packages(options: &crate::project::CreateOptions) -> BTreeSet<String> {
    BTreeSet::from([options
        .crate_name()
        .expect("the inspector display name derives a valid crate name")
        .to_string()])
}

fn inspector_signature(requirements: &InspectorRequirements) -> String {
    format!(
        "template_commit={INSPECTOR_TEMPLATE_COMMIT}\nscaffold_generation={INSPECTOR_SCAFFOLD_GENERATION}\nwaterui_dependency={}\nruntime_fingerprint={}\ntemplate_fingerprint={}",
        requirements.waterui_path.as_ref().map_or_else(
            || String::from("registry"),
            |path| path.display().to_string()
        ),
        requirements.runtime_fingerprint,
        crate::templates::inspector::template_fingerprint(&inspector_project_packages(
            &inspector_create_options(requirements.waterui_path.clone())
        )),
    )
}

async fn resolve_inspector_requirements(
    host: &crate::toolchain::Host,
    project_path: &Path,
) -> Result<InspectorRequirements> {
    let mut command = cargo_metadata::MetadataCommand::new();
    command.current_dir(project_path);
    let metadata = host
        .cargo_metadata(&command)
        .await
        .wrap_err("Failed to resolve user project Cargo metadata for inspector compatibility")?;

    let waterui = select_unique_package(&metadata, "waterui")?;
    let waterui_core = select_unique_package(&metadata, "waterui-core")?;
    let runtime_identity = runtime_package_identity(waterui_core);

    let runtime_fingerprint_base = if waterui.source.is_none() {
        let waterui_root = waterui
            .manifest_path
            .as_std_path()
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| eyre::eyre!("Failed to derive waterui package root path"))?;
        let fingerprint =
            compute_runtime_fingerprint(host, &waterui_root, &runtime_identity).await?;
        return Ok(InspectorRequirements {
            waterui_path: Some(waterui_root),
            runtime_fingerprint: format!("{fingerprint}|profile={}", runtime_profile_tag()),
        });
    } else {
        let source = waterui
            .source
            .as_ref()
            .map(ToString::to_string)
            .expect("registry dependency must have a source");
        format!("{runtime_identity}:source:{source}")
    };

    Ok(InspectorRequirements {
        waterui_path: None,
        runtime_fingerprint: format!(
            "{runtime_fingerprint_base}|profile={}",
            runtime_profile_tag()
        ),
    })
}

fn select_unique_package<'a>(
    metadata: &'a cargo_metadata::Metadata,
    name: &'a str,
) -> Result<&'a cargo_metadata::Package> {
    let mut matches = metadata.packages.iter().filter(|p| p.name == name);
    let first = matches
        .next()
        .ok_or_else(|| eyre::eyre!("Could not resolve package `{name}` from metadata"))?;
    if matches.next().is_some() {
        bail!(
            "Multiple `{name}` packages were resolved. Inspector requires a single resolved `{name}` package."
        );
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[cfg(unix)]
    use crate::toolchain::testing::TestMachine;
    #[cfg(unix)]
    use std::sync::Arc;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicBool, Ordering};

    use target_lexicon::{Environment, OperatingSystem, Triple};

    #[cfg(unix)]
    use super::PreparedInspector;
    use super::{InspectorRequirements, TargetPlatform, inspector_signature, inspector_targets};

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

    /// The inspector support app is the desktop client, so its serving set
    /// covers the same triples every manifest serves for a desktop run —
    /// including this host's own — and no mobile triple: an Android or iOS
    /// triple here would be an on-device client coming back.
    #[test]
    fn the_support_app_serves_desktop_triples_only() {
        let targets = inspector_targets();
        assert!(
            targets.contains(&Triple::host()),
            "the host's own {triple} must be served: {targets:?}",
            triple = Triple::host()
        );
        for triple in &targets {
            assert_ne!(
                triple.environment,
                Environment::Android,
                "an Android triple means an on-device client: {triple}"
            );
            assert!(
                !matches!(triple.operating_system, OperatingSystem::IOS(_)),
                "an iOS triple means a simulator/device client: {triple}"
            );
        }
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
        super::AndroidForward,
        smol::channel::Sender<crate::device::DeviceEvent>,
        smol::channel::Receiver<crate::device::StopRequest>,
    ) {
        machine.respond("ADB_FORWARD_PORT", "49321\n");
        let own = smol::block_on(super::AndroidForward::open(host, "pixel-9-pro:5555", 23106))
            .expect("the session's forward opens");
        machine.respond("ADB_FORWARD_PORT", "49322\n");
        let sibling = smol::block_on(super::AndroidForward::open(host, "pixel-9-pro:5555", 23106))
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
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
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
        let prepared = PreparedInspector {
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

        let prepared = PreparedInspector {
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

    /// A scaffold edit outside the templates lies outside the template
    /// fingerprint, so the signature carries the scaffold generation that
    /// invalidates cached support apps when the edit changes.
    #[test]
    fn the_signature_carries_the_scaffold_generation() {
        let signature = inspector_signature(&InspectorRequirements {
            waterui_path: Some(PathBuf::from("/waterui")),
            runtime_fingerprint: "fingerprint".to_string(),
        });
        assert!(
            signature.contains(&format!(
                "scaffold_generation={}",
                super::INSPECTOR_SCAFFOLD_GENERATION
            )),
            "{signature}"
        );
    }
}
