//! Project management and build utilities for `WaterUI` CLI.

use cargo_toml::Manifest as CargoManifest;
use color_eyre::eyre;
use sha2::Digest as _;
use tracing::info;

/// Represents a `WaterUI` project with its manifest and crate information.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    manifest: Manifest,
    crate_name: CrateName,
    target_dir: PathBuf,
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
    pub async fn run_with_options<B: Backend, D: Device>(
        &self,
        backend: &B,
        platform: TargetPlatform,
        device: D,
        run_options: RunOptions,
    ) -> Result<Running, FailToRun> {
        // Build rust library for the target platform
        backend
            .build(self, platform, BuildOptions::new(false))
            .await
            .map_err(FailToRun::Build)?;

        // Package the build artifacts for the target platform
        let artifact = backend
            .package(self, platform, PackageOptions::new(false, true))
            .await
            .map_err(FailToRun::Package)?;

        Self::run_packaged(device, artifact, run_options).await
    }

    /// Run the Android backend for the specific target ABI of the device.
    ///
    /// This is required because Android packaging is ABI-dependent (e.g., x86_64 emulator vs
    /// arm64-v8a physical device).
    pub async fn run_android_with_options<D: Device + AndroidAbiProvider>(
        &self,
        _backend: &AndroidBackend,
        device: D,
        run_options: RunOptions,
    ) -> Result<Running, FailToRun> {
        let abi = device.android_abi();

        AndroidPlatform::clean_jni_libs(self)
            .await
            .map_err(FailToRun::Build)?;

        AndroidPlatform::new(abi)
            .build(self, BuildOptions::new(false))
            .await
            .map_err(FailToRun::Build)?;

        let artifact =
            AndroidPlatform::package_with_abis(self, PackageOptions::new(false, true), &[abi])
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
    #[must_use]
    pub fn target_dir(&self) -> &Path {
        &self.target_dir
    }

    /// Get backend-specific target directory under the project's resolved Cargo target directory.
    ///
    /// This keeps generated backend builds inside the user-controlled target tree.
    #[must_use]
    pub fn backend_target_dir(&self, backend_name: &str) -> PathBuf {
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.root.display().to_string().as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        let project_fingerprint = &digest[..12];
        self.target_dir
            .join("water-backends")
            .join(format!("{backend_name}-{project_fingerprint}"))
    }

    /// Get the backends configured for the project.
    #[must_use]
    pub const fn backends(&self) -> &Backends {
        &self.manifest.backends
    }

    /// Get the crate name of the project.
    #[must_use]
    pub fn crate_name(&self) -> &CrateName {
        &self.crate_name
    }

    /// Get configured or default FFI crate name for app mode.
    #[must_use]
    pub fn ffi_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.ffi.clone())
            .unwrap_or_else(|| self.crate_name.with_suffix("ffi"))
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

    /// Get the manifest of the project.
    #[must_use]
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
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
            gtk4::platform::clean_gtk4, hydrolysis::platform::clean_hydrolysis,
        };

        if self.is_playground() {
            crate::water_dir::remove_project_build_cache(self.root()).await?;
            return Ok(());
        }

        // Clean Rust target directory
        let target_dir = self.target_dir();
        if target_dir.exists() {
            smol::fs::remove_dir_all(target_dir).await?;
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

    /// Backends configuration is not allowed in playground manifests.
    #[error("Backends configuration is not allowed in playground projects")]
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
    /// Bundle identifier (e.g., "com.example.waterexample").
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
        let ctx = TemplateContext::for_project_manifest(
            manifest,
            self.crate_name().clone(),
            app_name,
        )
        .with_backend_project_path(self.ffi_crate_path())
        .with_project_root_path(self.root.clone());

        templates::ffi::scaffold(&self.ffi_crate_path(), &ctx, &self.ffi_crate_name())
            .await
            .map_err(crate::backend::FailToInitBackend::Io)
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

        // Scaffold root files (Cargo.toml, src/lib.rs, .gitignore)
        templates::root::scaffold(&path, &ctx)
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
                assets_path: default_assets_path(),
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

        let target_dir = get_target_dir(&path)
            .await
            .map_err(FailToCreateProject::TargetDirError)?;
        let managed_backends_root = if options.package_type == PackageType::Playground {
            crate::water_dir::project_build_cache_dir(&path)
                .await
                .map_err(FailToCreateProject::BuildCache)?
        } else {
            path.join(manifest.backends.path())
        };

        Ok(Self {
            root: path,
            manifest,
            crate_name,
            target_dir,
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
            let path = backend.project_path().to_path_buf();
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
            let path = backend.project_path().to_path_buf();
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
            let path = backend.project_path().to_path_buf();
            self.remove_backend_relative_dir(&path).await?;
        }
        self.manifest.backends.clear_hydrolysis();
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
        use crate::backend::Backend;

        let path = path.as_ref().to_path_buf();
        let manifest = Manifest::open(path.join("Water.toml"))
            .await
            .map_err(FailToOpenProject::Manifest)?;

        let cargo_path = path.join("Cargo.toml");

        let cargo_manifest = unblock(move || CargoManifest::from_path(cargo_path))
            .await
            .map_err(FailToOpenProject::CargoManifest)?;
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

        // Check that backends are not configured in playground manifests
        if is_playground && !manifest.backends.is_empty() {
            return Err(FailToOpenProject::BackendsNotAllowedInPlayground);
        }

        let managed_backends_root = if is_playground {
            crate::water_dir::ensure_project_build_cache(&path)
                .await
                .map_err(FailToOpenProject::BuildCache)?
        } else {
            path.join(manifest.backends.path())
        };

        let target_dir = get_target_dir(&path)
            .await
            .map_err(FailToOpenProject::TargetDirError)?;

        let mut project = Self {
            root: path,
            manifest,
            crate_name,
            target_dir,
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
            .map(|v| v == "1")
            .unwrap_or(false)
            || std::env::var("ACTION").is_ok() // Xcode sets this during builds
            || std::env::var("XCODE_PRODUCT_BUILD_VERSION").is_ok();

        if is_playground && !skip_backend_init {
            // Apple backend - always re-scaffold to pick up manifest changes.
            let apple_backend = AppleBackend::init(&project)
                .await
                .map_err(FailToOpenProject::BackendInit)?;
            project.manifest.backends.set_apple(apple_backend);

            // Android backend - always re-scaffold to pick up manifest changes.
            let android_backend = AndroidBackend::init(&project)
                .await
                .map_err(FailToOpenProject::BackendInit)?;
            project.manifest.backends.set_android(android_backend);

            project
                .scaffold_ffi_companion()
                .await
                .map_err(FailToOpenProject::BackendInit)?;
        }

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

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use smol::{fs::read_to_string, process::Command, unblock};
use waterui_assets_plan::ThemeConfig;

use crate::{
    android::{backend::AndroidBackend, device::AndroidAbiProvider, platform::AndroidPlatform},
    apple::backend::AppleBackend,
    backend::{Backend, Backends},
    build::BuildOptions,
    device::{Artifact, Device, FailToRun, RunOptions, Running},
    platform::{PackageOptions, TargetPlatform},
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
    /// Bundle identifier for the application (e.g., "com.example.waterdemo").
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

fn is_false(value: &bool) -> bool {
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
