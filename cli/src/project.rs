use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, Context, bail};
use heck::ToSnakeCase;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use which::which;

use crate::{
    backend::{AnyBackend, Backend, scan_backends},
    crash::CrashReport,
    device::Device,
    platform::Platform,
    toolchain::ToolchainError,
};

#[derive(Debug)]
pub struct Project {
    dir: PathBuf,
    config: Config,
    identifier: String,
    cargo_manifest: CargoManifest,
}

/// Parsed Cargo.toml manifest for reading crate metadata.
#[derive(Debug, Clone)]
struct CargoManifest {
    name: String,
    authors: Vec<String>,
}

impl CargoManifest {
    fn load(project_dir: &Path) -> eyre::Result<Self> {
        let cargo_path = project_dir.join("Cargo.toml");
        let manifest = cargo_toml::Manifest::from_path(&cargo_path)
            .with_context(|| format!("failed to parse {}", cargo_path.display()))?;

        let package = manifest
            .package
            .ok_or_else(|| eyre::eyre!("missing [package] section in Cargo.toml"))?;

        let name = package.name;

        let authors = match package.authors {
            cargo_toml::Inheritable::Set(authors) => authors,
            cargo_toml::Inheritable::Inherited => Vec::new(),
        };

        Ok(Self { name, authors })
    }
}

/// Read crate name from Cargo.toml in the given project directory.
///
/// # Errors
/// Returns an error if `Cargo.toml` cannot be read or parsed.
pub fn read_crate_name(project_dir: &Path) -> eyre::Result<String> {
    let manifest = CargoManifest::load(project_dir)?;
    Ok(manifest.name)
}

/// Controls build and runtime behavior for `Project::run`.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Compile and package with release optimizations.
    pub release: bool,
    /// Hot reload configuration forwarded to platforms/devices.
    pub hot_reload: HotReloadOptions,
    /// Log filter (`RUST_LOG` syntax) forwarded to hot reload clients.
    pub log_filter: Option<String>,
}

impl RunOptions {
    #[must_use]
    pub const fn new(release: bool, hot_reload: HotReloadOptions) -> Self {
        Self {
            release,
            hot_reload,
            log_filter: None,
        }
    }
}

/// Toggle hot reload metadata generation and runtime wiring.
#[derive(Debug, Clone, Copy)]
pub struct HotReloadOptions {
    /// Whether hot reload support is enabled end-to-end.
    pub enabled: bool,
    /// TCP port used by the dev server.
    pub port: u16,
}

impl Default for HotReloadOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_HOT_RELOAD_PORT,
        }
    }
}

/// Structured description of a successful `Project::run`.
#[derive(Debug, Serialize)]
pub struct RunReport {
    /// Absolute path to the packaged artifact executed on the target device.
    pub artifact: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crash_report: Option<CrashReport>,
}

crate::impl_report!(RunReport, |r| {
    if r.crash_report.is_some() {
        format!("App ran and crashed: {}", r.artifact.display())
    } else {
        format!("App ran: {}", r.artifact.display())
    }
});

/// Build/run lifecycle milestones surfaced while executing [`Project::run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStage {
    /// Preparing the selected device or simulator.
    PrepareDevice,
    /// Compiling the Rust code via Cargo.
    BuildRust,
    /// Packaging/bundling for the target platform (e.g., Xcode build).
    Package,
    /// Launching the produced artifact on the device.
    Launch,
}

