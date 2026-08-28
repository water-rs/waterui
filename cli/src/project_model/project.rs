//! Project management and build utilities for `WaterUI` CLI.

use cargo_toml::Manifest as CargoManifest;
use color_eyre::eyre;
use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use tracing::info;

use crate::build::RustLinkage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenMode {
    Full,
    PreviewBuild,
}

fn spawn_target_dir_resolution(
    current_dir: &Path,
) -> Shared<BoxFuture<'static, Result<PathBuf, String>>> {
    let current_dir = current_dir.to_path_buf();
    smol::spawn(async move {
        get_target_dir(&current_dir)
            .await
            .map_err(|error| error.to_string())
    })
    .boxed()
    .shared()
}

/// Represents a `WaterUI` project with its manifest and crate information.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    manifest: Manifest,
    crate_name: CrateName,
    target_dir_future: Shared<BoxFuture<'static, Result<PathBuf, String>>>,
    linked_packages: Arc<async_lock::OnceCell<Result<BTreeMap<String, String>, String>>>,
    managed_backends_root: PathBuf,
}

impl Project {
    /// Run the `WaterUI` project on the specified device.
    ///
    /// This method handles building, packaging, and running the project.
    ///
    /// # Arguments
    /// - `backend`: The backend to use for building and packaging
    /// - `platform`: The target platform to build for
    /// - `device`: The device to run on
    ///
    /// # Errors
    /// - If any step in the build, package, or run process fails.
    pub async fn run<B: Backend, D: Device>(
        &self,
        backend: &B,
        platform: TargetPlatform,
        device: D,
    ) -> Result<Running, FailToRun> {
        self.run_with_options(backend, platform, device, RunOptions::new())
            .await
    }

    /// Run the `WaterUI` project with explicit run options.
    ///
    /// This allows callers (like preview) to inject extra environment variables.
    ///
    /// # Errors
    /// Returns an error if building, packaging, or launching the app fails.
    pub async fn run_with_options<B: Backend, D: Device>(
        &self,
        backend: &B,
        platform: TargetPlatform,
        device: D,
        run_options: RunOptions,
    ) -> Result<Running, FailToRun> {
        // Build rust library for the target platform
        backend
            .build(self, platform, BuildOptions::development(false))
            .await
            .map_err(FailToRun::Build)?;

        // Package the build artifacts for the target platform
        let artifact = backend
            .package(self, platform, PackageOptions::development())
            .await
            .map_err(FailToRun::Package)?;

        Self::run_packaged(device, artifact, run_options).await
    }

    /// Run the Android backend for the specific target ABI of the device.
    ///
    /// This is required because Android packaging is ABI-dependent (e.g., `x86_64` emulator vs
    /// `arm64-v8a` physical device).
    ///
    /// # Errors
    /// Returns an error if building, packaging, or launching the Android app fails.
    pub async fn run_android_with_options<D: Device + AndroidAbiProvider>(
        &self,
        _backend: &AndroidBackend,
        device: D,
        run_options: RunOptions,
    ) -> Result<Running, FailToRun> {
        let abi = device.android_abi();

        self.browser_runtime_plan(TargetPlatform::Android, TargetBackend::Android)
            .await
            .map_err(FailToRun::Build)?;

        AndroidPlatform::clean_jni_libs(self)
            .await
            .map_err(FailToRun::Build)?;

        AndroidPlatform::new(abi)
            .build(self, BuildOptions::development(false))
            .await
            .map_err(FailToRun::Build)?;

        let artifact =
            AndroidPlatform::package_with_abis(self, PackageOptions::development(), &[abi])
                .await
                .map_err(FailToRun::Package)?;

        Self::run_packaged(device, artifact, run_options).await
    }

    async fn run_packaged<D: Device>(
        device: D,
        artifact: Artifact,
        run_options: RunOptions,
    ) -> Result<Running, FailToRun> {
        info!("Running on device");

        let running = device.run(artifact, run_options).await?;
        Ok(running)
    }

    /// Get the root path of the project.
    ///
    /// Same as the directory containing `Water.toml`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the target directory for Rust build artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot resolve the target directory.
    pub async fn target_dir(&self) -> eyre::Result<PathBuf> {
        self.target_dir_future
            .clone()
            .await
            .map_err(|error| eyre::eyre!(error))
    }

    /// Resolve the Cargo target directory every generated backend crate builds into.
    ///
    /// Generated backends need an explicit target directory, because the default one
    /// would land inside the managed build cache next to the generated sources — and
    /// those sources are deleted and regenerated whenever the CLI's scaffold templates
    /// change. Compiled artifacts do not become stale for that reason, so keeping them
    /// there meant one CLI upgrade discarded the compiled dependency graph of every
    /// project on the machine.
    ///
    /// One directory serves every backend, platform, and feature set of a linkage:
    /// Cargo already keys each compiled unit by target triple, resolved features, and
    /// profile, so switching backends only rebuilds the units the two graphs do not
    /// share — measured on an example app, over 80% of the Apple FFI graph resolves
    /// identically to the Hydrolysis graph and is reused as-is. Builds must therefore
    /// agree on everything Cargo hashes into every unit — pass an explicit `--target`
    /// and keep final-artifact link flags out of `RUSTFLAGS` (see
    /// `RustBuild::with_final_rustc_arg`) — or two variants sharing this directory
    /// re-fingerprint each other's entire dependency graph on every switch.
    ///
    /// Linkage is the one axis Cargo cannot separate: shared-runtime development
    /// builds carry `-Cprefer-dynamic -Crpath` in `RUSTFLAGS` and static packaging
    /// builds carry none, so each linkage keeps its own directory instead of the two
    /// variants invalidating each other whenever a developer alternates `water run`
    /// and `water package`.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot resolve the project target directory.
    pub async fn water_target_dir(&self, linkage: RustLinkage) -> eyre::Result<PathBuf> {
        let variant = match linkage {
            RustLinkage::SharedRuntime => "shared",
            RustLinkage::Static => "static",
        };
        Ok(self
            .target_dir()
            .await?
            .join("water-backends")
            .join(variant))
    }

    /// Resolve an isolated target directory for a backend built by a different Rust
    /// toolchain.
    ///
    /// Cargo hashes the compiler into every unit fingerprint, so a backend that pins
    /// its own toolchain (ESP32's Espressif Rust fork) would invalidate the host
    /// units of [`Self::water_target_dir`] on every switch if it shared the directory.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot resolve the project target directory.
    pub async fn toolchain_target_dir(&self, toolchain: &str) -> eyre::Result<PathBuf> {
        Ok(self
            .target_dir()
            .await?
            .join("water-backends")
            .join(toolchain))
    }

    /// Get the backends configured for the project.
    #[must_use]
    pub const fn backends(&self) -> &Backends {
        &self.manifest.backends
    }

    /// Get the crate name of the project.
    #[must_use]
    pub const fn crate_name(&self) -> &CrateName {
        &self.crate_name
    }

