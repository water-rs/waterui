//! Hydrolysis platform build and package utilities.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use eyre::{Context, bail};
use futures_util::FutureExt as _;
use smol::{
    channel::{Sender, bounded},
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use target_lexicon::{OperatingSystem, Triple};
use tracing::info;

use crate::{
    assets, browser_runtime,
    build::{
        ArtifactLockScope, BuildOptions, BuiltTarget, RustBuild, RustDynamicLibraries, RustLinkage,
        stage_dxc_runtime,
    },
    device::Artifact,
    hydrolysis::backend::HydrolysisBackend,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    toolchain::{ToolchainError, windows_arm64_llvm::WindowsArm64LlvmToolchain},
    utils::{command, run_command_os, which},
};
#[cfg(target_os = "macos")]
use crate::{
    macos_bundle::{
        MacOsAppNames, MacOsSigning, MacOsUsageDescription, package_binary_as_app,
        package_cef_helper_app, sign_macos_app as sign_app,
    },
    platform::PackageAudience,
    project::BrowserRuntimePlan,
};

#[cfg(target_os = "macos")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform macos --backend hydrolysis";
#[cfg(target_os = "linux")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform linux --backend hydrolysis";
#[cfg(target_os = "windows")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform windows --backend hydrolysis";
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const HYDROLYSIS_INIT_HINT: &str = "initialize hydrolysis backend on macOS, Linux, or Windows";

/// Loader search paths the platform's dynamic linker resolves the shared runtime through.
///
/// Both situations a backend binary runs in need an entry. `water preview` and an
/// unpackaged `water run` execute the binary where Cargo left it, with the shared
/// runtime staged beside it — that is `@executable_path` on macOS and `$ORIGIN` on
/// Linux. Packaging then moves the runtime into `Contents/Frameworks`, which only
/// the bundle-relative entry reaches. macOS carried the bundle path alone, so a
/// binary run in place could not load the runtime at all (#140).
const fn hydrolysis_loader_search_paths(platform: TargetPlatform) -> &'static [&'static str] {
    match platform {
        TargetPlatform::MacOS => &["@executable_path", "@executable_path/../Frameworks"],
        TargetPlatform::Linux => &["$ORIGIN"],
        _ => &[],
    }
}

/// The CEF subprocess helper binary the generated hydrolysis crate declares.
///
/// `templates::hydrolysis` emits the helper `[[bin]]` under exactly this name
/// — [`cef_helper_binary_name`] of the generated package — so the build and
/// the packaging lookup must both resolve it through here.
fn hydrolysis_cef_helper_name(backend_crate_name: &str) -> String {
    crate::project_model::project_types::cef_helper_binary_name(backend_crate_name)
}