/// Alias retained for backwards compatibility with the legacy CLI code.
pub type ProjectConfig = Config;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("TOML error: {0}")]
    TomlError(#[from] toml::de::Error),
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Backend {0} already exists for this project")]
    BackendExists(String),
    #[error("{0}")]
    Other(#[from] eyre::Report),
}

#[derive(Debug, Error)]
pub enum FailToPackage {
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Toolchain requirement not met")]
    RequirementNotMet(Vec<ToolchainError>),
    #[error("Packaging error: {0}")]
    Other(#[from] eyre::Report),
}

impl From<Vec<ToolchainError>> for FailToPackage {
    fn from(issues: Vec<ToolchainError>) -> Self {
        Self::RequirementNotMet(issues)
    }
}

impl From<ToolchainError> for FailToPackage {
    fn from(issue: ToolchainError) -> Self {
        Self::RequirementNotMet(vec![issue])
    }
}

fn check_requirements() -> Result<(), ToolchainError> {
    // check rust basic toolchain
    if which("rustc").is_err() {
        return Err(ToolchainError::unfixable("Rust is not installed")
            .with_suggestion("Install Rust from https://rustup.rs"));
    }

    Ok(())
}

/// Errors that can occur when running a project.
#[derive(Debug, Error)]
pub enum FailToRun {
    /// Build error
    #[error("Build error: {0}")]
    BuildError(String),
    /// Toolchain requirement not met
    #[error("Toolchain requirement not met")]
    RequirementNotMet(Vec<ToolchainError>),
    /// Other runtime error
    #[error("Runtime error: {0}")]
    Other(#[from] eyre::Report),
}

impl From<Vec<ToolchainError>> for FailToRun {
    fn from(issues: Vec<ToolchainError>) -> Self {
        Self::RequirementNotMet(issues)
    }
}

impl From<ToolchainError> for FailToRun {
    fn from(issue: ToolchainError) -> Self {
        Self::RequirementNotMet(vec![issue])
    }
}

impl Project {
    /// Open a `WaterUI` project from the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `Water.toml` file cannot be read from the directory
    /// - The TOML file cannot be parsed into a valid `ProjectConfig`
    /// - The `Cargo.toml` file cannot be read or parsed
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref().to_path_buf();
        let config = Config::load(&dir).map_err(Error::Other)?;
        let cargo_manifest = CargoManifest::load(&dir).map_err(Error::Other)?;
        let identifier = cargo_manifest.name.to_snake_case();
        Ok(Self {
            dir,
            config,
            identifier,
            cargo_manifest,
        })
    }

    /// Run the project on the specified platform.
    ///
    /// This will build the project in debug mode and launch it on the target platform.
    /// # Errors
    /// Returns `FailToRun` if any step of the run process fails.
    pub fn run(&self, device: &impl Device, options: RunOptions) -> Result<RunReport, FailToRun> {
        self.run_with_observer(device, options, |_| {})
    }

    /// Run the project while reporting lifecycle milestones to the provided observer.
    ///
    /// This allows callers to react to stage transitions (e.g., updating UI progress).
    pub fn run_with_observer<O: FnMut(RunStage)>(
        &self,
        device: &impl Device,
        options: RunOptions,
        mut observer: O,
    ) -> Result<RunReport, FailToRun> {
        check_requirements()?;

        // First check requirements with the initial platform
        let platform = device.platform();
        platform.check_requirements(self)?;

        // Prepare device - this may detect device-specific information
        // like target architecture for Android devices
        observer(RunStage::PrepareDevice);
        device.prepare(self, &options)?;

        // Get platform again after prepare() to pick up any
        // device-specific configuration (e.g., detected target architecture)
        let platform = device.platform();

        // Note: Rust build is handled by platform.package() which has
        // incremental compilation. No need for separate build step.

        observer(RunStage::Package);
        let app_path = platform.package(self, options.release)?;

        observer(RunStage::Launch);
        let crash_report = device.run(self, &app_path, &options)?;

        Ok(RunReport {
            artifact: app_path,
            crash_report,
        })
    }

    /// Package the project for the specified platform.
    ///
    /// This will build the project and create a distributable package.
    /// The platform implementation handles both Rust compilation and
    /// platform-specific packaging in a single step.
    ///
    /// # Errors
    ///
    /// Returns `FailToPackage` if any step of the packaging process fails.
    pub fn package(
        &self,
        platform: &impl Platform,
        release: bool,
    ) -> Result<PathBuf, FailToPackage> {
        check_requirements()?;
        platform.check_requirements(self)?;

        // Platform::package() handles Rust compilation and packaging together
        let artifact = platform.package(self, release)?;
        Ok(artifact)
    }

    /// Get the display name of the project (human-readable).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.config.package.name
    }

    /// Get the crate name from Cargo.toml (kebab-case identifier).
    #[must_use]
    pub fn crate_name(&self) -> &str {
        &self.cargo_manifest.name
    }

    /// Get the unique identifier of the project (`snake_case` from crate name).
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Get the list of backends configured for the project.
    #[must_use]
    pub fn backends(&self) -> Vec<AnyBackend> {
        scan_backends(self)
    }

    /// Access the parsed project configuration.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Get the author of the project from Cargo.toml.
    /// Returns the first author if multiple are specified.
    #[must_use]
    pub fn author(&self) -> &str {
        self.cargo_manifest
            .authors
            .first()
            .map_or("", String::as_str)
    }

    /// Bundle identifier used for Apple/Android targets.
    #[must_use]
    pub fn bundle_identifier(&self) -> &str {
        &self.config.package.bundle_identifier
    }

    /// Get the root directory of the project.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// Add a backend to the project.
    ////
    /// # Errors
    /// Returns an error if the backend already exists for the project.
    pub fn add_backend<B: Backend>(&self, backend: B, dev: bool) -> Result<(), Error> {
        if backend.is_existing(self) {
            return Err(Error::BackendExists(backend.to_string()));
        }
        backend.init(self, dev)?;
        Ok(())
    }

    /// Check if this is a playground project.
    #[must_use]
    pub fn is_playground(&self) -> bool {
        self.config.package.package_type == PackageType::Playground
    }

    /// Get the package type.
    #[must_use]
    pub const fn package_type(&self) -> PackageType {
        self.config.package.package_type
    }
}

/// Configuration for a `WaterUI` project persisted to `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub package: Package,
    #[serde(default, skip_serializing_if = "Backends::is_empty")]
    pub backends: Backends,
    #[serde(default, skip_serializing_if = "HotReload::is_empty")]
    pub hot_reload: HotReload,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dev_dependencies: bool,
    /// Path to local `WaterUI` repository for dev mode.
    /// When set, all dependencies use local paths instead of git/crates.io.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waterui_path: Option<String>,
    /// Permission configuration for playground projects.
    #[serde(default, skip_serializing_if = "Permissions::is_empty")]
    pub permissions: Permissions,
}

