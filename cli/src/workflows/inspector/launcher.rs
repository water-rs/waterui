//! Inspector app launcher and session management.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use eyre::{Context as _, Result, bail};
use tracing::info;

use crate::build::{BuildOptions, BuildProfile, BuildProgress};
use crate::device::{Device, Local, RunOptions, Running, StopRequest};
use crate::hydrolysis::platform::{build_hydrolysis, package_hydrolysis};
use crate::platform::{NativeOs, PackageOptions, TargetPlatform};
use crate::project::Project;
use crate::runtime_compat::runtime_profile_tag;
use crate::runtime_fingerprint::{compute_runtime_fingerprint, runtime_package_identity};
use crate::support_app;
use crate::templates::TemplateContext;

use super::forward::AndroidForward;

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
    /// Running handle for apps launched by this session.
    running: Option<Pin<Box<Running>>>,
    /// Whether this session owns the app lifecycle.
    owns_app: bool,
}

impl InspectorSession {
    /// The address the inspector application dials — the endpoint itself,
    /// or the local end of the `adb forward` onto a device's endpoint.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Shutdown the inspector app if this session launched it, and remove
    /// the `adb forward` it wrote.
    ///
    /// # Errors
    /// Returns an error if the `adb forward` cannot be removed.
    pub async fn shutdown(&mut self) -> Result<()> {
        if self.owns_app
            && let Some(running) = self.running.take()
        {
            Pin::into_inner(running).shutdown(StopRequest::Kill).await;
        }
        if let Some(forward) = self.forward.take() {
            forward.close().await?;
        }
        Ok(())
    }

    /// Detach inspector app so it keeps running after session drop.
    ///
    /// A detached inspector keeps dialling through the `adb forward`, so
    /// the registration stays on the adb server — the detached app's to
    /// hold, never a later session's to remove.
    pub fn detach(&mut self) {
        if let Some(running) = self.running.take() {
            std::mem::forget(running);
            self.owns_app = false;
        }
        self.forward = None;
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

/// Launch (or relaunch) an inspector support app on this machine.
///
/// # Errors
/// Returns an error if the endpoint cannot be reached, the support project
/// cannot be prepared, or the inspector app fails to build or launch.
pub async fn launch_inspector_session(
    host: &crate::toolchain::Host,
    project_path: &Path,
    options: InspectorLaunchOptions,
    progress: Option<BuildProgress>,
) -> Result<InspectorSession> {
    let platform = client_platform();
    // The endpoint is resolved before the client is built: a device that
    // cannot be reached is worth knowing about now rather than after a
    // compile.
    let (addr, forward) = match &options.target {
        InspectorTarget::Direct(addr) => (*addr, None),
        InspectorTarget::AndroidDevice { serial, port } => {
            let forward = AndroidForward::open(host, serial, *port).await?;
            (forward.local_addr(), Some(forward))
        }
    };

    let running =
        match launch_client(host, project_path, platform, addr, &options.token, progress).await {
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
        platform,
        addr,
        forward,
        running: Some(Box::pin(running)),
        owns_app: true,
    })
}

/// Build and run the inspector client on this machine through the portable
/// desktop realization — Hydrolysis on the host's own platform, the same
/// way `water run` puts an app on a desktop.
async fn launch_client(
    host: &crate::toolchain::Host,
    project_path: &Path,
    platform: TargetPlatform,
    addr: SocketAddr,
    token: &str,
    progress: Option<BuildProgress>,
) -> Result<Running> {
    let requirements = resolve_inspector_requirements(host, project_path).await?;
    let inspector_app_path = inspector_support_path(host)?;
    ensure_inspector_support_app(host, &inspector_app_path, &requirements).await?;

    let project = crate::hydrolysis::backend::open_ready(host, &inspector_app_path)
        .await
        .wrap_err("Failed to open inspector support app project")?;

    let mut run_options = RunOptions::new();
    run_options.insert_env_var(
        "WATERUI_INSPECTOR_TARGET_ADDR".to_string(),
        addr.to_string(),
    );
    run_options.insert_env_var("WATERUI_INSPECTOR_TOKEN".to_string(), token.to_string());

    let mut build_options = BuildOptions::development(BuildProfile::Debug);
    if let Some(progress) = &progress {
        build_options = build_options.with_progress(progress.clone());
    }
    let mut package_options = PackageOptions::development();
    if let Some(progress) = progress {
        package_options = package_options.with_progress(progress);
    }

    info!("Building and running the inspector client on this machine...");
    let built = build_hydrolysis(&project, platform, build_options)
        .await
        .map_err(|e| eyre::eyre!("Failed to build inspector app: {e}"))?;
    let artifact = package_hydrolysis(&project, platform, package_options, Some(&built))
        .await
        .map_err(|e| eyre::eyre!("Failed to package inspector app: {e}"))?;
    Local
        .run(project.host(), artifact, run_options)
        .await
        .map_err(|e| eyre::eyre!("Failed to run inspector app: {e}"))
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
        None,
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
    crate::project::CreateOptions {
        name: "WaterUI Inspector".to_string(),
        bundle_identifier: crate::project_types::BundleIdentifier::try_from(
            "dev.waterui.inspector",
        )
        .expect("inspector support bundle identifier must be valid"),
        waterui_path,
        channel: None,
        framework_manifest: None,
        framework: None,
        framework_lock: None,
        author: String::new(),
        web: None,
    }
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

    use target_lexicon::{Environment, OperatingSystem, Triple};

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

        // The inspected project does not resolve, so the launch fails
        // after the forward opened.
        let options = super::InspectorLaunchOptions {
            target: super::InspectorTarget::AndroidDevice {
                serial: String::from("pixel-9-pro:5555"),
                port: 23106,
            },
            token: String::from("token"),
        };
        let error = smol::block_on(super::launch_inspector_session(
            &host,
            std::path::Path::new("/nonexistent/project"),
            options,
            None,
        ))
        .expect_err("a missing project cannot launch");

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

        let options = super::InspectorLaunchOptions {
            target: super::InspectorTarget::AndroidDevice {
                serial: String::from("pixel-9-pro:5555"),
                port: 23106,
            },
            token: String::from("token"),
        };
        let error = smol::block_on(super::launch_inspector_session(
            &host,
            std::path::Path::new("/nonexistent/project"),
            options,
            None,
        ))
        .expect_err("a missing project cannot launch");

        let rendered = format!("{error:#}");
        // The primary failure names the project that could not launch.
        assert!(
            rendered.contains("nonexistent") || rendered.contains("project"),
            "the launch failure is carried: {rendered}"
        );
        // The cleanup failure travels with it, not swallowed.
        assert!(
            rendered.contains("Also failed to remove the inspector's forwarding"),
            "the removal failure is carried: {rendered}"
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