    /// Get configured or default FFI crate name for app mode.
    #[must_use]
    pub fn ffi_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.ffi.clone())
            .unwrap_or_else(|| self.crate_name.with_suffix("ffi"))
    }

    /// Get configured preview wrapper crate name for preview dylib builds.
    #[must_use]
    pub fn preview_ffi_crate_name(&self) -> CrateName {
        self.crate_name.with_suffix("preview-ffi")
    }

    /// Get the crate root path used to build preview dylibs.
    #[must_use]
    pub fn preview_dylib_crate_path(&self, workspace_root: &Path) -> PathBuf {
        self.preview_ffi_crate_path(workspace_root)
    }

    /// Get the crate name used to build preview dylibs.
    #[must_use]
    pub fn preview_dylib_crate_name(&self) -> CrateName {
        self.preview_ffi_crate_name()
    }

    /// Get configured or default GTK backend crate name for app mode.
    #[must_use]
    pub fn gtk_backend_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.gtk.clone())
            .unwrap_or_else(|| self.crate_name.with_suffix("gtk4"))
    }

    /// Get configured or default hydrolysis backend crate name for app mode.
    #[must_use]
    pub fn hydrolysis_backend_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.hydrolysis.clone())
            .unwrap_or_else(|| self.crate_name.with_suffix("hydrolysis"))
    }

    /// Get the generated ESP32 firmware harness crate name.
    #[must_use]
    pub fn esp32_backend_crate_name(&self) -> CrateName {
        self.crate_name.with_suffix("esp32")
    }

    /// Get package type declared in `Water.toml`.
    #[must_use]
    pub const fn package_type(&self) -> PackageType {
        self.manifest.package.package_type
    }

    /// Returns true when this project is a playground project.
    #[must_use]
    pub fn is_playground(&self) -> bool {
        self.package_type() == PackageType::Playground
    }

    /// Get the Apple backend configuration if available.
    #[must_use]
    pub const fn apple_backend(&self) -> Option<&AppleBackend> {
        self.manifest.backends.apple()
    }

    /// Get the full path to a backend directory.
    ///
    /// Returns `project.root() / backends.path / B::DEFAULT_PATH`.
    #[must_use]
    pub fn backend_path<B: Backend>(&self) -> PathBuf {
        self.managed_backends_root.join(B::DEFAULT_PATH)
    }

    /// Get the relative path to a backend directory from project root.
    ///
    /// Returns `backends.path / B::DEFAULT_PATH`.
    #[must_use]
    pub fn backend_relative_path<B: Backend>(&self) -> PathBuf {
        self.manifest.backends.path().join(B::DEFAULT_PATH)
    }

    /// Get the full path to the managed native FFI companion crate.
    #[must_use]
    pub fn ffi_crate_path(&self) -> PathBuf {
        self.managed_backends_root.join("ffi")
    }

    /// Directory name this project's preview module occupies inside a workspace.
    #[must_use]
    pub fn preview_module_member_path(&self) -> PathBuf {
        Path::new(crate::templates::PREVIEW_MODULES_DIR)
            .join(self.preview_ffi_crate_name().to_string())
    }

    /// Get the full path to the managed preview-only companion crate.
    ///
    /// The crate lives inside the support runtime's workspace rather than this
    /// project's build cache, because a preview module and the runtime it is
    /// loaded into must come out of one Cargo resolution to agree on the
    /// `-C metadata` hash mangled into every symbol.
    #[must_use]
    pub fn preview_ffi_crate_path(&self, workspace_root: &Path) -> PathBuf {
        workspace_root.join(self.preview_module_member_path())
    }

    /// Get the relative path to the managed native FFI companion crate from project root.
    #[must_use]
    pub fn ffi_crate_relative_path(&self) -> PathBuf {
        self.manifest.backends.path().join("ffi")
    }

    /// Get the Android backend configuration if available.
    #[must_use]
    pub const fn android_backend(&self) -> Option<&AndroidBackend> {
        self.manifest.backends.android()
    }

    /// Get the GTK4 backend configuration if available.
    #[must_use]
    pub const fn gtk4_backend(&self) -> Option<&crate::gtk4::backend::Gtk4Backend> {
        self.manifest.backends.gtk4()
    }

    /// Get the hydrolysis backend configuration if available.
    #[must_use]
    pub const fn hydrolysis_backend(
        &self,
    ) -> Option<&crate::hydrolysis::backend::HydrolysisBackend> {
        self.manifest.backends.hydrolysis()
    }

    /// Get the ESP32 backend configuration if available.
    #[must_use]
    pub const fn esp32_backend(&self) -> Option<&crate::esp32::backend::Esp32Backend> {
        self.manifest.backends.esp32()
    }

    /// Get the manifest of the project.
    #[must_use]
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Returns whether the packaged application links `package_name`.
    ///
    /// Development-only and build-only dependencies are excluded because they
    /// do not become part of the packaged application. The resolved graph is
    /// cached so backend regeneration and scaffolding share one Cargo metadata
    /// resolution.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo cannot resolve the application graph or
    /// omits a package referenced by that graph.
    pub async fn links_runtime_package(&self, package_name: &str) -> eyre::Result<bool> {
        let project_root = self.root.clone();
        let target_dir_future = self.target_dir_future.clone();
        let packages = self
            .linked_packages
            .get_or_init(|| async move {
                target_dir_future.await?;
                resolve_linked_runtime_packages(project_root)
                    .await
                    .map_err(|error| error.to_string())
            })
            .await;
        match packages {
            Ok(packages) => Ok(packages.contains_key(package_name)),
            Err(error) => Err(eyre::eyre!(error.clone())),
        }
    }

    /// Resolve and validate the standard `WebView` engine for a build.
    ///
    /// The application's own dependency graph is the selection: linking
    /// `waterui-browser-cef` or `waterui-browser-wpe` picks that engine, and an
    /// app that links neither uses whatever web engine the target platform
    /// bridges. Nothing in `Water.toml` names an engine, because nothing else
    /// could keep the packaged runtime and the code that loads it in step.
    ///
    /// Returns `None` when the application does not link `waterui-webview`, so
    /// an engine crate reaching the graph through some other component never
    /// adds a `WebView` runtime to the package on its own.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot be resolved, when the
    /// application links two engines at once, or when the selected engine is
    /// unsupported for the requested platform and backend.
    pub async fn resolved_webview_backend(
        &self,
        platform: TargetPlatform,
        backend: TargetBackend,
    ) -> eyre::Result<Option<ResolvedWebViewBackend>> {
        if !self.links_runtime_package("waterui-webview").await? {
            return Ok(None);
        }
        let engine = self.linked_browser_engine().await?;
        engine
            .unwrap_or(ResolvedWebViewBackend::System)
            .validate(platform, backend)
            .map(Some)
            .map_err(Into::into)
    }

    /// The browser engine crate the application links, if any.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot be resolved, or when the
    /// application links more than one engine — two engines cannot both draw
    /// one `WebView`, and the second `install` would fail at startup.
    pub async fn linked_browser_engine(&self) -> eyre::Result<Option<ResolvedWebViewBackend>> {
        let cef = self.links_runtime_package("waterui-browser-cef").await?;
        let wpe = self.links_runtime_package("waterui-browser-wpe").await?;
        match (cef, wpe) {
            (true, true) => eyre::bail!(
                "the application links both waterui-browser-cef and waterui-browser-wpe; \
                 exactly one browser engine can draw a WebView"
            ),
            (true, false) => Ok(Some(ResolvedWebViewBackend::Cef)),
            (false, true) => Ok(Some(ResolvedWebViewBackend::Wpe)),
            (false, false) => Ok(None),
        }
    }

    /// Resolves and validates every embedded browser runtime linked by the application.
    ///
    /// # Errors
    ///
    /// Returns an error when standard `WebView` or Chromium is unsupported for
    /// the requested platform and backend.
    pub async fn browser_runtime_plan(
        &self,
        platform: TargetPlatform,
        backend: TargetBackend,
    ) -> eyre::Result<BrowserRuntimePlan> {
        let webview = self.resolved_webview_backend(platform, backend).await?;
        let chromium = self.links_runtime_package("waterui-chromium").await?;
        if chromium && !cef_is_supported(platform, backend) {
            eyre::bail!(
                "waterui-chromium requires CEF, which is unsupported for platform {platform:?} \
                 with backend {backend:?}"
            );
        }
        Ok(BrowserRuntimePlan { webview, chromium })
    }

    /// Get the bundle identifier of the project.
    #[must_use]
    pub const fn bundle_identifier(&self) -> &BundleIdentifier {
        &self.manifest.package.bundle_identifier
    }

    /// Get the assets directory path relative to project root.
    #[must_use]
    pub fn assets_path(&self) -> &str {
        &self.manifest.package.assets_path
    }

    /// Get the full path to the assets directory.
    #[must_use]
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join(&self.manifest.package.assets_path)
    }

    /// Clean build artifacts for the project using the specified backend.
    ///
    /// # Errors
    ///
    /// Returns an error if cleaning fails.
    pub async fn clean<B: Backend>(
        &self,
        backend: &B,
        platform: TargetPlatform,
    ) -> Result<(), eyre::Report> {
        backend.clean(self, platform).await
    }

    /// Clean all build artifacts for the project.
    ///
    /// This cleans:
    /// - Rust target directory
    /// - Apple build artifacts (if backend configured)
    /// - Android build artifacts (if backend configured)
    /// - GTK4 build artifacts (if backend configured)
    ///
    /// # Errors
    ///
    /// Returns an error if any cleaning operation fails.
    pub async fn clean_all(&self) -> Result<(), eyre::Report> {
        use crate::{
            android::platform::clean_android, apple::platform::clean_apple,
            esp32::platform::clean_esp32, gtk4::platform::clean_gtk4,
            hydrolysis::platform::clean_hydrolysis,
        };

        if self.is_playground() {
            crate::water_dir::remove_project_build_cache(self.root()).await?;
            // A playground's Cargo target directory is the user's own (often a
            // workspace-wide one), so only the CLI-owned `water-backends` subtree
            // is removed — including target directories older CLI layouts left
            // behind — never the user's other compiled artifacts.
            let water_backends_root = self.target_dir().await?.join("water-backends");
            if water_backends_root.exists() {
                smol::fs::remove_dir_all(&water_backends_root).await?;
            }
            return Ok(());
        }

        // Clean Rust target directory
        let target_dir = self.target_dir().await?;
        if target_dir.exists() {
            smol::fs::remove_dir_all(&target_dir).await?;
        }

        // Clean Apple backend if configured
        if self.apple_backend().is_some() {
            clean_apple(self).await?;
        }

        // Clean Android backend if configured
        if self.android_backend().is_some() {
            clean_android(self).await?;
        }

        // Clean GTK4 backend if configured
        if self.gtk4_backend().is_some() || (self.is_playground() && cfg!(target_os = "linux")) {
            clean_gtk4(self).await?;
        }

        // Clean hydrolysis backend if configured
        if self.hydrolysis_backend().is_some() || self.is_playground() {
            clean_hydrolysis(self).await?;
        }

        // Clean ESP32 backend if configured
        if self.esp32_backend().is_some() {
            clean_esp32(self).await?;
        }

        let ffi_target_dir = self.ffi_crate_path().join("target");
        if ffi_target_dir.exists() {
            smol::fs::remove_dir_all(&ffi_target_dir).await?;
        }

        Ok(())
    }

    /// Package the project for the specified platform.
    ///
    /// # Errors
    ///
    /// Returns an error if packaging fails.
    pub async fn package<B: Backend>(
        &self,
        backend: &B,
        platform: TargetPlatform,
        options: PackageOptions,
    ) -> Result<Artifact, eyre::Report> {
        backend.package(self, platform, options).await
    }

    fn app_crate_overrides(&self) -> Option<&AppCrates> {
        self.manifest.app.as_ref()?.crates.as_ref()
    }
}