const fn is_false(b: &bool) -> bool {
    !*b
}

impl Config {
    #[must_use]
    pub fn new(package: Package) -> Self {
        Self {
            package,
            backends: Backends::default(),
            hot_reload: HotReload::default(),
            dev_dependencies: false,
            waterui_path: None,
            permissions: Permissions::default(),
        }
    }

    /// Load the project configuration from disk.
    ///
    /// # Errors
    /// Returns an error if `Water.toml` cannot be read or parsed.
    pub fn load(root: &Path) -> eyre::Result<Self> {
        let path = Self::path(root);
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => bail!(
                "No WaterUI project found in {} (missing {}). Run this command from a WaterUI project root or pass --project to point to one. Create a new project with `water init`.",
                root.display(),
                path.display()
            ),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Persist the configuration to `Water.toml`.
    ///
    /// # Errors
    /// Returns an error if the configuration file cannot be written.
    pub fn save(&self, root: &Path) -> eyre::Result<()> {
        let path = Self::path(root);
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(&path, contents)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    #[must_use]
    pub fn path(root: &Path) -> PathBuf {
        root.join("Water.toml")
    }

    /// Check if this is a playground configuration.
    #[must_use]
    pub fn is_playground(&self) -> bool {
        self.package.package_type == PackageType::Playground
    }
}

/// Get the playground cache directory for a project.
/// This is where temporary platform backends are created for playground projects.
///
/// Structure: `<project>/.water/playground/`
#[must_use]
pub fn playground_cache_dir(project_root: &Path) -> PathBuf {
    project_root.join(".water").join("playground")
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    /// Type of the package (e.g., "app").
    #[serde(rename = "type")]
    pub package_type: PackageType,
    /// Human-readable name of the application (e.g., "Water Demo").
    pub name: String,
    pub bundle_identifier: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Backends {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swift: Option<Swift>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android: Option<Android>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<Web>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tui: Option<Tui>,
}

impl Backends {
    const fn is_empty(&self) -> bool {
        self.swift.is_none() && self.android.is_none() && self.web.is_none() && self.tui.is_none()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Swift {
    #[serde(
        default = "default_swift_project_path",
        skip_serializing_if = "is_default_swift_project_path"
    )]
    pub project_path: String,
    pub scheme: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Local path to the Apple backend for local dev mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dev: bool,
}

#[must_use]
pub fn default_swift_project_path() -> String {
    "apple".to_string()
}

fn is_default_swift_project_path(s: &str) -> bool {
    s == "apple"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Android {
    #[serde(
        default = "default_android_project_path",
        skip_serializing_if = "is_default_android_project_path"
    )]
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dev: bool,
}

#[must_use]
pub fn default_android_project_path() -> String {
    "android".to_string()
}

fn is_default_android_project_path(s: &str) -> bool {
    s == "android"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Web {
    #[serde(
        default = "default_web_project_path",
        skip_serializing_if = "is_default_web_project_path"
    )]
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dev: bool,
}

#[must_use]
pub fn default_web_project_path() -> String {
    "web".to_string()
}

fn is_default_web_project_path(s: &str) -> bool {
    s == "web"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tui {
    #[serde(
        default = "default_tui_project_path",
        skip_serializing_if = "is_default_tui_project_path"
    )]
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

fn default_tui_project_path() -> String {
    "tui".to_string()
}

fn is_default_tui_project_path(s: &str) -> bool {
    s == "tui"
}

/// Default hot reload server port.
pub const DEFAULT_HOT_RELOAD_PORT: u16 = 2006;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HotReload {
    /// TCP port for the hot reload WebSocket server.
    /// Defaults to 2006.
    #[serde(default = "default_hot_reload_port")]
    pub port: u16,
    /// Additional paths to watch for triggering rebuilds
    #[serde(default)]
    pub watch: Vec<String>,
}

impl Default for HotReload {
    fn default() -> Self {
        Self {
            port: DEFAULT_HOT_RELOAD_PORT,
            watch: Vec::new(),
        }
    }
}

const fn default_hot_reload_port() -> u16 {
    DEFAULT_HOT_RELOAD_PORT
}

impl HotReload {
    const fn is_empty(&self) -> bool {
        self.port == DEFAULT_HOT_RELOAD_PORT && self.watch.is_empty()
    }
}

/// Permission configuration for playground projects.
///
/// Supports two formats in Water.toml:
/// 1. Simple list: `enabled = ["camera", "location"]`
/// 2. With descriptions: `[permissions.camera] description = "..."`
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Permissions {
    /// List of enabled permissions (simple form).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled: Vec<String>,

    /// Permission-specific configurations with custom descriptions.
    #[serde(flatten)]
    pub detailed: HashMap<String, PermissionConfig>,
}

impl Permissions {
    /// Check if permissions section is empty (for serialization skip).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty() && self.detailed.is_empty()
    }