/// Build hydrolysis binary for the host platform.
///
/// # Errors
/// Returns an error if the platform is unsupported, the backend is missing, or Cargo fails.
pub async fn build_hydrolysis(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<BuiltTarget> {
    Box::pin(build_hydrolysis_with_envs_and_features(
        project,
        platform,
        options,
        &[],
        &[],
    ))
    .await
}

/// Build hydrolysis binary for the host platform with extra Cargo environment variables.
///
/// # Errors
/// Returns an error if the platform is unsupported, the backend is missing, or Cargo fails.
pub async fn build_hydrolysis_with_envs(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
    extra_envs: &[(String, OsString)],
) -> eyre::Result<BuiltTarget> {
    Box::pin(build_hydrolysis_with_envs_and_features(
        project,
        platform,
        options,
        extra_envs,
        &[],
    ))
    .await
}

/// Build hydrolysis binary for the host platform with extra Cargo environment variables and features.
///
/// # Errors
/// Returns an error if the platform is unsupported, the backend is missing, or Cargo fails.
pub async fn build_hydrolysis_with_envs_and_features(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
    extra_envs: &[(String, OsString)],
    extra_features: &[&str],
) -> eyre::Result<BuiltTarget> {
    if !is_hydrolysis_native_platform(platform) {
        bail!("Hydrolysis backend is only supported on macOS, Linux, and Windows");
    }

    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");

    if !cargo_toml.exists() {
        bail!(
            "Hydrolysis backend not found at {}. Run `{HYDROLYSIS_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    // The managed crate is its own workspace root, so its `Cargo.lock` is
    // re-seeded from the application's lockfile before every resolution —
    // the binary built here must resolve the same graph the application's
    // own `cargo build` does (#178). `prepare_build` merges the same
    // lockfile into the managed lock again for channel-resolved projects;
    // this seed is the carrier when `waterui_path` pins a checkout instead —
    // the path is the record there, so the framework resolves from the
    // checkout, whose own `Cargo.lock` is the canonical pin.
    let has_framework =
        project.manifest().framework.is_some() || project.manifest().waterui_path.is_some();
    let canonical = if has_framework {
        project
            .resolved_framework()
            .await?
            .canonical_lock(project.root())
            .await?
    } else {
        None
    };
    crate::templates::seed_lockfile(
        &backend_path,
        &project.lockfile_path().await?,
        canonical.as_ref(),
    )
    .await?;

    // The generated `build.rs` embeds `app-icon.ico` into the executable when
    // targeting Windows, so it has to exist before the backend compiles.
    // Assets and fonts stage after the build instead: the mount metadata they
    // need is read from the library artifact this build produces.
    crate::templates::write_file_if_changed(
        &backend_path.join("app-icon.ico"),
        &assets::project_windows_ico(project)?,
    )
    .await?;

    let llvm_envs = WindowsArm64LlvmToolchain
        .cargo_envs(&crate::toolchain::Host::current())
        .await
        .map_err(|error| match error {
            ToolchainError::Fixable(_) => eyre::eyre!(
                "Windows ARM64 LLVM toolchain is missing. Run `water doctor --fix` to install it automatically."
            ),
            ToolchainError::Unfixable(unfixable) => {
                eyre::eyre!("Windows ARM64 LLVM toolchain check failed: {unfixable}")
            }
        })?;

    let mut build = RustBuild::new(&backend_path, platform.triple())
        .with_project(project)
        .with_target_dir(project.water_target_dir(options.linkage()).await?)
        .with_features(extra_features.iter().copied())
        .with_linkage(
            options.linkage(),
            &format!("{}/dev", project.crate_name()),
            hydrolysis_loader_search_paths(platform),
        )
        .with_envs(llvm_envs)
        .with_envs(options.cargo_envs().iter().cloned())
        .with_envs(extra_envs.iter().cloned());
    if let Some(sccache_path) = options.sccache_path() {
        build = build.with_sccache(sccache_path.to_path_buf());
    }
    if let Some(progress) = options.progress() {
        build = build.with_progress(progress.clone());
    }
    let mut built_target = Box::pin(build.build_binary(
        project.hydrolysis_backend_crate_name().as_str(),
        options.is_release(),
    ))
    .await
    .wrap_err("Failed to build hydrolysis backend with cargo")?;

    // The generated manifest declares the CEF helper as a second `[[bin]]`
    // when the application links the CEF engine crate; a `--bin <main>`
    // build never emits it, so it needs its own build before packaging can
    // bundle it. The gate is the manifest's own predicate — a
    // `waterui-chromium` link alone declares no helper bin, and asking
    // Cargo for it would fail with `no bin target`. Packaging resolves the
    // helper through its own `BuiltTarget`, kept here on the application's
    // — the marked `deps/` artifact path it reports is the only spelling
    // that names this variant's helper.
    if project.declares_cef_helper().await? {
        // Packaging reads the helper's marked `deps/` artifact; nothing
        // execs its shared `<profile>/<name>` uplift, so the artifact lock
        // is released once the marked link exists rather than held while
        // this `BuiltTarget` rides through packaging.
        let helper = Box::pin(
            build
                .with_artifact_lock_scope(ArtifactLockScope::UntilMarked)
                .build_binary(
                    &hydrolysis_cef_helper_name(project.hydrolysis_backend_crate_name().as_str()),
                    options.is_release(),
                ),
        )
        .await
        .wrap_err("Failed to build the hydrolysis CEF helper with cargo")?;
        built_target.cef_helper = Some(Box::new(helper));
    }

    copy_assets_and_fonts(
        project,
        &backend_path,
        &built_target.app_symbols().await?,
        options.uses_dev_server(),
    )
    .await?;

    Ok(built_target)
}

/// Stage the shared `WaterUI` runtime and Rust standard library next to a raw
/// Hydrolysis development binary.
///
/// Packaged applications stage these libraries in their platform runtime
/// directory. Preview and test binaries execute the `<profile>/<name>`
/// uplift, whose `@executable_path`/`$ORIGIN` entry resolves this adjacent
/// copy — the staging directory is the profile directory itself, never the
/// `deps/` the marked artifact's path names: `deps/` holds Cargo's own
/// unit outputs and pruning staged names there would delete sibling
/// variants' artifacts and re-dirty them on the next build.
///
/// # Errors
/// Returns an error if the required shared libraries cannot be resolved and staged.
pub(crate) async fn stage_hydrolysis_shared_runtime(
    project: &Project,
    built: &BuiltTarget,
    platform: TargetPlatform,
) -> eyre::Result<()> {
    if !is_hydrolysis_native_platform(platform) {
        bail!("Hydrolysis shared runtime can only be staged for macOS, Linux, and Windows");
    }
    let libraries = RustDynamicLibraries::resolve(built, &platform.triple(), project).await?;
    // The staged runtime lands beside the `<profile>/<name>` uplift the
    // launch path executes — the profile directory itself, never `deps/`:
    // `built.artifact` lives there and pruning staged names would delete
    // Cargo's own unit outputs.
    synchronize_shared_runtime(&built.profile_dir, Some(&libraries), &platform.triple()).await
}

/// The packaged CEF helper's source: the helper build's own `BuiltTarget`
/// artifact — the marked `deps/<helper>-<marker>` file this build produced —
/// never a name reconstructed under the application binary's `deps/` parent.
#[cfg(target_os = "macos")]
fn cef_helper_binary(helper: Option<&BuiltTarget>) -> eyre::Result<&Path> {
    helper
        .map(|helper| helper.artifact.as_path())
        .ok_or_else(|| {
            eyre::eyre!(
                "the project declares a CEF helper but the build produced none; package the result of `build_hydrolysis`, which keeps the helper's own `BuiltTarget`"
            )
        })
}

/// Clean Cargo build artifacts for hydrolysis.
///
/// # Errors
/// Returns an error if `cargo clean` fails or generated web output cannot be removed.
pub async fn clean_hydrolysis(project: &Project) -> eyre::Result<()> {
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");

    if !cargo_toml.exists() {
        return Ok(());
    }

    // The target directories are shared with every other generated backend, so only
    // this backend's own package is cleaned — its dependency artifacts stay for
    // the other backends that resolve them identically.
    for linkage in [RustLinkage::SharedRuntime, RustLinkage::Static] {
        let shared_target_dir = project.water_target_dir(linkage).await?;
        if !shared_target_dir.exists() {
            continue;
        }
        let clean_args: Vec<OsString> = vec![
            "clean".into(),
            "--manifest-path".into(),
            cargo_toml.as_os_str().to_owned(),
            "--target-dir".into(),
            shared_target_dir.as_os_str().to_owned(),
            "--package".into(),
            project.hydrolysis_backend_crate_name().as_str().into(),
        ];
        run_command_os("cargo", clean_args).await?;
    }
    let dist_web = backend_path.join("dist/web");
    if dist_web.exists() {
        fs::remove_dir_all(&dist_web).await?;
    }
    let dist_web_dev = backend_path.join("dist/web-dev");
    if dist_web_dev.exists() {
        fs::remove_dir_all(&dist_web_dev).await?;
    }
    Ok(())
}

/// Package a hydrolysis app.
///
/// Linux/Windows return a binary artifact path.
/// macOS returns a `.app` bundle path.
///
/// # Errors
/// Returns an error if packaging prerequisites are missing, assets cannot be staged, or output artifacts cannot be produced.
pub async fn package_hydrolysis(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
    built: Option<&BuiltTarget>,
) -> eyre::Result<Artifact> {
    if platform == TargetPlatform::Web {
        let site_root = package_hydrolysis_web_site(project, options.is_debug(), false).await?;
        return Ok(Artifact::new(project.bundle_identifier(), site_root));
    }

    let built = built.ok_or_else(|| {
        eyre::eyre!(
            "Hydrolysis packaging for {platform:?} needs the build result of the native backend binary"
        )
    })?;

    if !is_hydrolysis_native_platform(platform) {
        bail!(
            "Hydrolysis backend is only supported on macOS, Linux, and Windows for binary packaging"
        );
    }

    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let backend_path = project.backend_path::<HydrolysisBackend>();
    copy_assets_and_fonts(
        project,
        &backend_path,
        &built.app_symbols().await?,
        options.uses_dev_server(),
    )
    .await?;

    let final_binary_path = &built.artifact;
    let profile_directory = built.profile_dir.as_path();
    let runtime_plan = project
        .browser_runtime_plan(platform, crate::platform::TargetBackend::Hydrolysis)
        .await?;
    let shared_libraries = if options.uses_shared_rust_runtime() {
        Some(RustDynamicLibraries::resolve(built, &platform.triple(), project).await?)
    } else {
        None
    };

    #[cfg(target_os = "macos")]
    {
        if platform == TargetPlatform::MacOS {
            return package_hydrolysis_macos(HydrolysisMacosPackage {
                project,
                platform,
                backend_path: &backend_path,
                binary_path: final_binary_path,
                profile_directory,
                cef_helper: built.cef_helper.as_deref(),
                runtime_plan,
                shared_libraries: shared_libraries.as_ref(),
                options: &options,
            })
            .await;
        }
    }

    // The shipped binary and everything `$ORIGIN` resolves beside it stage
    // into the project's own managed backend directory — the shared Cargo
    // profile directory would collide two same-named projects on
    // `<profile>/<product>`.
    let runtime_dir = crate::platforming::packaging::dist_dir(
        &backend_path,
        crate::browser_runtime::platform_name(platform)?,
        Some(profile),
    );
    fs::create_dir_all(&runtime_dir).await?;
    synchronize_shared_runtime(&runtime_dir, shared_libraries.as_ref(), &platform.triple()).await?;
    browser_runtime::stage(runtime_plan, platform, profile_directory, &runtime_dir).await?;

    // Ship the binary under the product name; the tagged Cargo artifact name
    // is internal to the shared target directory.
    let binary_name = project.hydrolysis_binary_name();
    let shipped_name = if platform == TargetPlatform::Windows {
        format!("{binary_name}.exe")
    } else {
        binary_name.to_string()
    };
    let packaged_binary = crate::platforming::packaging::stage_binary_as(
        final_binary_path,
        &runtime_dir,
        &shipped_name,
    )
    .await?;

    // The runner enumerates `resources/` relative to the launched
    // executable — `exe_dir/resources` — never the managed manifest's own
    // `resources/` the build staged into, so the staged tree ships beside
    // the binary inside the dist directory (water-rs/hydrolysis#182).
    crate::platforming::packaging::stage_backend_resources(&backend_path, &runtime_dir).await?;

    if platform == TargetPlatform::Linux {
        crate::platforming::linux_share::write_linux_share_material(
            project,
            &project.manifest().package.name,
            binary_name.as_str(),
            &runtime_dir.join("share"),
        )
        .await?;
    }

    Ok(Artifact::new(project.bundle_identifier(), packaged_binary))
}

/// The inputs `package_hydrolysis_macos` assembles the `.app` from.
#[cfg(target_os = "macos")]
struct HydrolysisMacosPackage<'a> {
    /// The project being packaged.
    project: &'a Project,
    /// The platform being packaged — always `TargetPlatform::MacOS`.
    platform: TargetPlatform,
    /// The managed backend directory the bundle's resources come from.
    backend_path: &'a Path,
    /// The built application binary.
    binary_path: &'a Path,
    /// The Cargo profile directory staged resources resolve against.
    profile_directory: &'a Path,
    /// The CEF helper binary's own build result — `Some` when the project
    /// declared a helper `[[bin]]`; its `artifact` is the packaged helper's
    /// source, never a name reconstructed under the profile directory.
    cef_helper: Option<&'a BuiltTarget>,
    /// The browser runtime the bundle embeds.
    runtime_plan: BrowserRuntimePlan,
    /// The shared Rust runtime dylibs, when the package embeds them.
    shared_libraries: Option<&'a RustDynamicLibraries>,
    /// The packaging options — the audience decides how the bundle is signed.
    options: &'a PackageOptions,
}

#[cfg(target_os = "macos")]
async fn package_hydrolysis_macos(
    HydrolysisMacosPackage {
        project,
        platform,
        backend_path,
        binary_path,
        profile_directory,
        cef_helper,
        runtime_plan,
        shared_libraries,
        options,
    }: HydrolysisMacosPackage<'_>,
) -> eyre::Result<Artifact> {
    // The identifier lands in the bundle's `Info.plist` and drives signing —
    // reject an Apple-invalid one before any staging or SDK work.
    let bundle_id = project
        .bundle_identifier()
        .apple_bundle_identifier()
        .map_err(|error| eyre::eyre!("{error}"))?;
    let app_name = project
        .manifest()
        .package
        .name
        .chars()
        .filter(|character| character.is_alphanumeric() || *character == ' ')
        .collect::<String>();
    let app_name = if app_name.is_empty() {
        "WaterUIHydrolysis".to_string()
    } else {
        app_name
    };
    let dist_dir = crate::platforming::packaging::dist_dir(backend_path, "macos", None);
    fs::create_dir_all(&dist_dir).await?;
    let usage_descriptions = project
        .manifest()
        .permissions
        .iter()
        .filter(|(_, entry)| entry.is_enabled())
        .flat_map(|(key, entry)| {
            key.macos_usage_description_keys()
                .iter()
                .map(|&plist_key| MacOsUsageDescription {
                    plist_key,
                    description: entry.description().to_string(),
                })
        })
        .collect::<Vec<_>>();
    let icns = assets::project_macos_icns(project)?;
    let app_path = package_binary_as_app(
        binary_path,
        &bundle_id,
        MacOsAppNames {
            app_name: &app_name,
            executable_name: project.hydrolysis_binary_name().as_str(),
        },
        &usage_descriptions,
        Some(&backend_path.join("resources")),
        &icns,
        &dist_dir,
    )
    .await?;
    synchronize_shared_runtime(
        &app_path.join("Contents/Frameworks"),
        shared_libraries,
        &platform.triple(),
    )
    .await?;
    browser_runtime::stage_macos_app(runtime_plan, profile_directory, &app_path.join("Contents"))
        .await?;
    if project.declares_cef_helper().await? {
        let helper_binary = cef_helper_binary(cef_helper)?;
        // The helper apps are named after the shipped executable, so they
        // derive from the packaged copy — not the tagged Cargo artifact.
        let main_binary = app_path
            .join("Contents/MacOS")
            .join(project.hydrolysis_binary_name().as_str());
        let _helper_apps =
            package_cef_helper_app(&app_path, &main_binary, helper_binary, &bundle_id).await?;
    }
    let signing = match options.audience() {
        PackageAudience::Development => MacOsSigning::Development {
            requires_stable_identity: !usage_descriptions.is_empty(),
        },
        PackageAudience::Distribution => {
            MacOsSigning::Distribution(crate::macos_bundle::DistributionSigning::from_manifest(
                project.manifest().signing.macos.as_ref(),
                None,
            )?)
        }
    };
    sign_app(&app_path, &bundle_id, &signing).await?;
    Ok(Artifact::new(project.bundle_identifier(), app_path))
}

async fn synchronize_shared_runtime(
    destination: &Path,
    libraries: Option<&RustDynamicLibraries>,
    triple: &Triple,
) -> eyre::Result<()> {
    if let Some(libraries) = libraries {
        libraries.stage(destination).await?;
        // A shared-runtime hydrolysis binary renders through wgpu, whose
        // DirectX 12 backend `LoadLibrary`s `dxcompiler.dll` and `dxil.dll`
        // by name at run time; the pair has to sit beside the executable.
        if triple.operating_system == OperatingSystem::Windows {
            stage_dxc_runtime(destination).await?;
        }
        Ok(())
    } else {
        RustDynamicLibraries::remove_staged(destination, triple).await
    }
}

/// Check if a platform is supported by the hydrolysis backend.
#[must_use]
pub const fn is_hydrolysis_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::Linux
            | TargetPlatform::MacOS
            | TargetPlatform::Windows
            | TargetPlatform::Web
    )
}

