//! The Hydrolysis Android host integration.
//!
//! The Kotlin host lives in the framework tree itself —
//! `hydrolysis-android-host-subdirectory` names the Gradle composite root
//! inside the selected framework source (#1428). The CLI materializes that
//! source: a managed shallow clone of the framework repository at the
//! selected revision for a channel project, the `waterui_path` checkout
//! itself for a local one. It renders the generated Gradle app against the
//! host inside it, and drives the same NDK/Gradle/ABI/signing/asset
//! machinery the Android platform backend uses — the launcher crate
//! `templates::hydrolysis` already produces is the cdylib the Kotlin
//! activity loads. Nothing here routes through the widget FFI companion.

use std::path::{Path, PathBuf};

use eyre::{Context, bail};
use serde::{Deserialize, Serialize};
use smol::fs;
use tracing::info;

use crate::{
    android::device::AndroidAbiProvider,
    android::{
        backend::manifest_permissions,
        output_metadata::{OutputKind, packaged_artifact},
        platform::{
            ANDROID_MAX_PAGE_SIZE_LINK_ARG, AndroidAbi, AndroidBuildContext, AndroidPlatform,
            android_cargo_envs, android_ffi_dependency_features, android_path_env, ndk_libcxx_path,
            resolve_android_build_context, run_gradle_tasks, staged_libs_need_libcxx,
        },
    },
    assets::{self, AndroidThemeParent},
    build::{BuildOptions, BuildProgress, BuiltTarget, RustBuild},
    device::{Artifact, Device, FailToRun, RunOptions, Running},
    framework::ResolvedFramework,
    hydrolysis::backend::HydrolysisBackend,
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::Project,
    templates::{
        self, HydrolysisAndroidPreviewTemplateEntry, HydrolysisAndroidTemplateEntry,
        TemplateContext,
    },
};

/// The painter the Hydrolysis Android host draws with.
///
/// `gpu` is the Vello attachment the host's `android/gpu` module ships;
/// `hwui` is the `RenderNode` painter of the planned `android/hwui` module.
/// Selection is explicit — `[hydrolysis] painter` in `Water.toml`
/// or `--painter` — and an absent module in the pinned checkout is an error
/// at scaffold time, never a substitution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum HydrolysisAndroidPainter {
    /// The `SurfaceView`-backed GPU band. Its own minimum is API 26; the
    /// generated app uses the maximum of that and the framework floor.
    #[default]
    Gpu,
    /// The `RenderNode` painter drawing through the host view hierarchy.
    /// Its own minimum is API 29; the generated app uses the maximum of
    /// that and the framework floor.
    Hwui,
}

impl HydrolysisAndroidPainter {
    /// The Gradle project inside the host checkout this painter provides.
    #[must_use]
    pub const fn host_module(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Hwui => "hwui",
        }
    }

    /// The `dev.waterui.hydrolysis` coordinate the app module declares.
    #[must_use]
    pub fn gradle_dependency(self) -> String {
        format!("dev.waterui.hydrolysis:{}", self.host_module())
    }

    /// The Android API level this painter's host module requires.
    #[must_use]
    pub const fn min_api_level(self) -> u32 {
        match self {
            // Painter minimums, not the framework floor. `template_entry`
            // takes the maximum of these and `android-min-api-level`.
            Self::Gpu => 26,
            Self::Hwui => 29,
        }
    }

    /// The band `View` this painter mounts as child 0 of the host view — the
    /// GPU painter's `HydrolysisGpuBand`. A painter that draws through the
    /// host view hierarchy itself ships no band.
    #[must_use]
    pub fn band_import(self) -> Option<String> {
        match self {
            Self::Gpu => Some("dev.waterui.hydrolysis.gpu.HydrolysisGpuBand".to_string()),
            Self::Hwui => None,
        }
    }

    /// The band class name [`Self::band_import`] supplies, when set.
    #[must_use]
    pub fn band_class(self) -> Option<String> {
        match self {
            Self::Gpu => Some("HydrolysisGpuBand".to_string()),
            Self::Hwui => None,
        }
    }

    /// The CLI's own spelling of the painter, for help text and messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Hwui => "hwui",
        }
    }
}

impl std::fmt::Display for HydrolysisAndroidPainter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Resolve the Android painter for a Hydrolysis build: the command's
/// `--painter` override, else `[hydrolysis] painter` in
/// `Water.toml`, else the GPU painter the plan names the default.
#[must_use]
pub fn resolve_painter(
    project: &Project,
    painter_override: Option<HydrolysisAndroidPainter>,
) -> HydrolysisAndroidPainter {
    painter_override
        .or_else(|| {
            project
                .manifest()
                .hydrolysis
                .as_ref()
                .and_then(|config| config.painter)
        })
        .unwrap_or_default()
}

/// The stamped marker recording which host revision a managed checkout holds.
const HOST_STAMP_FILE: &str = ".waterui-host-revision";

/// The managed checkout of the pinned Hydrolysis Android host for
/// `revision`, under the backend's cache directory.
fn android_host_dir(backend_path: &Path, revision: &str) -> PathBuf {
    backend_path.join("android-host").join(revision)
}

/// The generated Gradle project's root inside the managed backend.
fn android_dir(backend_path: &Path) -> PathBuf {
    backend_path.join("android")
}

/// Ensure the checkout the Hydrolysis Android host lives inside exists for
/// `resolved` and return its root.
///
/// The host is a subdirectory of the selected framework source —
/// `hydrolysis-android-host-subdirectory` names the Gradle root inside the
/// checkout this returns. For a channel selection the source is a shallow
/// clone of the framework repository at the selected revision, materialized
/// under `<backend>/android-host/<revision>` — a stamp file records the
/// revision a directory was produced for so a repin replaces it. For a
/// `waterui_path` project the framework checkout itself is the host
/// checkout; no clone exists for a filesystem source. The caller passes the
/// resolution it already holds, so one that also reads the resolution's
/// other metadata resolves it once.
///
/// # Errors
///
/// Returns an error when the framework's host coordinates are missing or
/// the git fetch fails.
async fn materialize_android_host(
    project: &Project,
    resolved: &ResolvedFramework,
) -> eyre::Result<PathBuf> {
    let host = project.host();
    let (url, revision) = match resolved.hydrolysis_android_host()? {
        crate::framework::HydrolysisAndroidHost::Git { url, revision } => (url, revision),
        crate::framework::HydrolysisAndroidHost::Local { root } => return Ok(root.to_path_buf()),
    };
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let host_dir = android_host_dir(&backend_path, revision);

    let stamped = fs::read_to_string(host_dir.join(HOST_STAMP_FILE))
        .await
        .ok()
        .map(|contents| contents.trim().to_string());
    if stamped.as_deref() == Some(revision) {
        return Ok(host_dir);
    }

    if host_dir.exists() {
        fs::remove_dir_all(&host_dir).await?;
    }
    fs::create_dir_all(&host_dir).await?;

    let dir = host_dir.to_string_lossy().into_owned();
    host.run("git", ["-C", dir.as_str(), "init", "-q"])
        .await
        .wrap_err_with(|| {
            format!(
                "failed to initialize the hydrolysis android host checkout at {}",
                host_dir.display()
            )
        })?;
    host.run("git", ["-C", dir.as_str(), "remote", "add", "origin", url])
        .await
        .wrap_err("failed to configure the hydrolysis android host remote")?;
    host.run(
        "git",
        [
            "-C",
            dir.as_str(),
            "fetch",
            "-q",
            "--depth",
            "1",
            "origin",
            revision,
        ],
    )
    .await
    .wrap_err_with(|| {
        format!("failed to fetch the hydrolysis android host at revision {revision} from {url}")
    })?;
    host.run(
        "git",
        [
            "-C",
            dir.as_str(),
            "checkout",
            "-q",
            "--detach",
            "FETCH_HEAD",
        ],
    )
    .await
    .wrap_err("failed to check out the pinned hydrolysis android host")?;

    fs::write(host_dir.join(HOST_STAMP_FILE), revision).await?;
    info!(
        "Checked out hydrolysis android host {} into {}",
        revision,
        host_dir.display()
    );
    Ok(host_dir)
}