/// Errors that can occur when opening a `WaterUI` project.
#[derive(Debug, thiserror::Error)]
pub enum FailToOpenProject {
    /// Failed to open the Water.toml manifest.
    #[error("Failed to open project manifest: {0}")]
    Manifest(FailToOpenManifest),
    /// Failed to read the Cargo.toml file.
    #[error("Failed to read Cargo.toml: {0}")]
    CargoManifest(cargo_toml::Error),

    /// Failed to get Cargo metadata.
    #[error("Failed to get Cargo metadata: {0}")]
    TargetDirError(#[from] cargo_metadata::Error),

    /// Missing crate name in Cargo.toml.
    #[error("Invalid Cargo.toml: missing crate name")]
    MissingCrateName,

    /// Crate name in Cargo.toml is invalid.
    #[error("Invalid Cargo.toml crate name: {0}")]
    InvalidCrateName(String),

    /// Project permissions are not allowed in non-playground projects.
    #[error("Project permissions are not allowed in non-playground projects")]
    PermissionsNotAllowedInNonPlayground,

    /// Backend-project configuration is not allowed in playground manifests.
    #[error(
        "Backend project configuration is not allowed in playground projects \
         (device settings under [backends.esp32] are the exception)"
    )]
    BackendsNotAllowedInPlayground,

    /// Failed to initialize backend for playground project.
    #[error("Failed to initialize backend: {0}")]
    BackendInit(#[from] crate::backend::FailToInitBackend),

    /// Failed to manage the global build cache directory.
    #[error("Failed to prepare managed build cache: {0}")]
    BuildCache(#[from] eyre::Report),
}

/// Errors that can occur when creating a new `WaterUI` project.
#[derive(Debug, thiserror::Error)]
pub enum FailToCreateProject {
    /// The project directory already exists.
    #[error("Directory already exists: {0}")]
    DirectoryExists(PathBuf),
    /// Failed to create project directory.
    #[error("Failed to create directory: {0}")]
    CreateDir(std::io::Error),
    /// Failed to scaffold project files.
    #[error("Failed to scaffold project: {0}")]
    Scaffold(std::io::Error),
    /// Failed to save manifest.
    #[error("Failed to save manifest: {0}")]
    SaveManifest(#[from] FailToSaveManifest),

    /// Failed to get Cargo metadata.
    #[error("Failed to get Cargo metadata: {0}")]
    TargetDirError(#[from] cargo_metadata::Error),

    /// Failed to resolve the managed build cache path.
    #[error("Failed to resolve managed build cache: {0}")]
    BuildCache(#[from] eyre::Report),

    /// Failed to initialize git repository.
    #[error("Failed to initialize git repository: {0}")]
    GitInit(std::io::Error),
    /// Failed to check git repository status.
    #[error("Failed to check git repository status: {0}")]
    GitStatus(std::io::Error),
}

/// Options for creating a new `WaterUI` project.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    /// Application display name (e.g., "Water Example").
    pub name: String,
    /// Bundle identifier (e.g., "dev.waterui.waterexample").
    pub bundle_identifier: BundleIdentifier,
    /// Package type for the project.
    pub package_type: PackageType,
    /// Path to local `WaterUI` repository for development.
    pub waterui_path: Option<PathBuf>,
    /// Author name for Cargo.toml.
    pub author: String,
}

impl Project {
    async fn scaffold_ffi_companion(&self) -> Result<(), crate::backend::FailToInitBackend> {
        let manifest = self.manifest();
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let webview_enabled = self
            .links_runtime_package("waterui-webview")
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;
        let chromium_enabled = self
            .links_runtime_package("waterui-chromium")
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;
        let browser_engine = self
            .linked_browser_engine()
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;
        let ctx =
            TemplateContext::for_project_manifest(manifest, self.crate_name().clone(), app_name)
                .with_backend_project_path(self.ffi_crate_path())
                .with_project_root_path(self.root.clone())
                .with_webview_enabled(webview_enabled)
                .with_chromium_enabled(chromium_enabled)
                .with_browser_engine(browser_engine);

        templates::ffi::scaffold(&self.ffi_crate_path(), &ctx, &self.ffi_crate_name())
            .await
            .map_err(crate::backend::FailToInitBackend::Io)
    }

    /// Scaffold this project's preview module inside `workspace_root`.
    ///
    /// # Errors
    ///
    /// Returns an error when the generated crate cannot be written.
    pub async fn scaffold_preview_ffi_companion(
        &self,
        workspace_root: &Path,
    ) -> Result<PathBuf, crate::backend::FailToInitBackend> {
        let manifest = self.manifest();
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let ctx =
            TemplateContext::for_project_manifest(manifest, self.crate_name().clone(), app_name)
                .with_backend_project_path(self.preview_ffi_crate_path(workspace_root))
                .with_project_root_path(self.root.clone());

        let crate_path = self.preview_ffi_crate_path(workspace_root);
        templates::preview_ffi::scaffold(&crate_path, &ctx, &self.preview_ffi_crate_name())
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;
        Ok(crate_path)
    }

    async fn remove_ffi_companion_if_unused(&self) -> eyre::Result<()> {
        if self.apple_backend().is_some() || self.android_backend().is_some() {
            return Ok(());
        }

        let ffi_path = self.ffi_crate_path();
        if ffi_path.exists() {
            smol::fs::remove_dir_all(&ffi_path).await?;
        }

        Ok(())
    }

    /// Create a new `WaterUI` project at the specified path.
    ///
    /// This creates the project directory, scaffolds root files (Cargo.toml, src/lib.rs),
    /// and saves the Water.toml manifest. Use `init_apple_backend()` and `init_android_backend()`
    /// to scaffold platform backends after creation.
    ///
    /// # Errors
    /// - `FailToCreateProject::DirectoryExists`: If the directory already exists.
    /// - `FailToCreateProject::CreateDir`: If creating the directory fails.
    /// - `FailToCreateProject::Scaffold`: If scaffolding files fails.
    /// - `FailToCreateProject::SaveManifest`: If saving the manifest fails.
    pub async fn create(
        path: impl AsRef<Path>,
        options: CreateOptions,
    ) -> Result<Self, FailToCreateProject> {
        let path = path.as_ref().to_path_buf();

        // Check if directory already exists
        if path.exists() {
            return Err(FailToCreateProject::DirectoryExists(path));
        }

        // Create project directory
        smol::fs::create_dir_all(&path)
            .await
            .map_err(FailToCreateProject::CreateDir)?;

        // Derive crate name from display name
        let crate_name = CrateName::try_from(
            options
                .name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>(),
        )
        .map_err(|error| {
            FailToCreateProject::Scaffold(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                error,
            ))
        })?;

        // Build template context for root files
        let ctx = TemplateContext::for_create_options(&options, crate_name.clone());

        // The assets root is derived once and shared with both the scaffold and
        // the manifest, so the created directory and `Water.toml` cannot disagree.
        let assets_path = default_assets_path();

        // Scaffold root files (Cargo.toml, src/lib.rs, .gitignore, assets/README.md)
        templates::root::scaffold(&path, &ctx, &assets_path)
            .await
            .map_err(FailToCreateProject::Scaffold)?;

        // Build manifest
        let package_type = options.package_type;
        let mut backends = Backends::default();
        if package_type == PackageType::App {
            backends.set_path("backends");
        }

        let manifest = Manifest {
            package: Package {
                package_type,
                name: options.name.clone(),
                bundle_identifier: options.bundle_identifier.clone(),
                assets_path,
                accessory: false,
            },
            backends,
            waterui_path: options
                .waterui_path
                .as_ref()
                .map(|p| p.display().to_string()),
            permissions: BTreeMap::default(),
            app: None,
            theme: None,
        };

        // Save Water.toml
        manifest.save(&path).await?;

        // Initialize git repository if not already in one
        Self::ensure_git_init(&path).await?;

        let managed_backends_root = if options.package_type == PackageType::Playground {
            crate::water_dir::project_build_cache_dir(&path)
                .await
                .map_err(FailToCreateProject::BuildCache)?
        } else {
            path.join(manifest.backends.path())
        };

        let target_dir_future = spawn_target_dir_resolution(&path);
        Ok(Self {
            root: path,
            manifest,
            crate_name,
            target_dir_future,
            linked_packages: Arc::new(async_lock::OnceCell::new()),
            managed_backends_root,
        })
    }

    /// Ensure the project is initialized with git.
    ///
    /// Checks if the project directory is already part of a git repository.
    /// If not, initializes a new git repository.
    async fn ensure_git_init(path: &Path) -> Result<(), FailToCreateProject> {
        // Check if already in a git repository

        let mut cmd = Command::new("git");

        let is_in_git = command(&mut cmd)
            .args(["rev-parse", "--git-dir"])
            .current_dir(path)
            .output()
            .await
            .map_err(FailToCreateProject::GitStatus)?
            .status
            .success();

        if !is_in_git {
            // Initialize a new git repository
            let mut cmd = Command::new("git");
            command(&mut cmd)
                .args(["init"])
                .current_dir(path)
                .status()
                .await
                .map_err(FailToCreateProject::GitInit)?;
        }

        Ok(())
    }

    /// Initialize the Apple backend for this project.
    ///
    /// This scaffolds the Apple backend files and updates the manifest.
    ///
    /// # Errors
    /// Returns an error if scaffolding fails.
    pub async fn init_apple_backend(&mut self) -> Result<(), crate::backend::FailToInitBackend> {
        use crate::backend::Backend;

        let backend = AppleBackend::init(self).await?;
        self.scaffold_ffi_companion().await?;
        self.manifest.backends.set_apple(backend);
        self.manifest
            .save(&self.root)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))?;
        Ok(())
    }

    /// Initialize the Android backend for this project.
    ///
    /// This scaffolds the Android backend files and updates the manifest.
    ///
    /// # Errors
    /// Returns an error if scaffolding fails.
    pub async fn init_android_backend(&mut self) -> Result<(), crate::backend::FailToInitBackend> {
        use crate::backend::Backend;

        let backend = AndroidBackend::init(self).await?;
        self.scaffold_ffi_companion().await?;
        self.manifest.backends.set_android(backend);
        self.manifest
            .save(&self.root)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))?;
        Ok(())
    }

    /// Initialize the GTK4 backend for an existing project.
    ///
    /// Creates necessary files/folders for the GTK4 backend under `backend_path::<Gtk4Backend>()`.
    ///
    /// # Errors
    /// Returns an error if scaffolding fails.
    pub async fn init_gtk4_backend(&mut self) -> Result<(), crate::backend::FailToInitBackend> {
        use crate::{backend::Backend, gtk4::backend::Gtk4Backend};

        if !cfg!(target_os = "linux") {
            return Err(crate::backend::FailToInitBackend::Io(
                std::io::Error::other("GTK4 backend is only supported on Linux hosts"),
            ));
        }

        let backend = Gtk4Backend::init(self).await?;
        self.manifest.backends.set_gtk4(backend);
        self.manifest
            .save(&self.root)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))?;
        Ok(())
    }

    /// Initialize the hydrolysis backend for an existing project.
    ///
    /// Creates necessary files/folders for the hydrolysis backend under
    /// `backend_path::<HydrolysisBackend>()`.
    ///
    /// # Errors
    /// Returns an error if scaffolding fails.
    pub async fn init_hydrolysis_backend(
        &mut self,
    ) -> Result<(), crate::backend::FailToInitBackend> {
        use crate::{backend::Backend, hydrolysis::backend::HydrolysisBackend};

        let backend = HydrolysisBackend::init(self).await?;
        self.manifest.backends.set_hydrolysis(backend);
        self.manifest
            .save(&self.root)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))?;
        Ok(())
    }

    /// Initialize the ESP32 backend for an existing project.
    ///
    /// Creates necessary files/folders for the ESP32 firmware harness under
    /// `backend_path::<Esp32Backend>()`.
    ///
    /// # Errors
    /// Returns an error if scaffolding fails.
    pub async fn init_esp32_backend(&mut self) -> Result<(), crate::backend::FailToInitBackend> {
        use crate::{backend::Backend, esp32::backend::Esp32Backend};

        let backend = Esp32Backend::init(self).await?;
        self.manifest.backends.set_esp32(backend);
        self.manifest
            .save(&self.root)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))?;
        Ok(())
    }

    /// Select the ESP32 target chip, persisting it to `Water.toml`.
    ///
    /// The chip is the single source of truth for the ESP32 backend's target
    /// triple, QEMU model, and firmware parameters. Selecting a platform such
    /// as `esp32c3` calls this so the generated harness and build target follow
    /// the platform. No-ops (and skips the manifest write) when the configured
    /// chip already matches.
    ///
    /// # Errors
    /// Returns an error if saving the manifest fails.
    pub async fn set_esp32_chip(
        &mut self,
        chip: crate::esp32::chip::Esp32Chip,
    ) -> eyre::Result<()> {
        let current = self.esp32_backend().cloned().unwrap_or_default();
        if current.chip() == chip.id() {
            return Ok(());
        }
        self.manifest.backends.set_esp32(current.with_chip(chip));
        self.save_manifest().await
    }

    /// Remove Apple backend configuration and generated files.
    ///
    /// # Errors
    /// Returns an error if deleting files or saving manifest fails.
    pub async fn remove_apple_backend(&mut self) -> eyre::Result<()> {
        if let Some(backend) = self.apple_backend() {
            let path = backend.project_path().to_path_buf();
            self.remove_backend_relative_dir(&path).await?;
        }
        self.manifest.backends.clear_apple();
        self.remove_ffi_companion_if_unused().await?;
        self.save_manifest().await
    }

    /// Remove Android backend configuration and generated files.
    ///
    /// # Errors
    /// Returns an error if deleting files or saving manifest fails.
    pub async fn remove_android_backend(&mut self) -> eyre::Result<()> {
        if let Some(backend) = self.android_backend() {
            let path = backend.project_path().clone();
            self.remove_backend_relative_dir(&path).await?;
        }
        self.manifest.backends.clear_android();
        self.remove_ffi_companion_if_unused().await?;
        self.save_manifest().await
    }

    /// Remove GTK4 backend configuration and generated files.
    ///
    /// # Errors
    /// Returns an error if deleting files or saving manifest fails.
    pub async fn remove_gtk4_backend(&mut self) -> eyre::Result<()> {
        if let Some(backend) = self.gtk4_backend() {
            let path = backend.project_path().clone();
            self.remove_backend_relative_dir(&path).await?;
        }
        self.manifest.backends.clear_gtk4();
        self.save_manifest().await
    }

    /// Remove hydrolysis backend configuration and generated files.
    ///
    /// # Errors
    /// Returns an error if deleting files or saving manifest fails.
    pub async fn remove_hydrolysis_backend(&mut self) -> eyre::Result<()> {
        if let Some(backend) = self.hydrolysis_backend() {
            let path = backend.project_path().clone();
            self.remove_backend_relative_dir(&path).await?;
        }
        self.manifest.backends.clear_hydrolysis();
        self.save_manifest().await
    }

    /// Remove ESP32 backend configuration and generated files.
    ///
    /// # Errors
    /// Returns an error if deleting files or saving manifest fails.
    pub async fn remove_esp32_backend(&mut self) -> eyre::Result<()> {
        if let Some(backend) = self.esp32_backend() {
            let path = backend.project_path().clone();
            self.remove_backend_relative_dir(&path).await?;
        }
        self.manifest.backends.clear_esp32();
        self.save_manifest().await
    }

    /// Open a `WaterUI` project located at the specified path.
    ///
    /// This loads both the `Water.toml` manifest and the `Cargo.toml` file.
    /// For playground projects, backends are automatically initialized if not configured.
    ///
    /// # Errors
    /// - `FailToOpenProject::Manifest`: If there was an error opening the `Water.toml` manifest.
    /// - `FailToOpenProject::CargoManifest`: If there was an error reading the `Cargo.toml` file.
    /// - `FailToOpenProject::MissingCrateName`: If the crate name is missing in `Cargo.toml`.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, FailToOpenProject> {
        Self::open_with_mode(path, OpenMode::Full).await
    }

    /// Open a project for preview dylib builds without initializing native app backends.
    ///
    /// Playground preview dylib builds only need the managed preview wrapper crate. Native
    /// backend initialization is reserved for support app projects that actually launch apps.
    ///
    /// # Errors
    /// - `FailToOpenProject::Manifest`: If there was an error opening the `Water.toml` manifest.
    /// - `FailToOpenProject::CargoManifest`: If there was an error reading the `Cargo.toml` file.
    /// - `FailToOpenProject::MissingCrateName`: If the crate name is missing in `Cargo.toml`.
    pub async fn open_for_preview_build(path: impl AsRef<Path>) -> Result<Self, FailToOpenProject> {
        Self::open_with_mode(path, OpenMode::PreviewBuild).await
    }

    #[allow(clippy::too_many_lines)]
    async fn open_with_mode(
        path: impl AsRef<Path>,
        open_mode: OpenMode,
    ) -> Result<Self, FailToOpenProject> {
        use crate::backend::Backend;

        let total_start = std::time::Instant::now();
        let path = path.as_ref().to_path_buf();

        let manifest_start = std::time::Instant::now();
        let manifest = Manifest::open(path.join("Water.toml"))
            .await
            .map_err(FailToOpenProject::Manifest)?;
        info!(
            path = %path.display(),
            open_mode = ?open_mode,
            elapsed_ms = manifest_start.elapsed().as_millis(),
            "Project::open loaded Water.toml"
        );

        let cargo_path = path.join("Cargo.toml");

        let cargo_manifest_start = std::time::Instant::now();
        let cargo_manifest = unblock(move || CargoManifest::from_path(cargo_path))
            .await
            .map_err(FailToOpenProject::CargoManifest)?;
        info!(
            path = %path.display(),
            open_mode = ?open_mode,
            elapsed_ms = cargo_manifest_start.elapsed().as_millis(),
            "Project::open loaded Cargo.toml"
        );
        let crate_name = cargo_manifest
            .package
            .map(|p| p.name)
            .ok_or(FailToOpenProject::MissingCrateName)
            .and_then(|value| {
                CrateName::try_from(value).map_err(FailToOpenProject::InvalidCrateName)
            })?;

        let is_playground = manifest.package.package_type == PackageType::Playground;

        // Check that permissions are only set for playground projects
        if !is_playground && !manifest.permissions.is_empty() {
            return Err(FailToOpenProject::PermissionsNotAllowedInNonPlayground);
        }

        // Playgrounds delegate backend projects to the CLI, so backend
        // scaffolding configuration is rejected. `[backends.esp32]` is the
        // exception: it is device configuration (chip, panel geometry,
        // bundled fonts) only the app author can supply, and its harness
        // still lives in the managed build cache.
        if is_playground && manifest.backends.configures_backend_projects() {
            return Err(FailToOpenProject::BackendsNotAllowedInPlayground);
        }

        let managed_backends_root = if is_playground {
            let build_cache_start = std::time::Instant::now();
            let root = crate::water_dir::ensure_project_build_cache(&path)
                .await
                .map_err(FailToOpenProject::BuildCache)?;
            info!(
                path = %path.display(),
                open_mode = ?open_mode,
                elapsed_ms = build_cache_start.elapsed().as_millis(),
                "Project::open ensured project build cache"
            );
            root
        } else {
            path.join(manifest.backends.path())
        };

        let target_dir_future = spawn_target_dir_resolution(&path);
        let mut project = Self {
            root: path,
            manifest,
            crate_name,
            target_dir_future,
            linked_packages: Arc::new(async_lock::OnceCell::new()),
            managed_backends_root,
        };

        // For playground projects, auto-initialize backends
        // Always re-scaffold templates on each run to pick up manifest changes (e.g., permissions)
        // Build cache (build/, .gradle/, DerivedData/) is preserved since scaffold only writes template files
        //
        // Skip backend initialization when:
        // 1. Running inside Xcode's sandboxed build script phase (WATERUI_SKIP_RUST_BUILD=1)
        // 2. Running inside any sandbox (sandbox-exec sets __XCODE_BUILT_PRODUCTS_DIR_PATHS or similar)
        // 3. Xcode is the current build tool (ACTION env var is set by Xcode)
        let skip_backend_init = std::env::var("WATERUI_SKIP_RUST_BUILD")
            .is_ok_and(|value| value == "1")
            || std::env::var("ACTION").is_ok() // Xcode sets this during builds
            || std::env::var("XCODE_PRODUCT_BUILD_VERSION").is_ok();

        if is_playground && !skip_backend_init && open_mode == OpenMode::Full {
            let apple_backend_start = std::time::Instant::now();
            let apple_backend = AppleBackend::init(&project)
                .await
                .map_err(FailToOpenProject::BackendInit)?;
            info!(
                path = %project.root.display(),
                elapsed_ms = apple_backend_start.elapsed().as_millis(),
                "Project::open initialized Apple backend"
            );
            project.manifest.backends.set_apple(apple_backend);

            let android_backend_start = std::time::Instant::now();
            let android_backend = AndroidBackend::init(&project)
                .await
                .map_err(FailToOpenProject::BackendInit)?;
            info!(
                path = %project.root.display(),
                elapsed_ms = android_backend_start.elapsed().as_millis(),
                "Project::open initialized Android backend"
            );
            project.manifest.backends.set_android(android_backend);

            let ffi_companion_start = std::time::Instant::now();
            project
                .scaffold_ffi_companion()
                .await
                .map_err(FailToOpenProject::BackendInit)?;
            info!(
                path = %project.root.display(),
                elapsed_ms = ffi_companion_start.elapsed().as_millis(),
                "Project::open scaffolded native ffi companion"
            );
        }

        if !is_playground
            && !skip_backend_init
            && open_mode == OpenMode::Full
            && (project.apple_backend().is_some() || project.android_backend().is_some())
        {
            let ffi_companion_start = std::time::Instant::now();
            project
                .scaffold_ffi_companion()
                .await
                .map_err(FailToOpenProject::BackendInit)?;
            info!(
                path = %project.root.display(),
                elapsed_ms = ffi_companion_start.elapsed().as_millis(),
                "Project::open refreshed native ffi companion"
            );
        }

        info!(
            path = %project.root.display(),
            open_mode = ?open_mode,
            elapsed_ms = total_start.elapsed().as_millis(),
            "Project::open completed"
        );

        Ok(project)
    }
}

