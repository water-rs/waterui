//! Android SDK toolchain and installers.

use std::{
    env,
    fmt::{self, Display},
    future::Future,
    path::PathBuf,
};

use which::which;

use super::{
    Toolchain, ToolchainError,
    installation::{Installation, InstallationReport, Many, Sequence},
    rust::RustTarget,
};

// ============================================================================
// Toolchain
// ============================================================================

/// Android SDK toolchain configuration.
#[derive(Debug, Clone, Default)]
pub struct Android {
    rust_targets: Vec<String>,
    require_cmake: bool,
}

impl Android {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_rust_target(mut self, target: impl Into<String>) -> Self {
        self.rust_targets.push(target.into());
        self
    }

    #[must_use]
    pub const fn with_cmake(mut self) -> Self {
        self.require_cmake = true;
        self
    }

    pub fn find_sdk_path() -> Option<PathBuf> {
        env::var("ANDROID_HOME")
            .or_else(|_| env::var("ANDROID_SDK_ROOT"))
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .or_else(Self::default_sdk_path)
    }

    fn default_sdk_path() -> Option<PathBuf> {
        let home = home::home_dir()?;

        #[cfg(target_os = "macos")]
        let path = home.join("Library/Android/sdk");
        #[cfg(target_os = "linux")]
        let path = home.join("Android/Sdk");
        #[cfg(target_os = "windows")]
        let path = home.join("AppData/Local/Android/Sdk");
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let path = home.join("android-sdk");

        if path.exists() { Some(path) } else { None }
    }

    pub fn find_ndk_path() -> Option<PathBuf> {
        env::var("ANDROID_NDK_HOME")
            .or_else(|_| env::var("NDK_HOME"))
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .or_else(|| {
                let sdk = Self::find_sdk_path()?;
                let ndk_dir = sdk.join("ndk");
                if ndk_dir.exists() {
                    std::fs::read_dir(&ndk_dir)
                        .ok()?
                        .filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .max()
                } else {
                    None
                }
            })
    }

    fn has_java() -> bool {
        env::var("JAVA_HOME").is_ok() || which("java").is_ok()
    }

    fn has_cmake() -> bool {
        which("cmake").is_ok() || Self::find_sdk_cmake().is_some()
    }

    fn find_sdk_cmake() -> Option<PathBuf> {
        let sdk = Self::find_sdk_path()?;
        let cmake_dir = sdk.join("cmake");
        if cmake_dir.exists() {
            std::fs::read_dir(&cmake_dir)
                .ok()?
                .filter_map(Result::ok)
                .map(|e| e.path().join("bin/cmake"))
                .find(|p| p.exists())
        } else {
            None
        }
    }

    fn installed_rust_targets() -> Vec<String> {
        Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn missing_rust_targets(&self) -> Vec<String> {
        if self.rust_targets.is_empty() {
            return Vec::new();
        }

        let installed = Self::installed_rust_targets();
        self.rust_targets
            .iter()
            .filter(|t| !installed.contains(t))
            .cloned()
            .collect()
    }

    fn find_sdkmanager() -> Option<PathBuf> {
        let sdk = Self::find_sdk_path()?;
        let cmdline_tools = sdk.join("cmdline-tools");

        for subdir in &["latest", ""] {
            let path = if subdir.is_empty() {
                cmdline_tools.join("bin/sdkmanager")
            } else {
                cmdline_tools.join(subdir).join("bin/sdkmanager")
            };
            if path.exists() {
                return Some(path);
            }
        }

        let tools_path = sdk.join("tools/bin/sdkmanager");
        if tools_path.exists() {
            return Some(tools_path);
        }

        None
    }
}

/// Installation type for Android toolchain.
#[derive(Debug)]
pub enum AndroidInstallation {
    RustTargets(Many<RustTarget>),
    SdkThenRust(Sequence<Many<SdkComponent>, Many<RustTarget>>),
    SdkOnly(Many<SdkComponent>),
    #[cfg(target_os = "macos")]
    Jdk(Sequence<HomebrewJdk, AndroidInstallationRest>),
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub enum AndroidInstallationRest {
    RustTargets(Many<RustTarget>),
    SdkThenRust(Sequence<Many<SdkComponent>, Many<RustTarget>>),
    SdkOnly(Many<SdkComponent>),
}

#[cfg(target_os = "macos")]
impl Display for AndroidInstallationRest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(e) => write!(f, "{e}"),
            Self::RustTargets(r) => write!(f, "{r}"),
            Self::SdkThenRust(s) => write!(f, "{s}"),
            Self::SdkOnly(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(target_os = "macos")]
impl Installation for AndroidInstallationRest {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        match self {
            Self::Empty(_) => "nothing",
            Self::RustTargets(_) => "Rust targets",
            Self::SdkThenRust(_) => "SDK and Rust targets",
            Self::SdkOnly(_) => "SDK components",
        }
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            match self {
                Self::Empty(e) => e.install(progress).await,
                Self::RustTargets(r) => r.install(progress).await,
                Self::SdkThenRust(s) => s.install(progress).await,
                Self::SdkOnly(s) => s.install(progress).await,
            }
        }
    }
}

