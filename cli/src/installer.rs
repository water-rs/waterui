//! Unified package installation system for WaterUI CLI.
//!
//! This module provides a consistent way to install dependencies across platforms.
//! It is used by `ToolchainIssue::fix()` implementations to automatically resolve
//! missing dependencies.
//!
//! # Design Philosophy
//!
//! - All installation logic is centralized here, not scattered across backends
//! - Each installer function is synchronous and blocks until completion
//! - Errors are descriptive and include remediation hints
//! - Platform differences are abstracted away from callers
//!
//! # Example
//!
//! ```ignore
//! use waterui_cli::installer;
//!
//! // Install a Rust target
//! installer::rust_target("aarch64-linux-android")?;
//!
//! // Install a system package
//! installer::system_package("cmake")?;
//!
//! // Install a macOS cask
//! installer::homebrew_cask("temurin@17")?;
//! ```

use std::process::Command;

use color_eyre::eyre::{Context, Result, bail};
use tracing::info;
#[cfg(target_os = "linux")]
use tracing::warn;
use which::which;

// ============================================================================
// Rust Toolchain Installation
// ============================================================================

/// Install a Rust target via rustup.
///
/// # Errors
/// Returns an error if rustup is not available or the installation fails.
///
/// # Example
/// ```ignore
/// installer::rust_target("aarch64-linux-android")?;
/// ```
pub fn rust_target(target: &str) -> Result<()> {
    if which("rustup").is_err() {
        bail!(
            "rustup is not installed. Install Rust from https://rustup.rs first, \
             then run `rustup target add {target}`"
        );
    }

    info!("Installing Rust target: {target}");

    let status = Command::new("rustup")
        .args(["target", "add", target])
        .status()
        .context("failed to run rustup")?;

    if status.success() {
        info!("Successfully installed Rust target: {target}");
        Ok(())
    } else {
        bail!(
            "`rustup target add {target}` failed with exit code {}. \
             Check your internet connection and try again.",
            status.code().unwrap_or(-1)
        )
    }
}

/// Check if a Rust target is installed.
#[must_use]
pub fn is_rust_target_installed(target: &str) -> bool {
    if which("rustup").is_err() {
        // No rustup means we can't check; assume it's fine (system Rust)
        return true;
    }

    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.trim() == target)
        })
        .unwrap_or(true)
}

/// Install rustup and Rust toolchain.
///
/// # Errors
/// Returns an error if Rust is already installed or installation fails.
pub fn rust_toolchain() -> Result<()> {
    if which("rustc").is_ok() {
        bail!("Rust is already installed");
    }

    info!("Installing Rust toolchain via rustup...");

    let status = Command::new("sh")
        .args([
            "-c",
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        ])
        .status()
        .context("failed to run rustup installer")?;

    if status.success() {
        info!("Rust toolchain installed successfully");
        Ok(())
    } else {
        bail!("Rust installation failed. Visit https://rustup.rs for manual installation.")
    }
}

// ============================================================================
// System Package Installation
// ============================================================================

/// Install a system package using the appropriate package manager.
///
/// On macOS, uses Homebrew. On Linux, detects apt/dnf/pacman.
///
/// # Errors
/// Returns an error if no supported package manager is found or installation fails.
///
/// # Example
/// ```ignore
/// installer::system_package("cmake")?;
/// ```
pub fn system_package(name: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        homebrew_formula(name)
    }

    #[cfg(target_os = "linux")]
    {
        linux_package(name)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        bail!(
            "Automatic package installation is not supported on this OS. \
             Please install `{name}` manually."
        )
    }
}

/// Install a Homebrew formula (macOS).
///
/// # Errors
/// Returns an error if Homebrew is not installed or installation fails.
#[cfg(target_os = "macos")]
pub fn homebrew_formula(name: &str) -> Result<()> {
    if which("brew").is_err() {
        bail!(
            "Homebrew is not installed. Install from https://brew.sh first, \
             then run `brew install {name}`"
        );
    }

    info!("Installing via Homebrew: {name}");

    let status = Command::new("brew")
        .args(["install", name])
        .status()
        .context("failed to run brew")?;

    if status.success() {
        info!("Successfully installed: {name}");
        Ok(())
    } else {
        bail!("`brew install {name}` failed. Try running it manually to see the error.")
    }
}