const fn is_hydrolysis_native_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::Linux | TargetPlatform::MacOS | TargetPlatform::Windows
    )
}

/// Stage the project's assets and fonts under the backend's `resources`
/// directory. Runs after the backend build: `symbols` is the app library
/// artifact it produced, whose `waterui_meta_bundle_*` statics declare the
/// asset mounts.
async fn copy_assets_and_fonts(
    project: &Project,
    backend_path: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
) -> eyre::Result<()> {
    let resources_dir = backend_path.join("resources");
    fs::create_dir_all(&resources_dir).await?;
    let manifest =
        assets::stage_project_assets_for_gtk(project, &resources_dir, symbols, dev_server).await?;

    let font_declarations = assets::scan_fonts(project, &backend_path.join("Cargo.toml")).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);
    if !resolved_fonts.is_empty() {
        let fonts_dest = resources_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
        info!(
            "Copied {} fonts to hydrolysis resources",
            resolved_fonts.len()
        );
    }
    Ok(())
}

/// Build the Hydrolysis web site in debug mode for `water run --platform web`.
///
/// # Errors
/// Returns an error if the web site cannot be packaged.
pub async fn prepare_hydrolysis_web_dev_site(project: &Project) -> eyre::Result<PathBuf> {
    package_hydrolysis_web_site(project, true, true).await
}

