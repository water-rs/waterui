//! Inspector app launcher and session management.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use eyre::{Context as _, Result, bail};
use tracing::info;

use crate::build::BuildProgress;
use crate::device::{Device, Local, RunOptions, Running, StopRequest};
use crate::platform::TargetPlatform;
use crate::project::{ManagedBackends, Project};
use crate::runtime_compat::runtime_profile_tag;
use crate::runtime_fingerprint::{compute_runtime_fingerprint, runtime_package_identity};
use crate::support_app;
use crate::templates::TemplateContext;

use super::forward::AndroidForward;

const INSPECTOR_TEMPLATE_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
const INSPECTOR_METADATA_FILE: &str = ".waterui-inspector-signature";
/// Bumped whenever `scaffold_inspector_app` changes what it generates beyond
/// the templated files (manifest edits, permissions), which the template
/// fingerprint does not cover.
const INSPECTOR_SCAFFOLD_GENERATION: u32 = 2;

#[derive(Debug, Clone)]
struct InspectorRequirements {
    waterui_path: Option<PathBuf>,
    runtime_fingerprint: String,
}

/// The platform the inspector application itself runs on.
///
/// The inspector always runs where the developer is: an application on an
/// Android device is inspected from here too, over the `adb forward` an
/// [`InspectorTarget::AndroidDevice`] session opens — a phone cannot show
/// the inspector and keep the inspected application running, because
/// Android freezes whichever of the two is in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorPlatform {
    /// iOS Simulator.
    IosSimulator,
    /// macOS.
    Macos,
}

/// Where the endpoint the inspector attaches to lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorTarget {
    /// An endpoint this machine can dial directly.
    Local(SocketAddr),
    /// An endpoint on an Android device, reached through an `adb forward`
    /// the session owns.
    AndroidDevice {
        /// The device's adb serial.
        serial: String,
        /// The port the application listens on, as the device sees it.
        port: u16,
    },
}

/// Launch options for Inspector app.
#[derive(Debug, Clone)]
pub struct InspectorLaunchOptions {
    /// The endpoint of the application to inspect.
    pub target: InspectorTarget,
    /// One-time token used by runtime endpoint and inspector app.
    pub token: String,
}

/// Inspector session state.
#[derive(Debug)]
pub struct InspectorSession {
    /// Current platform.
    pub platform: InspectorPlatform,
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
    /// the `adb forward` it opened.
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
    /// the registration stays on the adb server too; the next session for
    /// the same device endpoint reuses it rather than opening a second one.
    pub fn detach(&mut self) {
        if let Some(running) = self.running.take() {
            std::mem::forget(running);
            self.owns_app = false;
        }
        self.forward = None;
    }
}

/// The build target the inspector app runs on for `platform`.
const fn inspector_target_platform(platform: InspectorPlatform) -> TargetPlatform {
    match platform {
        InspectorPlatform::Macos => TargetPlatform::MacOS,
        InspectorPlatform::IosSimulator => TargetPlatform::IOSSimulator,
    }
}

/// Every triple the inspector support app can build for — the serving set
/// of its manifest's `[profile]` overrides.
fn inspector_targets() -> Vec<target_lexicon::Triple> {
    [InspectorPlatform::Macos, InspectorPlatform::IosSimulator]
        .iter()
        .map(|platform| inspector_target_platform(*platform).triple())
        .collect()
}

/// Launch (or relaunch) an inspector support app.
///
/// # Errors
/// Returns an error if the endpoint cannot be reached, the support project
/// cannot be prepared, or the inspector app fails to launch.
pub async fn launch_inspector_session(
    host: &crate::toolchain::Host,
    project_path: &Path,
    platform: InspectorPlatform,
    options: InspectorLaunchOptions,
    progress: Option<BuildProgress>,
) -> Result<InspectorSession> {
    // The endpoint is resolved before the support app is built: a device
    // that cannot be reached is worth knowing about now rather than after
    // a compile.
    let (addr, forward) = match &options.target {
        InspectorTarget::Local(addr) => (*addr, None),
        InspectorTarget::AndroidDevice { serial, port } => {
            let forward = AndroidForward::open(host, serial, *port).await?;
            (forward.local_addr(), Some(forward))
        }
    };

    let requirements = resolve_inspector_requirements(host, project_path).await?;

    let inspector_app_path = inspector_support_path(host)?;
    ensure_inspector_support_app(host, &inspector_app_path, &requirements).await?;

    let project = Project::open(
        host,
        &inspector_app_path,
        ManagedBackends::for_platform(inspector_target_platform(platform)),
    )
    .await
    .wrap_err("Failed to open inspector support app project")?;

    let mut run_options = RunOptions::new();
    run_options.insert_env_var(
        "WATERUI_INSPECTOR_TARGET_ADDR".to_string(),
        addr.to_string(),
    );
    run_options.insert_env_var("WATERUI_INSPECTOR_TOKEN".to_string(), options.token.clone());

    let running = match platform {
        InspectorPlatform::Macos => {
            let backend = project
                .apple_backend()
                .ok_or_else(|| eyre::eyre!("Apple backend not configured"))?;
            let device = Local;
            device.launch(host).await?;
            info!("Building and running inspector app on macOS...");
            project
                .run_with_options(
                    backend,
                    TargetPlatform::MacOS,
                    device,
                    run_options,
                    progress.clone(),
                )
                .await
                .map_err(|e| eyre::eyre!("Failed to run inspector app: {e}"))?
        }
        InspectorPlatform::IosSimulator => {
            let backend = project
                .apple_backend()
                .ok_or_else(|| eyre::eyre!("Apple backend not configured"))?;
            let simulator =
                crate::apple::device::AppleSimulator::select_ios(&project, None).await?;

            simulator.launch(host).await?;
            info!("Building and running inspector app on iOS Simulator...");
            project
                .run_with_options(
                    backend,
                    TargetPlatform::IOSSimulator,
                    simulator,
                    run_options,
                    progress.clone(),
                )
                .await
                .map_err(|e| eyre::eyre!("Failed to run inspector app: {e}"))?
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
    name: &str,
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

    use super::{InspectorRequirements, inspector_signature, inspector_targets};

    /// The inspector runs where the developer is, so the support app is
    /// built for this machine and the simulator only: an Android triple in
    /// its serving set would be an on-device inspector coming back.
    #[test]
    fn the_support_app_builds_for_no_android_triple() {
        let targets: Vec<String> = inspector_targets()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(
            !targets.iter().any(|triple| triple.contains("android")),
            "{targets:?}"
        );
        assert!(
            targets.iter().any(|triple| triple.contains("darwin")),
            "{targets:?}"
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