/// Install a Homebrew cask (macOS applications).
///
/// # Errors
/// Returns an error if Homebrew is not installed or installation fails.
///
/// # Example
/// ```ignore
/// // Install JDK 17
/// installer::homebrew_cask("temurin@17")?;
/// ```
#[cfg(target_os = "macos")]
pub fn homebrew_cask(name: &str) -> Result<()> {
    if which("brew").is_err() {
        bail!(
            "Homebrew is not installed. Install from https://brew.sh first, \
             then run `brew install --cask {name}`"
        );
    }

    info!("Installing Homebrew cask: {name}");

    let status = Command::new("brew")
        .args(["install", "--cask", name])
        .status()
        .context("failed to run brew")?;

    if status.success() {
        info!("Successfully installed cask: {name}");
        Ok(())
    } else {
        bail!("`brew install --cask {name}` failed. Try running it manually to see the error.")
    }
}

/// Install a package on Linux using the detected package manager.
#[cfg(target_os = "linux")]
fn linux_package(name: &str) -> Result<()> {
    let (manager, args) = detect_linux_package_manager()?;

    info!("Installing via {manager}: {name}");

    let status = Command::new("sudo")
        .args(args)
        .arg(name)
        .status()
        .with_context(|| format!("failed to run {manager}"))?;

    if status.success() {
        info!("Successfully installed: {name}");
        Ok(())
    } else {
        bail!("Package installation failed. Try running `sudo {manager} install {name}` manually.")
    }
}

/// Detect the Linux package manager and return the appropriate command args.
#[cfg(target_os = "linux")]
fn detect_linux_package_manager() -> Result<(&'static str, Vec<&'static str>)> {
    if which("apt").is_ok() {
        Ok(("apt", vec!["apt", "install", "-y"]))
    } else if which("dnf").is_ok() {
        Ok(("dnf", vec!["dnf", "install", "-y"]))
    } else if which("pacman").is_ok() {
        Ok(("pacman", vec!["pacman", "-S", "--noconfirm"]))
    } else if which("zypper").is_ok() {
        Ok(("zypper", vec!["zypper", "install", "-y"]))
    } else {
        bail!(
            "No supported package manager found (apt, dnf, pacman, zypper). \
             Please install the package manually."
        )
    }
}

// ============================================================================
// Java Installation
// ============================================================================

/// Install Java Development Kit.
///
/// On macOS, installs Temurin JDK 17 via Homebrew.
/// On Linux, installs openjdk-17-jdk via the system package manager.
///
/// # Errors
/// Returns an error if installation fails or the platform doesn't support auto-install.
pub fn java_jdk() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        homebrew_cask("temurin@17")
    }

    #[cfg(target_os = "linux")]
    {
        // Try common JDK package names
        let jdk_packages = ["openjdk-17-jdk", "java-17-openjdk-devel", "jdk17-openjdk"];

        for package in jdk_packages {
            match linux_package(package) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    warn!("Failed to install {package}: {e}");
                    continue;
                }
            }
        }

        bail!(
            "Could not install JDK automatically. Please install JDK 17 manually \
             using your distribution's package manager."
        )
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        bail!(
            "Automatic JDK installation is not supported on this OS. \
             Please install JDK 17 or newer from https://adoptium.net"
        )
    }
}

// ============================================================================
// Suggestion Helpers
// ============================================================================

/// Get a human-readable installation suggestion for a Rust target.
#[must_use]
pub fn rust_target_suggestion(target: &str) -> String {
    format!("Run `rustup target add {target}` to install the required Rust target.")
}

/// Get a human-readable installation suggestion for Java.
#[must_use]
pub fn java_suggestion() -> String {
    if cfg!(target_os = "macos") {
        "Install JDK 17: `brew install --cask temurin@17`".to_string()
    } else if cfg!(target_os = "linux") {
        "Install JDK 17 via your package manager (e.g., `apt install openjdk-17-jdk`).".to_string()
    } else {
        "Install JDK 17 from https://adoptium.net and set JAVA_HOME.".to_string()
    }
}

/// Get a human-readable installation suggestion for CMake.
#[must_use]
pub fn cmake_suggestion() -> String {
    if cfg!(target_os = "macos") {
        "Install CMake via Android SDK Manager (SDK Tools → CMake) or `brew install cmake`."
            .to_string()
    } else if cfg!(target_os = "linux") {
        "Install CMake via your package manager (e.g., `apt install cmake`) or Android SDK Manager."
            .to_string()
    } else {
        "Install CMake via Android SDK Manager (SDK Tools → CMake).".to_string()
    }
}