/// The Gradle project directory inside the host checkout the framework's
/// `hydrolysis-android-host-subdirectory` names — with the selected
/// painter's module verified inside it.
///
/// # Errors
///
/// Returns an error when the checkout ships no module for `painter` — an
/// explicit selection error, never a runtime fallback.
async fn require_painter_module(
    project: &Project,
    painter: HydrolysisAndroidPainter,
) -> eyre::Result<PathBuf> {
    let resolved = project.resolved_framework().await?;
    let host_root = materialize_android_host(project, &resolved).await?;
    let subdirectory = resolved.hydrolysis_android_host_subdirectory()?;
    let host_project_dir = host_root.join(subdirectory);
    require_host_module(
        &host_project_dir,
        painter.host_module(),
        "painter selection is explicit and never falls back",
    )?;
    Ok(host_project_dir)
}

/// The pinned host checkout must ship the Gradle `module` the selection
/// needs — a painter band (`gpu`) or the system-WebView bridge (`webview`) —
/// or the scaffold is an error here, never at Gradle's dependency
/// substitution.
///
/// # Errors
///
/// Returns an error when the checkout ships no `module/` directory.
fn require_host_module(host_project_dir: &Path, module: &str, reason: &str) -> eyre::Result<()> {
    let module_dir = host_project_dir.join(module);
    if !module_dir.is_dir() {
        bail!(
            "the hydrolysis android host at {} ships no `{module}` module: {} does not exist; \
             {reason}",
            host_project_dir.display(),
            module_dir.display()
        );
    }
    Ok(())
}

/// The template entry the generated `android/` Gradle project renders with.
async fn template_entry(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    host_project_dir: &Path,
    system_webview: bool,
) -> eyre::Result<HydrolysisAndroidTemplateEntry> {
    // The system-WebView decision arrives already made: the context's
    // `webview_backend_feature` is the one predicate the Gradle module flag
    // and the Cargo `webview-system` feature both read. An engine this pair
    // cannot host is an error upstream in `browser_runtime_plan`, before the
    // scaffold ever runs.
    if system_webview {
        require_host_module(
            host_project_dir,
            "webview",
            "the app's `webview` feature selection needs the system-WebView bridge",
        )?;
    }

    let backend_path = project.backend_path::<HydrolysisBackend>();
    let android_dir = android_dir(&backend_path);
    let host_project_dir =
        pathdiff::diff_paths(host_project_dir, &android_dir).ok_or_else(|| {
            eyre::eyre!(
                "cannot express the hydrolysis android host at {} relative to {}",
                host_project_dir.display(),
                android_dir.display()
            )
        })?;
    let project_root = pathdiff::diff_paths(project.root(), &android_dir).ok_or_else(|| {
        eyre::eyre!(
            "cannot express the project root at {} relative to {}",
            project.root().display(),
            android_dir.display()
        )
    })?;

    let framework_min = project
        .resolved_framework()
        .await?
        .android_min_api_level()?;

    Ok(HydrolysisAndroidTemplateEntry {
        native_library_name: templates::hydrolysis::hydrolysis_library_target_name(
            &project.hydrolysis_backend_crate_name(),
        ),
        host_project_dir: host_project_dir.to_string_lossy().replace('\\', "/"),
        project_root: project_root.to_string_lossy().replace('\\', "/"),
        painter_dependency: painter.gradle_dependency(),
        painter_module: painter.host_module().to_string(),
        min_api_level: framework_min.max(painter.min_api_level()),
        painter_band_import: painter.band_import(),
        painter_band_class: painter.band_class(),
    })
}

/// The template context the generated Gradle app renders with: the shared
/// launcher context plus the painter/host parameters and the manifest's
/// enabled Android permissions.
async fn android_template_context(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    host_project_dir: &Path,
) -> eyre::Result<crate::templates::TemplateContext> {
    // The scaffold renders the identifier as the app's Java package name —
    // reject an Android-invalid one before the Gradle project exists.
    let _ = project
        .bundle_identifier()
        .android_package_name()
        .map_err(|error| eyre::eyre!("{error}"))?;
    let ctx =
        HydrolysisBackend::template_context(project, &project.resolved_framework().await?).await?;
    let system_webview = ctx.webview_backend_feature().is_some();
    Ok(ctx
        .with_hydrolysis_android(
            template_entry(project, painter, host_project_dir, system_webview).await?,
        )
        .with_android_permissions(manifest_permissions(project.manifest())))
}

/// Render the generated Gradle app into `<backend>/android` for `painter`.
///
/// The ffi companion is rendered for this invocation first: it is not a
/// managed native backend, so `Project::open` never produces one for a
/// hydrolysis selection, yet `package_with_abis` reads its manifest to
/// mirror the app's feature selection onto the Gradle classpath. A
/// companion left over from a different selection must not be the one it
/// sees, so this renders it unconditionally — never an Apple one, this
/// backend has no Apple entry.
///
/// # Errors
///
/// Returns an error when template rendering or file writing fails.
pub async fn scaffold_android_project(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    host_project_dir: &Path,
) -> eyre::Result<()> {
    // Validate the engine plan for this pair before anything is scaffolded:
    // an app linking an engine (Android, Hydrolysis) cannot host fails here.
    project
        .browser_runtime_plan(TargetPlatform::Android, TargetBackend::Hydrolysis)
        .await?;
    let backend_path = project.backend_path::<HydrolysisBackend>();
    project.scaffold_ffi_companion(false).await?;
    let ctx = android_template_context(project, painter, host_project_dir).await?;
    templates::hydrolysis_android::scaffold(project.host(), &android_dir(&backend_path), &ctx)
        .await?;
    Ok(())
}

/// Every file the `android/` scaffold would write, as android-dir-relative
/// path and content — the regeneration check's comparison source.
///
/// # Errors
///
/// Returns an error when the template context or rendering fails.
pub async fn rendered_android_outputs(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    host_project_dir: &Path,
) -> eyre::Result<Vec<(PathBuf, Vec<u8>)>> {
    let ctx = android_template_context(project, painter, host_project_dir).await?;
    Ok(templates::hydrolysis_android::rendered_outputs(&ctx)?)
}

/// The `waterui_android` Gradle project needs these before `cargo build`:
/// `icons.json` and the other fetched font metadata the icon crates' build
/// scripts read.
async fn resolve_declared_fonts(project: &Project) -> eyre::Result<()> {
    let declarations = assets::scan_fonts(
        project,
        &project
            .backend_path::<HydrolysisBackend>()
            .join("Cargo.toml"),
    )
    .await?;
    let _resolved = assets::resolve_fonts(project.host(), declarations).await?;
    Ok(())
}

/// What an Android launcher build leaves behind: the cargo result, the
/// resolved NDK/SDK context callers reuse for post-build steps like
/// `llvm-strip`, and the staged shared libraries in `System.load` order.
#[derive(Debug)]
pub struct HydrolysisAndroidBuild {
    /// The cargo build result — the cdylib artifact and its app symbols.
    pub built: BuiltTarget,
    /// The build context this build resolved — the NDK path, linker and
    /// friends — so a caller post-processing the artifact never resolves
    /// the same toolchain a second time.
    pub(crate) context: AndroidBuildContext,
    /// The staged shared-library file names in load order:
    /// `libc++_shared.so` first when the staged libraries needed the STL,
    /// the launcher cdylib last.
    pub staged_libraries: Vec<String>,
}

/// Build the Hydrolysis launcher crate's cdylib for one Android ABI.
///
/// The same configured environment and caller-built application the desktop
/// Hydrolysis backend uses, cross-compiled to `abi` with the 16 KiB
/// page-alignment flag and staged under the generated app's `jniLibs` (or
/// `options.output_dir()` when Gradle delegates back to `water build`).
///
/// # Errors
///
/// Returns an error when the NDK or SDK cannot be resolved or the build or
/// staging fails.
pub async fn build(
    project: &Project,
    abi: AndroidAbi,
    options: BuildOptions,
) -> eyre::Result<BuiltTarget> {
    Ok(build_with_features(project, abi, options, &[]).await?.built)
}