impl Project {
    async fn save_manifest(&self) -> eyre::Result<()> {
        self.manifest.save(&self.root).await.map_err(Into::into)
    }

    async fn remove_backend_relative_dir(&self, relative_path: &Path) -> eyre::Result<()> {
        let backend_path = self.managed_backends_root.join(relative_path);
        if backend_path.exists() {
            smol::fs::remove_dir_all(&backend_path).await?;
        }
        Ok(())
    }
}

async fn get_target_dir(current_dir: &Path) -> Result<PathBuf, cargo_metadata::Error> {
    let current_dir = current_dir.to_path_buf();
    let metadata = unblock(|| {
        cargo_metadata::MetadataCommand::new()
            .no_deps()
            .current_dir(current_dir)
            .exec()
    })
    .await?;

    let target_dir = metadata.target_directory.as_std_path();

    Ok(target_dir.to_path_buf())
}

async fn resolve_linked_runtime_packages(
    project_root: PathBuf,
) -> eyre::Result<BTreeMap<String, String>> {
    let manifest_path = project_root.join("Cargo.toml");
    let metadata_manifest = manifest_path.clone();
    let metadata = unblock(move || {
        cargo_metadata::MetadataCommand::new()
            .no_deps()
            .manifest_path(metadata_manifest)
            .exec()
    })
    .await?;
    // `dunce`, not `std::fs::canonicalize`: on Windows the standard one returns
    // an extended-length path (`\\?\D:\...`), while `cargo metadata` reports the
    // plain one, so comparing the two never matched and the package below was
    // always "omitted" (part of #152). Everywhere else this is `canonicalize`.
    let application_manifest = dunce::canonicalize(&manifest_path)?;
    let root = metadata
        .packages
        .iter()
        .find(|package| package.manifest_path.as_std_path() == application_manifest)
        .ok_or_else(|| {
            eyre::eyre!(
                "Cargo metadata omitted the application package at {}",
                application_manifest.display()
            )
        })?;
    let package_spec = root.id.to_string();
    let output = Command::new("cargo")
        .arg("tree")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--package")
        .arg(package_spec)
        .arg("--edges")
        .arg("normal")
        .arg("--prefix")
        .arg("none")
        .arg("--format")
        .arg("{p}")
        .current_dir(&project_root)
        .output()
        .await?;
    if !output.status.success() {
        return Err(eyre::eyre!(
            "failed to resolve runtime dependency graph for {}: {}",
            manifest_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let mut linked = BTreeMap::new();
    let tree = String::from_utf8(output.stdout)
        .map_err(|error| eyre::eyre!("Cargo runtime dependency graph is not UTF-8: {error}"))?;
    for package in tree.lines() {
        let name = package
            .split_ascii_whitespace()
            .next()
            .ok_or_else(|| eyre::eyre!("Cargo emitted an empty runtime dependency entry"))?;
        linked.insert(name.to_string(), package.to_string());
    }

    Ok(linked)
}

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use smol::{fs::read_to_string, process::Command, unblock};
use waterui_assets_planner::ThemeConfig;

use crate::{
    android::{backend::AndroidBackend, device::AndroidAbiProvider, platform::AndroidPlatform},
    apple::backend::AppleBackend,
    backend::{Backend, Backends},
    build::BuildOptions,
    device::{Artifact, Device, FailToRun, RunOptions, Running},
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project_types::{BundleIdentifier, CrateName, PermissionKey},
    templates::{self, TemplateContext},
    utils::command,
};

/// Configuration for a `WaterUI` project persisted to `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Manifest {
    /// Package information.
    pub package: Package,
    /// Backend configurations for various platforms.
    #[serde(default, skip_serializing_if = "Backends::is_empty")]
    pub backends: Backends,
    /// Web engine selected for the standard `WebView` component.
    /// Path to local `WaterUI` repository for dev mode.
    /// When set, all backends will use this path instead of the published versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waterui_path: Option<String>,
    /// Permission configuration for playground projects.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<PermissionKey, PermissionEntry>,
    /// App-only configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppConfig>,
    /// Cross-platform app theme slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeConfig>,
}

/// Permission entry for playground projects.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PermissionEntry {
    enable: bool,
    /// Explain why this permission is needed.
    description: String,
}

impl PermissionEntry {
    /// Check if this permission is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enable
    }

    /// Get the description of why this permission is needed.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Errors that can occur when opening a `Water.toml` manifest file.
#[derive(Debug, thiserror::Error)]
pub enum FailToOpenManifest {
    /// Failed to read the manifest file from the filesystem.
    #[error("Failed to read manifest file: {0}")]
    ReadError(std::io::Error),
    /// The manifest file is invalid or malformed.
    #[error("Invalid manifest file: {0}")]
    InvalidManifest(toml::de::Error),

    /// The manifest file was not found at the specified path.
    #[error("Manifest file not found at the specified path")]
    NotFound,
}

/// Errors that can occur when saving a `Water.toml` manifest file.
#[derive(Debug, thiserror::Error)]
pub enum FailToSaveManifest {
    /// Failed to serialize the manifest to TOML.
    #[error("Failed to serialize manifest: {0}")]
    Serialize(toml::ser::Error),
    /// Failed to write the manifest file to disk.
    #[error("Failed to write manifest file: {0}")]
    Write(std::io::Error),
}
impl Manifest {
    /// Open and parse a `Water.toml` manifest file from the specified path.
    ///
    /// # Errors
    /// - `FailToOpenManifest::ReadError`: If there was an error reading the file.
    /// - `FailToOpenManifest::InvalidManifest`: If the file contents are not valid TOML.
    /// - `FailToOpenManifest::NotFound`: If the file does not exist at the specified path.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, FailToOpenManifest> {
        let path = path.as_ref();
        let result = read_to_string(path).await;

        match result {
            Ok(c) => toml::from_str(&c).map_err(FailToOpenManifest::InvalidManifest),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(FailToOpenManifest::NotFound),
            Err(e) => Err(FailToOpenManifest::ReadError(e)),
        }
    }

    /// Save the manifest to a `Water.toml` file at the specified directory.
    ///
    /// # Errors
    /// - If there was an error serializing the manifest to TOML.
    /// - If there was an error writing the file.
    pub async fn save(&self, dir: impl AsRef<Path>) -> Result<(), FailToSaveManifest> {
        let path = dir.as_ref().join("Water.toml");
        let content = toml::to_string_pretty(self).map_err(FailToSaveManifest::Serialize)?;
        smol::fs::write(&path, content)
            .await
            .map_err(FailToSaveManifest::Write)
    }

    /// Create a new `Manifest` with the specified package information.
    #[must_use]
    pub fn new(package: Package) -> Self {
        Self {
            package,
            backends: Backends::default(),
            waterui_path: None,
            permissions: BTreeMap::default(),
            app: None,
            theme: None,
        }
    }
}