    /// Check if a permission is configured.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.enabled.contains(&name.to_string()) || self.detailed.contains_key(name)
    }

    /// Remove a permission from the configuration.
    /// Returns true if the permission was found and removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let in_enabled = self.enabled.iter().position(|p| p == name).map(|i| {
            self.enabled.remove(i);
        });
        let in_detailed = self.detailed.remove(name);
        in_enabled.is_some() || in_detailed.is_some()
    }

    /// Get all enabled permission names (combining both formats).
    #[must_use]
    pub fn all_enabled(&self) -> Vec<String> {
        let mut perms: Vec<String> = self.enabled.clone();
        for (name, config) in &self.detailed {
            if config.is_enabled() && !perms.contains(name) {
                perms.push(name.clone());
            }
        }
        perms
    }

    /// Get the custom description for a permission, if any.
    #[must_use]
    pub fn get_description(&self, name: &str) -> Option<String> {
        self.detailed
            .get(name)
            .and_then(PermissionConfig::description)
    }

    /// Add a permission to the enabled list (simple form).
    pub fn add(&mut self, name: String) {
        if !self.has(&name) {
            self.enabled.push(name);
        }
    }

    /// Add a permission with a custom description.
    pub fn add_with_description(&mut self, name: String, description: String) {
        // Remove from enabled list if present (since we're adding with description)
        if let Some(pos) = self.enabled.iter().position(|p| p == &name) {
            self.enabled.remove(pos);
        }
        self.detailed
            .insert(name, PermissionConfig::WithDescription { description });
    }
}

/// Configuration for a single permission.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum PermissionConfig {
    /// Simple enable: `permission = true`
    Enabled(bool),
    /// With description: `permission = { description = "..." }`
    WithDescription { description: String },
}

impl PermissionConfig {
    /// Check if this permission is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        match self {
            Self::Enabled(enabled) => *enabled,
            Self::WithDescription { .. } => true,
        }
    }

    /// Get the description if present.
    #[must_use]
    pub fn description(&self) -> Option<String> {
        match self {
            Self::Enabled(_) => None,
            Self::WithDescription { description } => Some(description.clone()),
        }
    }
}