/// A lightweight static-file server for packaged Hydrolysis web output.
#[derive(Debug)]
pub struct HydrolysisWebDevServer {
    address: SocketAddr,
    shutdown_tx: Option<Sender<()>>,
    _task: smol::Task<()>,
}

impl HydrolysisWebDevServer {
    /// Start serving the provided Hydrolysis web site root on a random localhost port.
    ///
    /// # Errors
    /// Returns an error if the local TCP listener cannot be bound.
    pub async fn start(site_root: PathBuf) -> eyre::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .wrap_err("Failed to bind Hydrolysis web dev server")?;
        let address = listener
            .local_addr()
            .wrap_err("Failed to resolve Hydrolysis web dev server address")?;
        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);

        let task = smol::spawn(async move {
            let shutdown = shutdown_rx.recv().fuse();
            futures_util::pin_mut!(shutdown);

            loop {
                let accept = listener.accept().fuse();
                futures_util::pin_mut!(accept);

                match futures_util::future::select(accept, shutdown.as_mut()).await {
                    futures_util::future::Either::Left((Ok((mut stream, _peer)), _)) => {
                        let site_root = site_root.clone();
                        smol::spawn(async move {
                            if let Err(error) = serve_http_request(&mut stream, &site_root).await {
                                tracing::warn!(
                                    target: "waterui::hydrolysis::web",
                                    error = %error,
                                    "Hydrolysis web dev server request failed"
                                );
                            }
                        })
                        .detach();
                    }
                    futures_util::future::Either::Left((Err(error), _)) => {
                        tracing::warn!(
                            target: "waterui::hydrolysis::web",
                            error = %error,
                            "Hydrolysis web dev server accept failed"
                        );
                        break;
                    }
                    futures_util::future::Either::Right((_shutdown, _)) => break,
                }
            }
        });

        Ok(Self {
            address,
            shutdown_tx: Some(shutdown_tx),
            _task: task,
        })
    }

    /// Get the bound localhost address for this server instance.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for HydrolysisWebDevServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }
    }
}