/// [`build`] with extra Cargo features — the preview entry compiles the
/// launcher cdylib with `waterui-preview-mode`.
///
/// # Errors
///
/// Returns an error when the NDK or SDK cannot be resolved or the build or
/// staging fails.
pub async fn build_with_features(
    project: &Project,
    abi: AndroidAbi,
    options: BuildOptions,
    features: &[&str],
) -> eyre::Result<HydrolysisAndroidBuild> {
    // Validate the engine plan for this pair before any work runs: the same
    // bound the desktop `hydrolysis::backend` enforces on its own path.
    project
        .browser_runtime_plan(TargetPlatform::Android, TargetBackend::Hydrolysis)
        .await?;

    let host = project.host();
    // `-Cprefer-dynamic` on Android cannot resolve `std` to rustup's
    // prebuilt `libstd.so`, so the launcher — which never `dlopen`s modules —
    // links the runtime in.
    let options = options.with_static_runtime();

    let backend_path = project.backend_path::<HydrolysisBackend>();
    if !backend_path.join("Cargo.toml").is_file() {
        bail!(
            "Hydrolysis backend not found at {}. Run `water run --platform android --backend hydrolysis` to initialize it.",
            backend_path.display()
        );
    }

    resolve_declared_fonts(project).await?;

    let platform = AndroidPlatform::new(abi);
    let triple = platform.triple();
    let min_api_level = project
        .resolved_framework()
        .await?
        .android_min_api_level()?;
    let context = resolve_android_build_context(host, abi, &triple, min_api_level).await?;

    let rust_build = RustBuild::for_project(project, &backend_path, triple.clone())
        .with_crate_type_override("cdylib")
        .with_rustc_flag(ANDROID_MAX_PAGE_SIZE_LINK_ARG)
        .with_target_dir(project.water_target_dir(options.linkage()).await?)
        .with_features(features.iter().copied())
        .with_envs(options.cargo_envs().iter().cloned());
    let rust_build = rust_build
        .with_envs(android_cargo_envs(&context, &triple))
        .with_env("PATH", android_path_env(host, &context).await?);

    let built = rust_build
        .build_lib(options.is_release())
        .await
        .wrap_err("failed to build the hydrolysis android launcher with cargo")?;

    let staged_libraries = copy_build_outputs(project, &options, abi, &context, &built).await?;
    Ok(HydrolysisAndroidBuild {
        built,
        context,
        staged_libraries,
    })
}

/// Stage the built cdylib where the generated Gradle project packages it:
/// `app/src/main/jniLibs/<abi>/`, plus `libc++_shared.so` when the staged
/// libraries need the STL, with 16 KiB `LOAD`-segment alignment enforced.
/// Returns the staged library file names in load order — `libc++_shared.so`
/// first, the launcher cdylib last.
async fn copy_build_outputs(
    project: &Project,
    options: &BuildOptions,
    abi: AndroidAbi,
    context: &AndroidBuildContext,
    built: &BuiltTarget,
) -> eyre::Result<Vec<String>> {
    let output_dir = options.output_dir().map_or_else(
        || {
            android_dir(&project.backend_path::<HydrolysisBackend>())
                .join("app/src/main/jniLibs")
                .join(abi.as_str())
        },
        PathBuf::from,
    );
    fs::create_dir_all(&output_dir).await?;

    let library_name = format!(
        "lib{}.so",
        templates::hydrolysis::hydrolysis_library_target_name(
            &project.hydrolysis_backend_crate_name()
        )
    );
    let library = output_dir.join(&library_name);
    crate::utils::copy_file_if_changed(&built.artifact, &library).await?;

    // The NDK's shared STL follows the libraries that actually need it —
    // a Rust-only build never does — and loads first, ahead of the cdylib
    // that depends on it.
    let mut staged = Vec::new();
    if staged_libs_need_libcxx(&output_dir).await? {
        let libcxx = "libc++_shared.so".to_string();
        crate::utils::copy_file_if_changed(
            ndk_libcxx_path(&context.ndk_path, abi),
            output_dir.join(&libcxx),
        )
        .await?;
        staged.push(libcxx);
    }
    staged.push(library_name);
    crate::elf::require_aligned_shared_libraries(&output_dir).await?;
    Ok(staged)
}

/// Stage the project's `res`/`assets` tree into the generated Gradle app —
/// platform-theme resources, no `WaterUIFonts.kt` (the host registers no
/// fonts by reflection).
async fn copy_assets(
    project: &Project,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
) -> eyre::Result<()> {
    let android_dir = android_dir(&project.backend_path::<HydrolysisBackend>());
    assets::stage_project_assets_for_android(
        project,
        &android_dir,
        symbols,
        dev_server,
        AndroidThemeParent::Platform,
    )
    .await?;
    Ok(())
}

/// Package the staged Android build into an APK or AAB for `abis`.
///
/// Fetches the pinned host, verifies the painter's module exists in it,
/// renders the generated Gradle project, stages assets, then runs the
/// Gradle assemble/bundle task — the same task matrix the Android platform
/// backend drives.
///
/// # Errors
///
/// Returns an error when the host fetch fails, the painter is unavailable,
/// or the Gradle build fails.
pub async fn package_with_abis(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    options: &PackageOptions,
    abis: &[AndroidAbi],
    built: &BuiltTarget,
    prepared: &crate::android::signing::PreparedSigning,
) -> eyre::Result<Artifact> {
    let host = project.host();
    // Prove the release-signing plan belongs to this project and these
    // options before any work — the same bound the Android platform backend
    // enforces; `[signing.android]` is an Android platform contract, not a
    // per-backend feature.
    let release_signing = prepared.release_signing_for(project.root(), options)?;

    // The identifier becomes the app's Java package name in the Gradle
    // build below — reject an Android-invalid one before the SDK runs.
    let _ = project
        .bundle_identifier()
        .android_package_name()
        .map_err(|error| eyre::eyre!("{error}"))?;

    let host_project_dir = require_painter_module(project, painter).await?;
    scaffold_android_project(project, painter, &host_project_dir).await?;

    copy_assets(project, &built.app_symbols()?, options.uses_dev_server()).await?;

    let android_dir = android_dir(&project.backend_path::<HydrolysisBackend>());

    // Kotlin helpers and Maven coordinates declared by the dependency graph go
    // on the app module's classpath so Gradle compiles them into the dex, and
    // its manifest components into the app manifest. The scan mirrors the
    // Rust build's feature selection so helpers behind optional features are
    // not missed.
    crate::assets::stage_android_declarations(
        project,
        &project.ffi_crate_path().join("Cargo.toml"),
        &android_dir.join("app"),
        crate::assets::AndroidDependencyScope::Implementation,
        &android_ffi_dependency_features(project).await?,
    )
    .await?;

    let (command_name, output_kind, variant) = match (options.is_distribution(), options.is_debug())
    {
        (true, false) => ("bundleRelease", OutputKind::Bundle, "release"),
        (false, false) => ("assembleRelease", OutputKind::Apk, "release"),
        (false, true) => ("assembleDebug", OutputKind::Apk, "debug"),
        (true, true) => ("bundleDebug", OutputKind::Bundle, "debug"),
    };

    let abis_str = abis
        .iter()
        .map(|abi| abi.as_str())
        .collect::<Vec<_>>()
        .join(",");

    // The Rust cdylib is already staged under jniLibs; Gradle must not
    // rebuild it, and its ABI filters narrow to the requested set.
    // Release variants sign with the manifest's `[signing.android]` unless
    // the decision was `--unsigned`, which suppresses the generated
    // signingConfig through the environment; debug variants keep the Gradle
    // debug keystore.
    let mut envs = vec![
        ("WATERUI_SKIP_RUST_BUILD", "1".to_owned()),
        ("WATERUI_ANDROID_ABIS", abis_str),
    ];
    if release_signing == Some(crate::android::signing::ReleaseSigning::Suppressed) {
        envs.push((crate::android::signing::UNSIGNED_ENV, "1".to_owned()));
    }
    {
        // The app `includeBuild`s the host checkout, whose `build/` and
        // `.gradle/` every other build of that checkout — a preview host
        // included — writes too.
        let _host_build =
            crate::water_dir::android_host_build_lock(host, &host_project_dir).await?;
        run_gradle_tasks(host, &android_dir, &[command_name], &envs).await?;
    }

    let path = packaged_artifact(&android_dir, output_kind, variant).await?;
    Ok(Artifact::new(project.bundle_identifier(), path))
}