/// The engine that draws this application's standard `WebView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedWebViewBackend {
    /// Platform-provided `WebView`.
    System,
    /// Bundled WPE `WebKit` runtime.
    Wpe,
    /// Bundled Chromium Embedded Framework runtime.
    Cef,
}

/// Browser engines that must be staged for one resolved application graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserRuntimePlan {
    /// Standard `WebView` engine, when `waterui-webview` is linked.
    pub webview: Option<ResolvedWebViewBackend>,
    /// Whether the independent full Chromium component is linked.
    pub chromium: bool,
}

impl BrowserRuntimePlan {
    /// Returns whether this application requires a packaged CEF runtime and
    /// subprocess helper.
    #[must_use]
    pub const fn requires_cef(self) -> bool {
        self.chromium || matches!(self.webview, Some(ResolvedWebViewBackend::Cef))
    }
}

impl ResolvedWebViewBackend {
    /// Return whether this engine can be hosted by a platform and backend pair.
    #[must_use]
    pub const fn supports(self, platform: TargetPlatform, backend: TargetBackend) -> bool {
        match self {
            Self::System => matches!(
                (platform, backend),
                (
                    TargetPlatform::MacOS,
                    TargetBackend::Apple | TargetBackend::Hydrolysis
                ) | (
                    TargetPlatform::IOS
                        | TargetPlatform::IOSSimulator
                        | TargetPlatform::VisionOS
                        | TargetPlatform::VisionOSSimulator,
                    TargetBackend::Apple
                ) | (TargetPlatform::Android, TargetBackend::Android)
                    | (TargetPlatform::Linux, TargetBackend::Gtk4)
                    | (TargetPlatform::Web, TargetBackend::Hydrolysis)
            ),
            Self::Wpe => {
                matches!(platform, TargetPlatform::Linux)
                    && matches!(backend, TargetBackend::Gtk4 | TargetBackend::Hydrolysis)
            }
            Self::Cef => cef_is_supported(platform, backend),
        }
    }