async fn package_hydrolysis_web_site(
    project: &Project,
    debug: bool,
    dev_site: bool,
) -> eyre::Result<PathBuf> {
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let lib_rs = backend_path.join("src/lib.rs");
    if !cargo_toml.exists() {
        bail!(
            "The generated Hydrolysis backend is missing at {}; `water build`, `water run` \
             and `water package` generate it.",
            backend_path.display(),
        );
    }
    if !lib_rs.exists() {
        bail!(
            "Hydrolysis backend at {} is missing src/lib.rs for web packaging. Re-scaffold the backend and try again.",
            backend_path.display(),
        );
    }

    let site_root = backend_path.join(if dev_site { "dist/web-dev" } else { "dist/web" });
    if site_root.exists() {
        fs::remove_dir_all(&site_root).await?;
    }
    fs::create_dir_all(&site_root).await?;

    // The shell is written after the bundle so the page knows the wasm size.
    build_hydrolysis_web_bundle(&backend_path, &site_root, debug).await?;
    super::web_launch::write_web_shell(project, &site_root).await?;
    copy_web_assets_and_fonts(project, &backend_path, &site_root).await?;

    Ok(site_root)
}

async fn build_hydrolysis_web_bundle(
    backend_path: &Path,
    site_root: &Path,
    debug: bool,
) -> eyre::Result<()> {
    let wasm_pack = which("wasm-pack")
        .await
        .wrap_err("wasm-pack is required to build Hydrolysis web bundles")?;
    let pkg_dir = site_root.join("pkg");
    fs::create_dir_all(&pkg_dir).await?;

    let mut wasm_pack_cmd = smol::process::Command::new(wasm_pack);
    let wasm_pack_cmd = command(&mut wasm_pack_cmd);
    wasm_pack_cmd
        .current_dir(backend_path)
        .arg("build")
        .arg("--target")
        .arg("web")
        .arg("--out-dir")
        .arg(&pkg_dir)
        .arg("--out-name")
        .arg("app");
    if debug {
        wasm_pack_cmd.arg("--dev");
    } else {
        wasm_pack_cmd.arg("--release");
    }

    let output = wasm_pack_cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let details = if stderr.trim().is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        bail!(
            "Failed to build Hydrolysis web bundle with wasm-pack (status {}):\n{}",
            output.status,
            details
        );
    }

    Ok(())
}

