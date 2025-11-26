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
    doctor::{AnyToolchainIssue, ToolchainIssue},
    platform::Platform,
};

#[derive(Debug)]
pub struct Project {
    dir: PathBuf,
    config: Config,
    identifier: String,
}

/// Controls build and runtime behavior for `Project::run`.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Compile and package with release optimizations.
    pub release: bool,
    /// Hot reload configuration forwarded to platforms/devices.
    pub hot_reload: HotReloadOptions,
    /// Log filter (RUST_LOG syntax) forwarded to hot reload clients.
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
#[derive(Debug, Clone, Copy, Default)]
pub struct HotReloadOptions {
    /// Whether hot reload support is enabled end-to-end.
    pub enabled: bool,
    /// TCP port used by the dev server, when enabled.
    pub port: Option<u16>,
}

/// Structured description of a successful `Project::run`.
#[derive(Debug, Serialize)]
pub struct RunReport {
    /// Absolute path to the packaged artifact executed on the target device.
    pub artifact: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crash_report: Option<CrashReport>,
}

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
    RequirementNotMet(Vec<AnyToolchainIssue>),
    #[error("Packaging error: {0}")]
    Other(#[from] eyre::Report),
}

macro_rules! tool_chain_requirement {
    ($ty:ty) => {
        impl<T: ToolchainIssue> From<T> for $ty {
            fn from(issue: T) -> Self {
                let any_issue = AnyToolchainIssue::from(Box::new(issue));
                Self::RequirementNotMet(vec![any_issue])
            }
        }

        impl<T: ToolchainIssue> From<Vec<T>> for $ty {
            fn from(issues: Vec<T>) -> Self {
                let any_issues = issues
                    .into_iter()
                    .map(|issue| AnyToolchainIssue::from(Box::new(issue)))
                    .collect();
                Self::RequirementNotMet(any_issues)
            }
        }
    };
}

tool_chain_requirement!(FailToPackage);
tool_chain_requirement!(FailToRun);

#[derive(Debug, Clone, Error)]
pub enum BasicToolchainIssue {
    /// Rust is not installed
    #[error("Rust is not installed")]
    RustNotInstalled,
}

impl ToolchainIssue for BasicToolchainIssue {
    fn suggestion(&self) -> String {
        match self {
            Self::RustNotInstalled => {
                "Install Rust from https://www.rust-lang.org/tools/install".to_string()
            }
        }
    }

    fn fix(&self) -> eyre::Result<()> {
        todo!()
    }
}

fn check_requirements() -> Result<(), BasicToolchainIssue> {
    // check rust basic toolchain
    if which("rustc").is_err() {
        return Err(BasicToolchainIssue::RustNotInstalled);
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
    RequirementNotMet(Vec<AnyToolchainIssue>),
    /// Other runtime error
    #[error("Runtime error: {0}")]
    Other(#[from] eyre::Report),
}

impl Project {
    /// Open a `WaterUI` project from the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `Water.toml` file cannot be read from the directory
    /// - The TOML file cannot be parsed into a valid `ProjectConfig`
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref().to_path_buf();
        let config = Config::load(&dir).map_err(Error::Other)?;
        let identifier = config.package.name.to_snake_case();
        Ok(Self {
            dir,
            config,
            identifier,
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

    /// Get the name of the project.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.config.package.name
    }

    /// Get the unique identifier of the project.
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

    /// Get the author of the project.
    #[must_use]
    pub const fn author(&self) -> &str {
        self.config.package.author.as_str()
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
}

/// Configuration for a `WaterUI` project persisted to `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub package: Package,
    #[serde(default)]
    pub backends: Backends,
    #[serde(default)]
    pub hot_reload: HotReload,
    #[serde(default)]
    pub dev_dependencies: bool,
}

impl Config {
    #[must_use]
    pub fn new(package: Package) -> Self {
        Self {
            package,
            backends: Backends::default(),
            hot_reload: HotReload::default(),
            dev_dependencies: false,
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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    pub name: String,
    pub display_name: String,
    pub bundle_identifier: String,
    #[serde(default)]
    pub author: String,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Swift {
    #[serde(default = "default_swift_project_path")]
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
    #[serde(default)]
    pub dev: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ffi_version: Option<String>,
}

fn default_swift_project_path() -> String {
    "apple".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Android {
    #[serde(default = "default_android_project_path")]
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub dev: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ffi_version: Option<String>,
}

fn default_android_project_path() -> String {
    "android".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Web {
    #[serde(default = "default_web_project_path")]
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub dev: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ffi_version: Option<String>,
}

fn default_web_project_path() -> String {
    "web".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tui {
    #[serde(default = "default_tui_project_path")]
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

fn default_tui_project_path() -> String {
    "tui".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct HotReload {
    /// Additional paths to watch for triggering rebuilds
    #[serde(default)]
    pub watch: Vec<String>,
}