impl Display for AndroidInstallation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(e) => write!(f, "{e}"),
            Self::RustTargets(r) => write!(f, "{r}"),
            Self::SdkThenRust(s) => write!(f, "{s}"),
            Self::SdkOnly(s) => write!(f, "{s}"),
            #[cfg(target_os = "macos")]
            Self::Jdk(j) => write!(f, "{j}"),
        }
    }
}

impl Installation for AndroidInstallation {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        match self {
            Self::Empty(_) => "nothing",
            Self::RustTargets(_) => "Rust targets",
            Self::SdkThenRust(_) => "SDK and Rust targets",
            Self::SdkOnly(_) => "SDK components",
            #[cfg(target_os = "macos")]
            Self::Jdk(_) => "JDK and Android toolchain",
        }
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            match self {
                Self::Empty(e) => e.install(progress).await,
                Self::RustTargets(r) => r.install(progress).await,
                Self::SdkThenRust(s) => s.install(progress).await,
                Self::SdkOnly(s) => s.install(progress).await,
                #[cfg(target_os = "macos")]
                Self::Jdk(j) => j.install(progress).await,
            }
        }
    }
}

impl Toolchain for Android {
    type Installation = AndroidInstallation;

    fn name(&self) -> &'static str {
        "Android"
    }

    async fn check(&self) -> Result<(), ToolchainError> {
        if Self::find_sdk_path().is_none() {
            return Err(ToolchainError::missing("Android SDK not found")
                .with_suggestion("Install Android Studio or set ANDROID_HOME"));
        }

        if Self::find_ndk_path().is_none() {
            return Err(ToolchainError::missing("Android NDK not found")
                .with_suggestion("Install NDK via Android Studio SDK Manager"));
        }

        if !Self::has_java() {
            return Err(ToolchainError::missing("JDK not found").with_suggestion(
                if cfg!(target_os = "macos") {
                    "Run: brew install --cask temurin@17"
                } else {
                    "Install JDK 17 and set JAVA_HOME"
                },
            ));
        }

        if self.require_cmake && !Self::has_cmake() {
            return Err(ToolchainError::missing("CMake not found")
                .with_suggestion("Install via Android SDK Manager"));
        }

        let missing = self.missing_rust_targets();
        if !missing.is_empty() {
            return Err(ToolchainError::missing(format!(
                "Missing Rust targets: {}",
                missing.join(", ")
            ))
            .with_suggestion(format!("Run: rustup target add {}", missing.join(" "))));
        }

        Ok(())
    }

    fn fix(&self) -> Result<Self::Installation, ToolchainError> {
        let mut sdk_components = Vec::new();

        if Self::find_sdk_path().is_none() {
            return Err(
                ToolchainError::unfixable("Android SDK not found").with_suggestion(
                    "Install Android Studio from https://developer.android.com/studio",
                ),
            );
        }

        if Self::find_ndk_path().is_none() {
            if Self::find_sdkmanager().is_some() {
                sdk_components.push(SdkComponent::new("ndk;26.1.10909125", "Android NDK"));
            } else {
                return Err(
                    ToolchainError::unfixable("NDK not found, sdkmanager unavailable")
                        .with_suggestion("Install NDK via Android Studio"),
                );
            }
        }

        let need_java = !Self::has_java();

        if self.require_cmake && !Self::has_cmake() {
            if Self::find_sdkmanager().is_some() {
                sdk_components.push(SdkComponent::new("cmake;3.22.1", "CMake"));
            } else {
                return Err(ToolchainError::unfixable("CMake not found")
                    .with_suggestion("Install CMake via Android SDK Manager"));
            }
        }

        let missing_targets: Vec<_> = self
            .missing_rust_targets()
            .into_iter()
            .map(RustTarget::new)
            .collect();

        #[cfg(target_os = "macos")]
        if need_java {
            let rest = if sdk_components.is_empty() && missing_targets.is_empty() {
                AndroidInstallationRest::Empty(Empty::new())
            } else if sdk_components.is_empty() {
                AndroidInstallationRest::RustTargets(Many::new(missing_targets))
            } else if missing_targets.is_empty() {
                AndroidInstallationRest::SdkOnly(Many::new(sdk_components))
            } else {
                AndroidInstallationRest::SdkThenRust(Sequence {
                    first: Many::new(sdk_components),
                    second: Many::new(missing_targets),
                })
            };
            return Ok(AndroidInstallation::Jdk(Sequence {
                first: HomebrewJdk,
                second: rest,
            }));
        }

        if sdk_components.is_empty() && missing_targets.is_empty() {
            Ok(AndroidInstallation::Empty(Empty::new()))
        } else if sdk_components.is_empty() {
            Ok(AndroidInstallation::RustTargets(Many::new(missing_targets)))
        } else if missing_targets.is_empty() {
            Ok(AndroidInstallation::SdkOnly(Many::new(sdk_components)))
        } else {
            Ok(AndroidInstallation::SdkThenRust(Sequence {
                first: Many::new(sdk_components),
                second: Many::new(missing_targets),
            }))
        }
    }
}