/// Build, package and run the project's Hydrolysis app on `device` as a
/// debug development build.
///
/// The library counterpart of `water run --platform android --backend
/// hydrolysis` for flows that pick their own device, such as the inspector
/// support app: the launcher crate is built for the device's ABI, the
/// generated Gradle app is assembled over the pinned host with `painter`, and
/// the APK is installed and launched with `run_options`. The project's
/// managed Hydrolysis backend must already be generated
/// ([`crate::hydrolysis::backend::open_ready`]).
///
/// # Errors
///
/// Returns an error when signing resolution, the build, the packaging, or the
/// launch fails.
pub async fn run_on_device<D: Device + AndroidAbiProvider>(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    device: D,
    run_options: RunOptions,
    build_options: BuildOptions,
    progress: Option<BuildProgress>,
) -> Result<Running, FailToRun> {
    let host = project.host();
    let abi = device.android_abi();

    clean_jni_libs(project).await.map_err(FailToRun::Build)?;

    let mut package_options = PackageOptions::development();
    if let Some(progress) = &progress {
        package_options = package_options.with_progress(progress.clone());
    }
    // Resolve release signing before the Rust build, as `water run` does: a
    // misconfigured release package fails before compilation. A debug run
    // resolves to a no-decision plan.
    let prepared = crate::android::signing::PreparedSigning::resolve(project, &package_options)
        .map_err(FailToRun::Package)?;

    let build_options = match progress {
        Some(progress) => build_options.with_progress(progress),
        None => build_options,
    };
    let built = build(project, abi, build_options)
        .await
        .map_err(FailToRun::Build)?;

    let artifact = package_with_abis(
        project,
        painter,
        &package_options,
        &[abi],
        &built,
        &prepared,
    )
    .await
    .map_err(FailToRun::Package)?;

    info!("Running on device");
    device.run(host, artifact, run_options).await
}

/// Remove the staged `jniLibs` under the generated Gradle app.
///
/// # Errors
///
/// Returns an error if the directory cannot be removed.
pub async fn clean_jni_libs(project: &Project) -> eyre::Result<()> {
    let jni_libs_dir =
        android_dir(&project.backend_path::<HydrolysisBackend>()).join("app/src/main/jniLibs");
    if jni_libs_dir.exists() {
        fs::remove_dir_all(&jni_libs_dir).await?;
    }
    Ok(())
}

/// The preview host APK's application id — also the `run-as` package a
/// preview render stages its payload under.
pub const PREVIEW_HOST_PACKAGE: &str = "dev.waterui.hydrolysis.preview";

/// The instrumentation `am instrument` runs for a preview render.
pub const PREVIEW_HOST_INSTRUMENTATION: &str = "dev.waterui.hydrolysis.preview/dev.waterui.hydrolysis.preview.HydrolysisPreviewInstrumentation";

/// The Gradle project the preview host renders into, inside the managed
/// Hydrolysis backend.
fn preview_host_dir(backend_path: &Path) -> PathBuf {
    backend_path.join("android-preview-host")
}

/// The stamp recording the fingerprint a generated preview host was built
/// for — a match plus a resolvable APK skips the Gradle build entirely.
const PREVIEW_HOST_STAMP_FILE: &str = ".waterui-preview-host-fingerprint";

/// The sha256 a preview-host fingerprint carries — typed so its
/// `versionCode` projection needs no re-parse.
struct PreviewHostFingerprint([u8; 32]);

impl PreviewHostFingerprint {
    /// The APK `versionCode` this fingerprint installs as: its first 4
    /// bytes, big-endian, folded into `1..=2_000_000_000` so the code stays
    /// below the platform ceiling and never zero.
    const fn version_code(&self) -> u32 {
        u32::from_be_bytes([self.0[0], self.0[1], self.0[2], self.0[3]]) % 2_000_000_000 + 1
    }
}

impl std::fmt::Display for PreviewHostFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