async fn copy_web_assets_and_fonts(
    project: &Project,
    backend_path: &Path,
    site_root: &Path,
) -> eyre::Result<()> {
    assets::stage_project_assets_for_web(project, site_root).await?;
    assets::stage_hydrolysis_web_fonts(project, backend_path, site_root).await?;
    Ok(())
}

async fn serve_http_request(
    stream: &mut smol::net::TcpStream,
    site_root: &Path,
) -> eyre::Result<()> {
    let mut buffer = vec![0u8; 8192];
    let bytes_read = stream
        .read(&mut buffer)
        .await
        .wrap_err("Failed to read HTTP request")?;
    if bytes_read == 0 {
        return Ok(());
    }

    let request = std::str::from_utf8(&buffer[..bytes_read])
        .wrap_err("Hydrolysis web dev server received non-UTF-8 request head")?;
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| eyre::eyre!("Hydrolysis web dev server received an empty request"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or("/");
    if method != "GET" && method != "HEAD" {
        write_response(
            stream,
            405,
            "text/plain; charset=utf-8",
            b"Method Not Allowed",
        )
        .await?;
        return Ok(());
    }

    let Ok(file_path) = resolve_site_path(site_root, path) else {
        write_response(stream, 404, "text/plain; charset=utf-8", b"Not Found").await?;
        return Ok(());
    };
    let body = match fs::read(&file_path).await {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_response(stream, 404, "text/plain; charset=utf-8", b"Not Found").await?;
            return Ok(());
        }
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("Failed to read {}", file_path.display()));
        }
    };
    let mime = mime_type_for_path(&file_path);
    if method == "HEAD" {
        write_headers(stream, 200, mime, body.len()).await?;
        return Ok(());
    }
    write_response(stream, 200, mime, &body).await
}