// ============================================================================
// Atomic Installers
// ============================================================================

/// Install an Android SDK component.
#[derive(Debug, Clone)]
pub struct SdkComponent {
    package: String,
    name: String,
}

impl SdkComponent {
    pub fn new(package: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            package: package.into(),
            name: name.into(),
        }
    }
}

impl Display for SdkComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sdkmanager \"{}\"", self.package)
    }
}

impl Installation for SdkComponent {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        &self.name
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            let sdkmanager = Android::find_sdkmanager()
                .ok_or_else(|| ToolchainError::install_failed("sdkmanager not found"))?;

            progress.start(&self.name);
            progress.update(&self.name, 0, "installing");

            let status = AsyncCommand::new(&sdkmanager)
                .args(["--install", &self.package])
                .env("JAVA_TOOL_OPTIONS", "-Dfile.encoding=UTF8")
                .status()
                .await
                .map_err(|e| {
                    progress.fail(&self.name, format!("{e}"));
                    ToolchainError::install_failed(format!("Failed to run sdkmanager: {e}"))
                })?;

            if !status.success() {
                progress.fail(&self.name, "failed");
                return Err(ToolchainError::install_failed(format!(
                    "Failed to install {}",
                    self.package
                ))
                .with_suggestion("Try: sdkmanager --licenses"));
            }

            progress.done(&self.name, "installed");
            Ok(InstallationReport::completed(format!(
                "Installed {}",
                self.name
            )))
        }
    }
}

/// Install JDK via Homebrew (macOS only).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
pub struct HomebrewJdk;

#[cfg(target_os = "macos")]
impl Display for HomebrewJdk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "brew install --cask temurin@17")
    }
}

#[cfg(target_os = "macos")]
impl Installation for HomebrewJdk {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &'static str {
        "JDK 17"
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            progress.start("jdk");
            progress.update("jdk", 0, "installing via Homebrew");

            let status = AsyncCommand::new("brew")
                .args(["install", "--cask", "temurin@17"])
                .status()
                .await
                .map_err(|e| {
                    progress.fail("jdk", format!("{e}"));
                    ToolchainError::install_failed(format!("Failed to run brew: {e}"))
                })?;

            if !status.success() {
                progress.fail("jdk", "failed");
                return Err(ToolchainError::install_failed("Failed to install JDK")
                    .with_suggestion("Try: brew install --cask temurin@17"));
            }

            progress.done("jdk", "installed");
            Ok(InstallationReport::completed(
                "Installed JDK 17 via Homebrew",
            ))
        }
    }
}