    /// Returns this engine, or an error naming what cannot host it.
    ///
    /// # Errors
    ///
    /// Returns an error when this platform and backend pair cannot host the
    /// engine the application selected.
    pub const fn validate(
        self,
        platform: TargetPlatform,
        backend: TargetBackend,
    ) -> Result<Self, UnsupportedWebViewBackend> {
        if self.supports(platform, backend) {
            Ok(self)
        } else {
            Err(UnsupportedWebViewBackend {
                resolved: self,
                platform,
                backend,
            })
        }
    }

    /// Stable lowercase name used for Cargo features, runtime manifests, and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Wpe => "wpe",
            Self::Cef => "cef",
        }
    }
}

const fn cef_is_supported(platform: TargetPlatform, backend: TargetBackend) -> bool {
    !matches!(backend, TargetBackend::Dew)
        && matches!(
            platform,
            TargetPlatform::MacOS | TargetPlatform::Linux | TargetPlatform::Windows
        )
}

/// Error returned for an unsupported `WebView` engine/platform/backend combination.
#[derive(Debug, thiserror::Error)]
#[error(
    "this application's WebView engine resolves to {resolved:?}, which is unsupported for \
     platform {platform:?} with backend {backend:?}. The engine follows the application's \
     dependencies: link waterui-browser-cef or waterui-browser-wpe to select one, or \
     neither to use the engine this platform bridges."
)]
pub struct UnsupportedWebViewBackend {
    resolved: ResolvedWebViewBackend,
    platform: TargetPlatform,
    backend: TargetBackend,
}