/// The sha256 of everything a preview host APK bakes in: the host's
/// rendered scaffold (`rendered_outputs` with `versionCode` held constant,
/// so the hash covers the inputs — template trees, `minSdk`, host path —
/// not the fingerprint's own projection), the declared input set of the
/// pinned host's `preview` module — `src/` plus `build.gradle.kts`, plus
/// its manifest when it lives outside `src/`, so a `.DS_Store` or IDE file
/// dropped beside them cannot force a rebuild — and the host's
/// `settings.gradle.kts` — hashed sorted by relative path, each entry
/// length-prefixed so no pair of inputs can alias.
///
/// # Errors
///
/// Returns an error when a host file cannot be walked or read.
fn preview_host_fingerprint(
    host_project_dir: &Path,
    rendered: &[(PathBuf, Vec<u8>)],
) -> eyre::Result<PreviewHostFingerprint> {
    use sha2::Digest as _;

    let mut fingerprint_inputs: Vec<(String, Vec<u8>)> = rendered
        .iter()
        .map(|(path, contents)| {
            (
                format!("rendered:{}", path.to_string_lossy().replace('\\', "/")),
                contents.clone(),
            )
        })
        .collect();

    let preview_module = host_project_dir.join("preview");
    let mut hash_module_file = |path: &Path| -> eyre::Result<()> {
        let relative = path
            .strip_prefix(host_project_dir)
            .map_err(|error| eyre::eyre!("{error}"))?
            .to_string_lossy()
            .replace('\\', "/");
        fingerprint_inputs.push((
            relative,
            std::fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?,
        ));
        Ok(())
    };

    for entry in walkdir::WalkDir::new(preview_module.join("src")).follow_links(false) {
        let entry = entry.wrap_err_with(|| {
            format!(
                "failed to walk the preview host module {}",
                preview_module.display()
            )
        })?;
        if entry.file_type().is_file() {
            hash_module_file(entry.path())?;
        }
    }
    // `build.gradle.kts` is a declared input: a module without one is a
    // broken host checkout, not an empty input. Only the module-root
    // manifest is optional — most modules keep theirs under `src/`.
    hash_module_file(&preview_module.join("build.gradle.kts"))?;
    let root_manifest = preview_module.join("AndroidManifest.xml");
    match std::fs::symlink_metadata(&root_manifest) {
        Ok(_) => hash_module_file(&root_manifest)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(eyre::Report::from(error))
                .wrap_err_with(|| format!("failed to inspect {}", root_manifest.display()));
        }
    }
    let settings = host_project_dir.join("settings.gradle.kts");
    fingerprint_inputs.push((
        "host:settings.gradle.kts".to_string(),
        std::fs::read(&settings).wrap_err_with(|| {
            format!(
                "failed to read the preview host settings {}",
                settings.display()
            )
        })?,
    ));

    fingerprint_inputs.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut hasher = sha2::Sha256::new();
    for (path, contents) in &fingerprint_inputs {
        hasher.update((path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update((contents.len() as u64).to_le_bytes());
        hasher.update(contents);
    }
    Ok(PreviewHostFingerprint(hasher.finalize().into()))
}

/// Build — or reuse — the `water preview --platform android` host APK.
///
/// The host is a composite over the pinned Hydrolysis Android checkout
/// (`hydrolysis-android-host-subdirectory`), rendered under
/// `<backend>/android-preview-host/`; its `versionCode` is the
/// `preview_host_fingerprint`'s projection, so a changed template or a
/// changed host `preview` module produces a different code and the device
/// receives a reinstall. Returns the APK path and that `versionCode`.
///
/// Every project's composite `includeBuild`s the one host checkout, whose
/// Gradle build directories the builds share, so the Gradle build runs
/// under the host's build lock.
///
/// # Errors
///
/// Returns an error when the host checkout ships no `preview` module, the
/// project cannot render, or the Gradle build fails.
pub async fn ensure_preview_host_apk(project: &Project) -> eyre::Result<(PathBuf, u32)> {
    let host = project.host();
    let host_dir = preview_host_dir(&project.backend_path::<HydrolysisBackend>());
    let composite = PreviewHostComposite::prepare(project, &host_dir).await?;

    // The fingerprint covers the rendered scaffold with `versionCode` held
    // constant — hashing the real code would feed the projection back into
    // its own input.
    let rendered = templates::hydrolysis_android_preview::rendered_outputs(&composite.context(0))?;
    let fingerprint = smol::unblock({
        let host_project_dir = composite.host_project_dir.clone();
        move || preview_host_fingerprint(&host_project_dir, &rendered)
    })
    .await?;
    let version_code = fingerprint.version_code();
    composite.write(version_code).await?;

    let stamp = host_dir.join(PREVIEW_HOST_STAMP_FILE);
    if fs::read_to_string(&stamp)
        .await
        .is_ok_and(|contents| contents.trim() == fingerprint.to_string())
        && let Ok(apk) = packaged_artifact(&host_dir, OutputKind::Apk, "debug").await
    {
        return Ok((apk, version_code));
    }

    {
        let _host_build =
            crate::water_dir::android_host_build_lock(host, &composite.host_project_dir).await?;
        info!("Building the hydrolysis preview host APK");
        run_gradle_tasks(host, &host_dir, &[":app:assembleDebug"], &[]).await?;
    }
    fs::write(&stamp, fingerprint.to_string()).await?;
    let apk = packaged_artifact(&host_dir, OutputKind::Apk, "debug").await?;
    Ok((apk, version_code))
}

/// Render the preview-host composite Gradle project into `out`, stamped
/// with `version_code`.
///
/// The composite's `:app` packages the instrumentation and
/// `includeBuild`s the project's Hydrolysis Android host, resolved exactly
/// as `materialize_android_host` and `hydrolysis-android-host-subdirectory`
/// resolve it for every other build.
///
/// [`ensure_preview_host_apk`] renders the managed host through the same
/// preparation, and the `render_preview_host` example calls this so CI can
/// assemble the composite without a device.
///
/// # Errors
///
/// Returns an error when the host checkout cannot be materialized or ships
/// no `preview` module, the project's framework resolution or template
/// context cannot be built, the host cannot be expressed relative to `out`,
/// or the scaffold cannot be written.
pub async fn render_preview_host(
    project: &Project,
    out: &Path,
    version_code: u32,
) -> eyre::Result<()> {
    PreviewHostComposite::prepare(project, out)
        .await?
        .write(version_code)
        .await
}

/// A preview-host composite ready to render into `out`: the host checkout
/// materialized and its `preview` module verified, and the template context
/// built once from one framework resolution — the only place its host path
/// and `minSdk` are assembled. Only the `versionCode` varies per render.
struct PreviewHostComposite {
    /// The machine the project was opened on; the scaffold's Android SDK
    /// probe runs on it.
    toolchain_host: crate::toolchain::Host,
    out: PathBuf,
    /// The host checkout's Gradle root the composite `includeBuild`s.
    host_project_dir: PathBuf,
    /// The launcher context, without the preview entry.
    context: TemplateContext,
    /// The preview entry, its `versionCode` filled in per render.
    entry: HydrolysisAndroidPreviewTemplateEntry,
}

impl PreviewHostComposite {
    async fn prepare(project: &Project, out: &Path) -> eyre::Result<Self> {
        let resolved = project.resolved_framework().await?;
        let host_root = materialize_android_host(project, &resolved).await?;
        let host_project_dir = host_root.join(resolved.hydrolysis_android_host_subdirectory()?);
        let preview_module = host_project_dir.join("preview");
        let metadata = fs::metadata(&preview_module).await.wrap_err_with(|| {
            format!(
                "the hydrolysis android host at {} ships no `preview` module: {} does not exist",
                host_root.display(),
                preview_module.display()
            )
        })?;
        if !metadata.is_dir() {
            bail!(
                "the hydrolysis android host at {} ships no `preview` module: {} is not a directory",
                host_root.display(),
                preview_module.display()
            );
        }
        let relative_host = pathdiff::diff_paths(&host_project_dir, out).ok_or_else(|| {
            eyre::eyre!(
                "cannot express the hydrolysis android host at {} relative to {}",
                host_project_dir.display(),
                out.display()
            )
        })?;
        let entry = HydrolysisAndroidPreviewTemplateEntry {
            host_project_dir: relative_host.to_string_lossy().replace('\\', "/"),
            min_api_level: resolved.android_min_api_level()?,
            version_code: 0,
        };
        Ok(Self {
            toolchain_host: project.host().clone(),
            out: out.to_path_buf(),
            host_project_dir,
            context: HydrolysisBackend::template_context(project, &resolved).await?,
            entry,
        })
    }

    /// The template context stamped with `version_code`.
    fn context(&self, version_code: u32) -> TemplateContext {
        self.context.clone().with_hydrolysis_android_preview(
            HydrolysisAndroidPreviewTemplateEntry {
                version_code,
                ..self.entry.clone()
            },
        )
    }

    /// Write the composite stamped with `version_code` into `out`.
    async fn write(&self, version_code: u32) -> eyre::Result<()> {
        templates::hydrolysis_android_preview::scaffold(
            &self.toolchain_host,
            &self.out,
            &self.context(version_code),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };

    use super::*;
    use crate::{
        framework::test_fixtures::stable_checkout_framework,
        project::{ManagedBackends, Manifest},
        toolchain::{
            Host,
            testing::{TestMachine, tool_file_name},
        },
    };

    /// A minimal project whose `Water.toml` records the stable framework
    /// resolution so `resolved_framework` answers offline.
    async fn fixture_project(host: &Host, extra_manifest: &str) -> (tempfile::TempDir, Project) {
        fixture_project_with_deps(host, extra_manifest, "").await
    }

    /// [`fixture_project`] with extra `[dependencies]` entries on the
    /// fixture's `Cargo.toml`.
    async fn fixture_project_with_deps(
        host: &Host,
        extra_manifest: &str,
        cargo_dependencies: &str,
    ) -> (tempfile::TempDir, Project) {
        let temporary = tempfile::tempdir().expect("tempdir");
        let root = temporary.path().join("fixture");
        std::fs::create_dir_all(root.join("src")).expect("crate src");
        let mut manifest = Manifest::parse(&format!(
            "[package]\nname = \"Fixture\"\nbundle_identifier = \"dev.waterui.fixture\"\n{extra_manifest}"
        ))
        .expect("Water.toml parses");
        manifest.framework = Some(stable_checkout_framework());
        std::fs::write(
            root.join("Water.toml"),
            toml::to_string(&manifest).expect("manifest serializes"),
        )
        .expect("Water.toml");
        std::fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{cargo_dependencies}"
            ),
        )
        .expect("Cargo.toml");
        // `cargo metadata --locked` reads the lockfile; a dependency-free
        // crate's lock holds only itself.
        std::fs::write(
            root.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .expect("Cargo.lock");
        std::fs::write(root.join("src/lib.rs"), "").expect("lib.rs");

        // The ffi companion's feature-table probe resolves the generated
        // manifest's registry pins through `cargo metadata`; the workspace
        // `[patch.crates-io]` table the scaffold propagates redirects them to
        // local stubs — the only source `[patch]` can redirect without a
        // crates.io index — so the probe exercises the real generated
        // manifest without a published `waterui-*` 0.4.1 to find.
        let vendor_dir = temporary.path().join("vendor");
        crate::framework::test_fixtures::write_vendor_stub(
            &vendor_dir.join("waterui"),
            "waterui",
            &["dynamic_linking", "media", "webview"],
        );
        crate::framework::test_fixtures::write_vendor_stub(
            &vendor_dir.join("waterui-ffi"),
            "waterui-ffi",
            &[
                "android-jni",
                "c-api",
                "chromium",
                "dev",
                "gpu",
                "inspector",
                "map",
                "media",
                "video",
                "webview",
                "webview-cef",
            ],
        );
        let manifest_path = root.join("Cargo.toml");
        let mut document: toml_edit::DocumentMut = std::fs::read_to_string(&manifest_path)
            .expect("project Cargo.toml exists")
            .parse()
            .expect("project Cargo.toml parses");
        for name in ["waterui", "waterui-ffi"] {
            document["patch"]["crates-io"][name]["path"] =
                toml_edit::value(vendor_dir.join(name).to_string_lossy().as_ref());
        }
        std::fs::write(&manifest_path, document.to_string()).expect("write the patch table");

        // `Project::open` resolves the project's layout with `cargo metadata
        // --locked`; a plain offline resolve records the patched sources in
        // the lock first.
        let mut command = cargo_metadata::MetadataCommand::new();
        command
            .manifest_path(&manifest_path)
            .other_options(vec!["--offline".to_string()]);
        host.cargo_metadata(&command)
            .await
            .expect("offline metadata resolves the patched project");

        let project = Project::open(host, &root, ManagedBackends::NONE)
            .await
            .expect("fixture project opens");
        (temporary, project)
    }

    /// A host whose `git` materializes `staged` — a fake framework checkout
    /// root carrying `backends/hydrolysis/android/<module>` directories as
    /// the host subdirectory — on checkout, as `materialize_android_host`'s fetch
    /// sequence would produce.
    fn machine_with_staged_host(staged: &Path, modules: &[&str]) -> (TestMachine, Host) {
        let machine = TestMachine::new();
        machine.install("git");
        for module in modules {
            let module_dir = machine.dir(staged.join("backends/hydrolysis/android").join(module));
            std::fs::write(module_dir.join("build.gradle.kts"), "// host module\n")
                .expect("staged gradle file");
        }
        let checkout = machine.root().join(staged).to_string_lossy().into_owned();
        let host = machine.host_with_rust_toolchain([("WATERUI_FAKE_GIT_CHECKOUT", checkout)]);
        (machine, host)
    }

    /// The painter is explicit: `--painter` wins over
    /// `[hydrolysis] painter`, which wins over the GPU default the
    /// plan names. Nothing in the chain substitutes silently.
    #[test]
    fn painter_resolution_prefers_the_override_then_the_manifest() {
        smol::block_on(async {
            let (_temporary, project) = fixture_project(&Host::current(), "").await;
            assert_eq!(
                resolve_painter(&project, None),
                HydrolysisAndroidPainter::Gpu
            );
            assert_eq!(
                resolve_painter(&project, Some(HydrolysisAndroidPainter::Hwui)),
                HydrolysisAndroidPainter::Hwui
            );

            let (_temporary, project) =
                fixture_project(&Host::current(), "\n[hydrolysis]\npainter = \"hwui\"\n").await;
            assert_eq!(
                resolve_painter(&project, None),
                HydrolysisAndroidPainter::Hwui
            );
            assert_eq!(
                resolve_painter(&project, Some(HydrolysisAndroidPainter::Gpu)),
                HydrolysisAndroidPainter::Gpu
            );
        });
    }

    /// A pinned host without a `preview` module is an explicit error naming
    /// the missing module — never a substitution.
    #[test]
    fn ensure_preview_host_apk_fails_without_the_preview_module() {
        smol::block_on(async {
            let (_machine, host) = machine_with_staged_host(Path::new("staged"), &["gpu"]);
            let (_temporary, project) = fixture_project(&host, "").await;

            let error = ensure_preview_host_apk(&project)
                .await
                .expect_err("a host without `preview` must fail");
            let message = format!("{error:#}");
            assert!(message.contains("preview"), "{message}");
            assert!(
                message.contains("backends/hydrolysis/android/preview"),
                "the error must name the missing module path: {message}"
            );
        });
    }

    /// The managed host checkout is the `git -C` init/remote/fetch/checkout
    /// sequence the fake `git` materializes; the stamp file then answers the
    /// next scaffold without a fetch at all.
    #[test]
    fn materialize_android_host_materializes_then_reuses_the_stamped_checkout() {
        smol::block_on(async {
            let (machine, host) = machine_with_staged_host(Path::new("staged"), &["gpu", "hwui"]);
            let (_temporary, project) = fixture_project(&host, "").await;

            let resolved = project
                .resolved_framework()
                .await
                .expect("framework resolves");
            let checkout = materialize_android_host(&project, &resolved)
                .await
                .expect("host materializes");
            // The checkout clones the framework repository itself at the
            // selected revision — the host lives inside it.
            let revision = "a".repeat(40);
            assert!(checkout.ends_with(&revision));
            assert!(
                checkout
                    .join("backends/hydrolysis/android/gpu/build.gradle.kts")
                    .is_file()
            );
            assert_eq!(
                std::fs::read_to_string(checkout.join(HOST_STAMP_FILE)).expect("stamp file"),
                revision
            );

            // A stamped checkout short-circuits before any fetch: remove the
            // fake git entirely and the host still resolves.
            std::fs::remove_file(machine.bin().join(tool_file_name("git")))
                .expect("remove fake git");
            let again = materialize_android_host(&project, &resolved)
                .await
                .expect("stamped checkout needs no git");
            assert_eq!(again, checkout);
        });
    }

    /// A painter the pinned checkout does not ship is an error naming the
    /// painter and the missing module — never a substitution, per the plan's
    /// "an unavailable painter errors; it never selects another at runtime".
    #[test]
    fn an_unavailable_painter_is_an_error_not_a_fallback() {
        smol::block_on(async {
            let (_machine, host) = machine_with_staged_host(Path::new("staged"), &["gpu"]);
            let (_temporary, project) = fixture_project(&host, "").await;

            let host_project_dir = require_painter_module(&project, HydrolysisAndroidPainter::Gpu)
                .await
                .expect("gpu module exists");
            assert_eq!(host_project_dir.file_name().unwrap(), "android");

            let error = require_painter_module(&project, HydrolysisAndroidPainter::Hwui)
                .await
                .expect_err("hwui module is absent")
                .to_string();
            assert!(error.contains("hwui"), "{error}");
            assert!(error.contains("never falls back"), "{error}");
        });
    }

    fn rendered_files(outputs: Vec<(PathBuf, Vec<u8>)>) -> BTreeMap<String, String> {
        outputs
            .into_iter()
            .filter_map(|(path, content)| {
                String::from_utf8(content)
                    .ok()
                    .map(|text| (path.to_string_lossy().into_owned(), text))
            })
            .collect()
    }

    /// The generated Gradle project composites the pinned host checkout:
    /// `includeBuild` + `dependencySubstitution` for `host` and the selected
    /// painter, the painter's API floor, its ABI profile filters, R8 keeps,
    /// manifest permissions, and the painter's band mounted in the activity.
    #[test]
    fn the_scaffold_composites_the_pinned_host_and_gpu_painter() {
        smol::block_on(async {
            let (_machine, host) = machine_with_staged_host(Path::new("staged"), &["gpu"]);
            let (_temporary, project) = fixture_project(
                &host,
                "\n[permissions]\ninternet = { enable = true, description = \"fixture\" }\n",
            )
            .await;
            let host_project_dir = require_painter_module(&project, HydrolysisAndroidPainter::Gpu)
                .await
                .expect("host project dir");

            let files = rendered_files(
                rendered_android_outputs(
                    &project,
                    HydrolysisAndroidPainter::Gpu,
                    &host_project_dir,
                )
                .await
                .expect("scaffold renders"),
            );

            let settings = files["settings.gradle.kts"].as_str();
            assert!(
                settings.contains("includeBuild(\"../android-host/"),
                "the host checkout is an included build: {settings}"
            );
            assert!(
                settings.contains("dev.waterui.hydrolysis:host"),
                "host substitution: {settings}"
            );
            assert!(
                settings.contains(
                    "substitute(module(\"dev.waterui.hydrolysis:gpu\")).using(project(\":gpu\"))"
                ),
                "gpu painter substitution: {settings}"
            );
            crate::assets::assert_settings_plugin_markers(settings);

            let gradle = files["app/build.gradle.kts"].as_str();
            assert!(gradle.contains("minSdk = 31"), "gpu api floor: {gradle}");
            crate::assets::assert_module_plugin_markers(gradle);
            assert!(
                gradle.contains("\"dev.waterui.hydrolysis:host\""),
                "{gradle}"
            );
            assert!(
                gradle.contains("\"dev.waterui.hydrolysis:gpu\""),
                "{gradle}"
            );
            assert!(gradle.contains("WATERUI_ANDROID_ABIS"), "{gradle}");
            assert!(gradle.contains("WATERUI_SKIP_RUST_BUILD"), "{gradle}");
            // The ABI profile: release filters to the ABI map so a packaged
            // app ships the requested architectures only.
            assert!(gradle.contains("\"arm64-v8a\" to \"arm64\""), "{gradle}");
            assert!(gradle.contains("\"x86_64\" to \"x86-64\""), "{gradle}");
            // Signing/shrinking on the release profile, kept narrow.
            assert!(gradle.contains("isMinifyEnabled = true"), "{gradle}");
            assert!(gradle.contains("isShrinkResources = true"), "{gradle}");
            assert!(gradle.contains("proguard-rules.pro"), "{gradle}");

            let manifest = files["app/src/main/AndroidManifest.xml"].as_str();
            assert!(
                manifest.contains("android:name=\"android.permission.INTERNET\""),
                "manifest permissions carry the manifest's enabled set: {manifest}"
            );
            assert!(
                manifest.contains("android:name=\".MainActivity\""),
                "{manifest}"
            );
            assert!(manifest.contains("android:exported=\"true\""), "{manifest}");
            crate::assets::assert_component_markers_inside_application(manifest);

            // A debug build prepares the inspector endpoint, so the debug
            // source set declares INTERNET whatever the project declares.
            let debug_manifest = files["app/src/debug/AndroidManifest.xml"].as_str();
            assert!(
                debug_manifest.contains("android:name=\"android.permission.INTERNET\""),
                "{debug_manifest}"
            );

            let activity = files["app/src/main/java/MainActivity.kt"].as_str();
            assert!(
                activity.contains("import dev.waterui.hydrolysis.gpu.HydrolysisGpuBand"),
                "gpu band mounts on the gpu painter: {activity}"
            );
            assert!(activity.contains("HydrolysisHostView"), "{activity}");

            // The environment reaches the app through `waterui.env.*` intent
            // extras applied by `Os.setenv`; nothing reads system properties.
            assert!(
                activity.contains("setupEnvironmentFromIntent(intent)"),
                "{activity}"
            );
            assert!(!activity.contains("SystemProperties"), "{activity}");

            // The JNI keep travels with the host library: its
            // consumer-rules.pro keeps every @CalledFromNative member, so
            // the app's own rules carry no per-method list.
            let proguard = files["app/proguard-rules.pro"].as_str();
            assert!(!proguard.contains("keepclassmembers"), "{proguard}");
            assert!(proguard.contains("CalledFromNative"), "{proguard}");

            // The 16 KiB page alignment the plan requires of every staged
            // native library is asserted by build(), which reuses
            // `require_aligned_shared_libraries` — the template level only
            // needs to not defeat it.
            assert!(files.contains_key("gradlew"), "gradle wrapper ships");
        });
    }

    /// The application's own graph selects the engine: a `webview` feature
    /// edge on the vendored `waterui` stub is the signal `uses_standard_webview`
    /// resolves through `cargo tree`.
    fn stage_webview_feature(temporary: &tempfile::TempDir, project: &Project) {
        let manifest_path = project.root().join("Cargo.toml");
        let mut document: toml_edit::DocumentMut = std::fs::read_to_string(&manifest_path)
            .expect("project Cargo.toml exists")
            .parse()
            .expect("project Cargo.toml parses");
        document["dependencies"]["waterui"]["path"] = toml_edit::value(
            temporary
                .path()
                .join("vendor/waterui")
                .to_string_lossy()
                .as_ref(),
        );
        document["dependencies"]["waterui"]["features"] =
            toml_edit::value(toml_edit::Array::from_iter(["webview"]));
        std::fs::write(&manifest_path, document.to_string()).expect("write Cargo.toml");
    }

    /// The Cargo feature and the Gradle module flag are one decision,
    /// `webview_backend_feature`: a project using the standard `WebView`
    /// compiles `webview-system` into the launcher's `hydrolysis` and marks
    /// the Gradle context so the scaffold substitutes the `:webview` module.
    /// Both assertions run through `HydrolysisBackend::template_context` —
    /// the context the launcher `Cargo.toml` actually renders from.
    #[test]
    fn the_launcher_manifest_enables_webview_system_for_a_webview_app() {
        smol::block_on(async {
            let (temporary, project) = fixture_project("").await;
            stage_webview_feature(&temporary, &project);

            let resolved = project
                .resolved_framework()
                .await
                .expect("the fixture's framework resolves");
            let ctx = HydrolysisBackend::template_context(&project, &resolved)
                .await
                .expect("template context builds");
            assert_eq!(ctx.webview_backend_feature(), Some("webview-system"));
            let outputs =
                crate::templates::hydrolysis::rendered_outputs(&ctx, "waterui-test-hydrolysis")
                    .expect("hydrolysis outputs render");
            let cargo_toml = outputs
                .iter()
                .find(|(path, _)| path == Path::new("Cargo.toml"))
                .map(|(_, content)| String::from_utf8_lossy(content).into_owned())
                .expect("a Cargo.toml output");
            // Only the Android target's hydrolysis edge counts: the desktop
            // section may carry the same feature name.
            let manifest = cargo_toml
                .parse::<toml::Table>()
                .expect("launcher Cargo.toml should parse");
            let features = manifest["target"]["cfg(target_os = \"android\")"]["dependencies"]
                ["hydrolysis"]["features"]
                .as_array()
                .expect("hydrolysis dependency features should be an array")
                .iter()
                .map(|feature| feature.as_str().expect("feature should be a string"))
                .collect::<Vec<_>>();
            assert!(
                features.contains(&"webview-system"),
                "android hydrolysis features: {features:?}"
            );

            let (_machine, host) =
                machine_with_staged_host(Path::new("staged"), &["gpu", "webview"]);
            let host_project_dir =
                require_painter_module(&host, &project, HydrolysisAndroidPainter::Gpu)
                    .await
                    .expect("host project dir");
            let ctx = android_template_context(
                &project,
                HydrolysisAndroidPainter::Gpu,
                &host_project_dir,
            )
            .await
            .expect("android template context builds");
            assert!(ctx.hydrolysis_android_has_system_webview());
        });
    }

    /// An app that enables the facade `webview` feature composites the pinned
    /// host's `webview/` module the same way it composites the painter:
    /// `dependencySubstitution` for the coordinate and the module on the app
    /// classpath. An app without it renders neither line.
    #[test]
    fn the_scaffold_composites_the_webview_module_for_a_webview_app() {
        smol::block_on(async {
            let (temporary, project) = fixture_project("").await;
            stage_webview_feature(&temporary, &project);

            let (_machine, host) =
                machine_with_staged_host(Path::new("staged"), &["gpu", "webview"]);
            let host_project_dir =
                require_painter_module(&host, &project, HydrolysisAndroidPainter::Gpu)
                    .await
                    .expect("host project dir");

            let files = rendered_files(
                rendered_android_outputs(
                    &project,
                    HydrolysisAndroidPainter::Gpu,
                    &host_project_dir,
                )
                .await
                .expect("scaffold renders"),
            );

            let settings = files["settings.gradle.kts"].as_str();
            assert!(
                settings.contains(
                    "substitute(module(\"dev.waterui.hydrolysis:webview\")).using(project(\":webview\"))"
                ),
                "webview module substitution: {settings}"
            );
            let gradle = files["app/build.gradle.kts"].as_str();
            assert!(
                gradle.contains("implementation(\"dev.waterui.hydrolysis:webview\")"),
                "webview module on the classpath: {gradle}"
            );

            // A staged host without the module is a scaffold-time error, not
            // a Gradle failure downstream. A fresh fixture keeps the pinned
            // checkout from the first half out of the way — it is keyed by
            // project and fake-git only overlays into it.
            let (temporary2, project2) = fixture_project("").await;
            stage_webview_feature(&temporary2, &project2);
            let (_machine2, host) = machine_with_staged_host(Path::new("staged"), &["gpu"]);
            let missing_dir =
                require_painter_module(&host, &project2, HydrolysisAndroidPainter::Gpu)
                    .await
                    .expect("host project dir");
            let error =
                rendered_android_outputs(&project2, HydrolysisAndroidPainter::Gpu, &missing_dir)
                    .await
                    .expect_err("a host without webview/ is an error");
            assert!(
                error.to_string().contains("webview"),
                "the error names the missing module: {error}"
            );
        });
    }

    /// The same project without the `webview` feature renders neither the
    /// substitution nor the dependency.
    #[test]
    fn a_non_webview_app_composites_no_webview_module() {
        smol::block_on(async {
            let (_temporary, project) = fixture_project("").await;
            let (_machine, host) = machine_with_staged_host(Path::new("staged"), &["gpu"]);
            let host_project_dir =
                require_painter_module(&host, &project, HydrolysisAndroidPainter::Gpu)
                    .await
                    .expect("host project dir");

            let files = rendered_files(
                rendered_android_outputs(
                    &project,
                    HydrolysisAndroidPainter::Gpu,
                    &host_project_dir,
                )
                .await
                .expect("scaffold renders"),
            );

            let settings = files["settings.gradle.kts"].as_str();
            assert!(
                !settings.contains("hydrolysis:webview"),
                "no webview substitution: {settings}"
            );
            let gradle = files["app/build.gradle.kts"].as_str();
            assert!(
                !gradle.contains("hydrolysis:webview"),
                "no webview dependency: {gradle}"
            );
        });
    }

    /// Release builds carry only the permissions the project declares: one
    /// declaring none gets no INTERNET in its main manifest, while its debug
    /// source set still declares it for the inspector endpoint.
    #[test]
    fn only_the_debug_source_set_adds_internet_to_an_undeclaring_project() {
        smol::block_on(async {
            let (_machine, host) = machine_with_staged_host(Path::new("staged"), &["gpu"]);
            let (_temporary, project) = fixture_project(&host, "").await;
            let host_project_dir = require_painter_module(&project, HydrolysisAndroidPainter::Gpu)
                .await
                .expect("host project dir");

            let files = rendered_files(
                rendered_android_outputs(
                    &project,
                    HydrolysisAndroidPainter::Gpu,
                    &host_project_dir,
                )
                .await
                .expect("scaffold renders"),
            );

            let manifest = files["app/src/main/AndroidManifest.xml"].as_str();
            assert!(
                !manifest.contains("android.permission.INTERNET"),
                "{manifest}"
            );
            let debug_manifest = files["app/src/debug/AndroidManifest.xml"].as_str();
            assert!(
                debug_manifest.contains("android:name=\"android.permission.INTERNET\""),
                "{debug_manifest}"
            );
        });
    }

    /// The generated Gradle script resolves `projectRoot` against the
    /// `android/` Gradle project dir, so the entry must carry the root
    /// relative to that dir — the launcher crate's backend-dir-relative path
    /// (`project_root_relative_path`) lands one `..` short and was the F2
    /// defect. Canonicalizing the joined path must land back on the project
    /// root, and the rendered script must use this value.
    #[test]
    fn the_scaffolded_project_root_resolves_against_the_android_dir() {
        smol::block_on(async {
            let (_temporary, project) = fixture_project(&Host::current(), "").await;
            let android_dir = android_dir(&project.backend_path::<HydrolysisBackend>());
            std::fs::create_dir_all(&android_dir).expect("android dir");
            let host_project_dir = project.root().join("android-host");

            let entry = template_entry(
                &project,
                HydrolysisAndroidPainter::Gpu,
                &host_project_dir,
                false,
            )
            .await
            .expect("template entry");
            let resolved = android_dir
                .join(&entry.project_root)
                .canonicalize()
                .expect("the scaffolded projectRoot resolves");
            assert_eq!(
                resolved,
                project.root().canonicalize().expect("project root"),
                "projectRoot must resolve to the application project root"
            );

            let files = rendered_files(
                rendered_android_outputs(
                    &project,
                    HydrolysisAndroidPainter::Gpu,
                    &host_project_dir,
                )
                .await
                .expect("scaffold renders"),
            );
            let gradle = files["app/build.gradle.kts"].as_str();
            assert!(
                gradle.contains(&format!("resolve(\"{}\")", entry.project_root)),
                "the rendered projectRoot uses the android-dir-relative path: {gradle}"
            );
        });
    }

    /// A cold managed-backend cache: opening with `ManagedBackends::NONE`
    /// leaves no ffi companion, and the Gradle packaging step reads its
    /// manifest for the classpath staging. The Android scaffold step must
    /// render it rather than rely on a prior native-Android open.
    #[test]
    fn the_android_scaffold_renders_the_ffi_companion_on_a_cold_cache() {
        smol::block_on(async {
            let (_machine, host) = machine_with_staged_host(Path::new("staged"), &["gpu"]);
            let (_temporary, project) = fixture_project(&host, "").await;
            assert!(
                !project.ffi_crate_path().join("Cargo.toml").exists(),
                "fixture opened with no managed backends: no companion scaffolded"
            );
            let host_project_dir = require_painter_module(&project, HydrolysisAndroidPainter::Gpu)
                .await
                .expect("host project dir");

            scaffold_android_project(&project, HydrolysisAndroidPainter::Gpu, &host_project_dir)
                .await
                .expect("android scaffold renders");

            assert!(
                project.ffi_crate_path().join("Cargo.toml").exists(),
                "the packaging path rendered the companion manifest it reads"
            );
        });
    }

    /// The hwui painter renders its module and dependency coordinates and
    /// ships no GPU band view. Its own API 29 requirement sits under the
    /// framework floor, so the generated `minSdk` is the framework's.
    #[test]
    fn the_hwui_painter_renders_at_the_framework_floor_and_mounts_no_band() {
        smol::block_on(async {
            let (_machine, host) = machine_with_staged_host(Path::new("staged"), &["hwui"]);
            let (_temporary, project) = fixture_project(&host, "").await;
            let host_project_dir = require_painter_module(&project, HydrolysisAndroidPainter::Hwui)
                .await
                .expect("host project dir");

            let files = rendered_files(
                rendered_android_outputs(
                    &project,
                    HydrolysisAndroidPainter::Hwui,
                    &host_project_dir,
                )
                .await
                .expect("scaffold renders"),
            );

            let settings = files["settings.gradle.kts"].as_str();
            assert!(
                settings.contains(
                    "substitute(module(\"dev.waterui.hydrolysis:hwui\")).using(project(\":hwui\"))"
                ),
                "hwui substitution: {settings}"
            );

            let gradle = files["app/build.gradle.kts"].as_str();
            assert!(
                gradle.contains("minSdk = 31"),
                "framework api floor: {gradle}"
            );
            assert!(
                gradle.contains("\"dev.waterui.hydrolysis:hwui\""),
                "{gradle}"
            );

            let activity = files["app/src/main/java/MainActivity.kt"].as_str();
            assert!(
                !activity.contains("HydrolysisGpuBand"),
                "no GPU band view exists in the hwui build: {activity}"
            );
        });
    }
}