fn resolve_site_path(site_root: &Path, request_path: &str) -> eyre::Result<PathBuf> {
    let request_path = request_path.split('?').next().unwrap_or("/");
    let trimmed = request_path.trim_start_matches('/');
    let relative = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    let mut resolved = site_root.to_path_buf();
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\\') {
            bail!("Hydrolysis web dev server rejected invalid path {request_path}");
        }
        resolved.push(segment);
    }

    if resolved.is_dir() {
        resolved.push("index.html");
    }
    if !resolved.exists() {
        bail!("Hydrolysis web dev server could not resolve {request_path}");
    }
    Ok(resolved)
}

async fn write_headers(
    stream: &mut smol::net::TcpStream,
    status_code: u16,
    content_type: &str,
    content_length: usize,
) -> eyre::Result<()> {
    let status_text = match status_code {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {content_length}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(headers.as_bytes()).await?;
    Ok(())
}

async fn write_response(
    stream: &mut smol::net::TcpStream,
    status_code: u16,
    content_type: &str,
    body: &[u8],
) -> eyre::Result<()> {
    write_headers(stream, status_code, content_type, body.len()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

fn mime_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(std::ffi::OsStr::to_str) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::{
        framework::test_fixtures::stable_framework,
        project::ResolvedWebViewBackend,
        project_types::{BundleIdentifier, CrateName, declares_cef_helper},
        templates::TemplateContext,
    };

    fn demo_context() -> TemplateContext {
        TemplateContext::for_support_app(
            crate::templates::SupportAppIdentity {
                display_name: "Demo".to_string(),
                crate_name: CrateName::try_from("demo").expect("crate name must be valid"),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.demo")
                    .expect("bundle id must be valid"),
            },
            None,
            &stable_framework(),
            false,
            None,
            &crate::templates::LocalBackendSources::default(),
        )
        .with_project_packages(std::collections::BTreeSet::from(["demo".to_string()]))
    }

    fn rendered_bin_names(ctx: &TemplateContext, package_name: &str) -> Vec<String> {
        let cargo_toml = crate::templates::hydrolysis::rendered_outputs(ctx, package_name)
            .expect("hydrolysis outputs should render")
            .into_iter()
            .find_map(|(path, content)| {
                (path == Path::new("Cargo.toml"))
                    .then(|| String::from_utf8(content).expect("Cargo.toml must be UTF-8"))
            })
            .expect("hydrolysis Cargo.toml output should exist");
        cargo_toml
            .parse::<toml::Table>()
            .expect("hydrolysis Cargo.toml should parse")["bin"]
            .as_array()
            .expect("a hydrolysis manifest should declare binaries")
            .iter()
            .filter_map(|bin| bin["name"].as_str().map(str::to_string))
            .collect()
    }

    /// `package_hydrolysis_macos` locates the built helper under the name
    /// [`super::hydrolysis_cef_helper_name`] returns; the generated manifest
    /// declares the helper `[[bin]]` under `cef_helper_binary_name` of the
    /// package. Pin the two together so a rename on either side fails here
    /// instead of at packaging time on a user's machine.
    #[test]
    fn cef_helper_lookup_name_is_a_bin_the_manifest_declares() {
        let ctx = demo_context()
            .with_webview_enabled(true)
            .with_browser_engine(Some(ResolvedWebViewBackend::Cef));
        let package_name = "demo-hydrolysis-deadbeef";
        let bin_names = rendered_bin_names(&ctx, package_name);
        let helper_name = super::hydrolysis_cef_helper_name(package_name);
        assert!(
            bin_names.contains(&helper_name),
            "the helper the packager looks up must be a declared bin: {bin_names:?}"
        );
        assert!(
            bin_names.contains(&package_name.to_string()),
            "the main binary must remain declared too: {bin_names:?}"
        );
    }

    /// water-rs/hydrolysis#182: `native_resource_fonts` resolves
    /// `<exe_dir>/resources/fonts` — the directory beside the executable the
    /// CLI launches — and never beside the managed backend's `Cargo.toml`,
    /// which is where the build stages `resources/`. A packaged app
    /// therefore ships the staged tree inside `dist/<platform>/<profile>`,
    /// or the launched binary finds no fonts at all.
    #[test]
    fn a_packaged_app_ships_staged_fonts_beside_the_launched_executable() {
        smol::block_on(async {
            use crate::{
                platforming::platform::PackageOptions,
                project::{ManagedBackends, Project},
                project_model::project_types::{CrateName, generated_crate_name},
            };
            use tempfile::tempdir;

            let temporary = tempdir().expect("tempdir");
            let root = temporary.path().join("fixture");
            std::fs::create_dir_all(root.join("src")).expect("crate src");
            std::fs::write(
                root.join("Water.toml"),
                "[package]\nname = \"Fixture\"\n\
                 bundle_identifier = \"dev.waterui.fixture\"\n\n\
                 [[assets.font]]\nname = \"Fixture Sans\"\n\
                 local_path = \"assets/fonts/FixtureSans.ttf\"\n",
            )
            .expect("Water.toml");
            std::fs::write(
                root.join("Cargo.toml"),
                "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .expect("Cargo.toml");
            std::fs::write(root.join("src/lib.rs"), "").expect("lib.rs");

            // A `resources/fonts` face the app bundles.
            std::fs::create_dir_all(root.join("assets/fonts")).expect("assets dir");
            std::fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/fonts/Roboto-Regular.ttf"),
                root.join("assets/fonts/FixtureSans.ttf"),
            )
            .expect("fixture font");

            let project = Project::open(&root, ManagedBackends::NONE)
                .await
                .expect("fixture project opens");
            // The managed backend crate the build compiles, kept dependency-
            // free so its `cargo metadata` resolves without the network.
            let backend_path =
                project.backend_path::<crate::hydrolysis::backend::HydrolysisBackend>();
            std::fs::create_dir_all(backend_path.join("src")).expect("backend src");
            let backend_crate = generated_crate_name(
                &CrateName::try_from("fixture").expect("crate name"),
                "hydrolysis",
                &root,
            );
            std::fs::write(
                backend_path.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{backend_crate}\"\nversion = \"0.1.0\"\n\
                     edition = \"2021\"\n\n\
                     [[bin]]\nname = \"{backend_crate}\"\npath = \"src/main.rs\"\n"
                ),
            )
            .expect("backend manifest");
            std::fs::write(backend_path.join("src/main.rs"), "fn main() {}\n").expect("main.rs");

            let built = crate::build::RustBuild::new(&backend_path, target_lexicon::Triple::host())
                .with_target_dir(temporary.path().join("target"))
                .build_binary(backend_crate.as_str(), false)
                .await
                .expect("fixture backend builds");
            let artifact = super::package_hydrolysis(
                &project,
                crate::platform::TargetPlatform::Linux,
                PackageOptions::packaging(crate::platform::PackageAudience::Development, true),
                Some(&built),
            )
            .await
            .expect("packaging succeeds");

            // The face ships at the path the runner searches relative to the
            // launched executable: `<exe_dir>/resources/fonts`.
            let executable = artifact.path().to_path_buf();
            let searched = executable
                .parent()
                .expect("packaged executable has a directory")
                .join("resources")
                .join("fonts")
                .join("FixtureSans.ttf");
            assert!(
                searched.is_file(),
                "the packaged app ships the staged font beside the executable {}: {}",
                executable.display(),
                searched.display()
            );
        });
    }

    /// The build compiles the helper under
    /// [`crate::project_types::declares_cef_helper`] — the manifest's own
    /// predicate — not `BrowserRuntimePlan::requires_cef`, which is wider:
    /// a `waterui-chromium` link without a CEF engine still requires the CEF
    /// runtime but declares no helper `[[bin]]`, and gating the build on the
    /// wider predicate fails with Cargo's `no bin target`.
    #[test]
    fn chromium_without_the_cef_engine_declares_no_helper_bin() {
        let package_name = "demo-hydrolysis-deadbeef";

        // Chromium linked, no engine crate: the runtime plan requires CEF
        // but the manifest declares only the main binary.
        let ctx = demo_context().with_chromium_enabled(true);
        assert_eq!(rendered_bin_names(&ctx, package_name), [package_name]);

        // Chromium plus a non-CEF engine is the same shape.
        let ctx = demo_context()
            .with_chromium_enabled(true)
            .with_browser_engine(Some(ResolvedWebViewBackend::Wpe));
        assert_eq!(rendered_bin_names(&ctx, package_name), [package_name]);

        // The predicate the build and packaging gates consult agrees with
        // both renders — and still says yes for the CEF engine.
        assert!(declares_cef_helper(Some(ResolvedWebViewBackend::Cef)));
        assert!(!declares_cef_helper(Some(ResolvedWebViewBackend::Wpe)));
        assert!(!declares_cef_helper(None));
    }
}