/// App-specific configuration in `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    /// Optional crate name overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crates: Option<AppCrates>,
}

/// Crate name overrides for app mode.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AppCrates {
    /// Optional override crate name for generated FFI crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffi: Option<CrateName>,
    /// Optional override crate name for generated GTK backend crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gtk: Option<CrateName>,
    /// Optional override crate name for generated hydrolysis backend crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hydrolysis: Option<CrateName>,
}

/// `[package]` section in `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    /// Type of the package (e.g., "app").
    #[serde(rename = "type")]
    pub package_type: PackageType,
    /// Human-readable name of the application (e.g., "Water Demo").
    pub name: String,
    /// Bundle identifier for the application (e.g., "dev.waterui.waterdemo").
    pub bundle_identifier: BundleIdentifier,
    /// Path to assets directory relative to project root. Defaults to "assets".
    #[serde(
        default = "default_assets_path",
        skip_serializing_if = "is_default_assets_path"
    )]
    pub assets_path: String,
    /// Whether to build as an accessory (headless) app on macOS.
    #[serde(default, skip_serializing_if = "is_false")]
    pub accessory: bool,
}

fn default_assets_path() -> String {
    "assets".to_string()
}

fn is_default_assets_path(path: &str) -> bool {
    path == "assets"
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// Package type indicating what kind of project this is.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    /// A standalone application with platform-specific backends.
    #[default]
    App,
    /// A playground project for quick experimentation.
    /// Platform projects are created in a temporary directory.
    Playground,
}

#[cfg(test)]
mod webview_backend_tests {
    use std::path::Path;

    use super::{
        ResolvedWebViewBackend, TargetBackend, TargetPlatform, resolve_linked_runtime_packages,
    };

    /// An application that links no engine crate uses whatever the platform
    /// bridges, and the bridge is not everywhere: Linux outside GTK has none, so
    /// such a build is refused with an explanation instead of producing a
    /// contentless web view at runtime.
    #[test]
    fn the_platform_bridge_is_the_selection_without_an_engine_crate() {
        assert_eq!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::MacOS, TargetBackend::Hydrolysis)
                .expect("macOS Hydrolysis bridges WKWebView"),
            ResolvedWebViewBackend::System
        );
        assert_eq!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::Linux, TargetBackend::Gtk4)
                .expect("GTK bridges WebKitGTK"),
            ResolvedWebViewBackend::System
        );
        assert!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::Linux, TargetBackend::Hydrolysis)
                .is_err()
        );
        assert!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::Windows, TargetBackend::Hydrolysis)
                .is_err()
        );
    }

    #[test]
    fn unsupported_engine_combinations_fail_before_build() {
        assert!(
            ResolvedWebViewBackend::Wpe
                .validate(TargetPlatform::MacOS, TargetBackend::Hydrolysis)
                .is_err()
        );
        assert!(
            ResolvedWebViewBackend::Cef
                .validate(TargetPlatform::Android, TargetBackend::Android)
                .is_err()
        );
        assert_eq!(
            ResolvedWebViewBackend::Cef
                .validate(TargetPlatform::MacOS, TargetBackend::Apple)
                .expect("CEF must compose with the native Apple renderer on macOS"),
            ResolvedWebViewBackend::Cef
        );
    }

    #[test]
    fn cef_is_available_to_every_non_dew_backend_on_desktop_platforms() {
        for backend in [
            TargetBackend::Apple,
            TargetBackend::Android,
            TargetBackend::Gtk4,
            TargetBackend::Hydrolysis,
        ] {
            for platform in [
                TargetPlatform::MacOS,
                TargetPlatform::Linux,
                TargetPlatform::Windows,
            ] {
                assert_eq!(
                    ResolvedWebViewBackend::Cef
                        .validate(platform, backend)
                        .expect("CEF availability must not depend on the WaterUI backend"),
                    ResolvedWebViewBackend::Cef
                );
            }
        }
    }

    #[test]
    fn cef_rejects_dew_and_platforms_without_cef_distributions() {
        for platform in [
            TargetPlatform::MacOS,
            TargetPlatform::Linux,
            TargetPlatform::Windows,
        ] {
            assert!(
                ResolvedWebViewBackend::Cef
                    .validate(platform, TargetBackend::Dew)
                    .is_err()
            );
        }
        for (platform, backend) in [
            (TargetPlatform::Android, TargetBackend::Android),
            (TargetPlatform::IOS, TargetBackend::Apple),
            (TargetPlatform::Web, TargetBackend::Hydrolysis),
        ] {
            assert!(
                ResolvedWebViewBackend::Cef
                    .validate(platform, backend)
                    .is_err()
            );
        }
    }

    /// The engine is read out of the application's own graph, so the examples
    /// are the test: the CEF `WebView` example links `waterui-browser-cef` and
    /// the shared system-`WebView` example links no engine at all.
    #[test]
    fn runtime_graph_is_scoped_to_the_selected_application() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CLI crate must be inside the WaterUI repository");
        let chromium = smol::block_on(resolve_linked_runtime_packages(
            repository.join("examples/chromium"),
        ))
        .expect("Chromium example runtime graph must resolve");
        assert!(
            chromium.contains_key("waterui-chromium"),
            "Chromium example graph: {chromium:#?}"
        );
        // The Chromium example links the engine it draws through, and nothing
        // else: no second engine, and no `waterui` facade `webview` feature.
        assert!(
            chromium.contains_key("waterui-browser-cef"),
            "Chromium example graph: {chromium:#?}"
        );
        assert!(
            !chromium.contains_key("waterui-browser-wpe"),
            "Chromium example graph: {chromium:#?}"
        );

        let webview = smol::block_on(resolve_linked_runtime_packages(
            repository.join("examples/webview"),
        ))
        .expect("WebView example runtime graph must resolve");
        assert!(
            webview.contains_key("waterui-webview"),
            "WebView example graph: {webview:#?}"
        );
        assert!(
            !webview.contains_key("waterui-browser-cef"),
            "WebView example graph: {webview:#?}"
        );
        assert!(
            !webview.contains_key("waterui-chromium"),
            "WebView example graph: {webview:#?}"
        );

        let cef_webview = smol::block_on(resolve_linked_runtime_packages(
            repository.join("examples/webview-cef"),
        ))
        .expect("CEF WebView example runtime graph must resolve");
        assert!(
            cef_webview.contains_key("waterui-browser-cef"),
            "CEF WebView example graph: {cef_webview:#?}"
        );
        assert!(
            !cef_webview.contains_key("waterui-browser-wpe"),
            "CEF WebView example graph: {cef_webview:#?}"
        );
    }
}

#[cfg(test)]
mod scaffold_tests {
    use super::{BundleIdentifier, CreateOptions, PackageType, Project};

    /// The documented `assets!` workflow requires the assets root to exist: the
    /// planner walks it recursively, so a missing directory fails the first
    /// `assets!` call. `water create` must therefore produce it, tracked, and at
    /// exactly the path the generated `Water.toml` declares.
    #[test]
    fn create_scaffolds_the_assets_directory_declared_by_the_manifest() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("water-example");

        let project = smol::block_on(Project::create(
            &root,
            CreateOptions {
                name: "Water Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                    .expect("bundle identifier"),
                package_type: PackageType::Playground,
                waterui_path: None,
                author: "Lexo Liu".to_string(),
            },
        ))
        .expect("project creation must succeed");

        let assets = project.assets_dir();
        assert!(
            assets.is_dir(),
            "the assets root {} must exist after `water create`",
            assets.display()
        );
        assert_eq!(
            assets,
            root.join(project.assets_path()),
            "the scaffolded directory must be the one the manifest declares"
        );
        assert!(
            assets.join("README.md").is_file(),
            "a tracked file keeps the assets directory present in git"
        );
    }
}
