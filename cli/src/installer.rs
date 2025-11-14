//! Install package using system package manager

use std::process::{Child, Command};

use color_eyre::eyre::{self, bail};
use tracing::info;
use which::which;

#[allow(dead_code)]
pub fn install_rust() -> eyre::Result<Child> {
    if which("rustc").is_ok() {
        bail!("Rust is already installed");
    }

    info!("Rust is not installed. Starting installation...");

    let child = std::process::Command::new("sh")
        .args([
            "-c",
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        ])
        .spawn()
        .expect("Failed to start rustup installer");
    Ok(child)
}

#[allow(dead_code)]
pub fn install_package(name: &'static str) -> eyre::Result<Child> {
    // install package takes time, run it on background thread

    if cfg!(target_os = "macos") {
        install_package_macos(name)
    } else if cfg!(target_os = "linux") {
        install_package_linux(name)
    } else {
        bail!("Unsupported OS for package installation")
    }
}

#[allow(dead_code)]
fn install_package_macos(name: &str) -> eyre::Result<Child> {
    // Use Homebrew to install package
    let child = std::process::Command::new("brew")
        .args(["install", name])
        .spawn()?;

    Ok(child)
}

// it can be super slow...complete it on background thread
#[allow(dead_code)]
fn install_package_linux(name: &str) -> eyre::Result<Child> {
    // linux has various package managers:
    // 1. apt (Debian/Ubuntu)
    // 2. dnf (Fedora)
    // 3. pacman (Arch)

    if which("apt").is_ok() {
        Ok(Command::new("sudo")
            .args(["apt", "install", "-y", name])
            .spawn()?)
    } else if which("dnf").is_ok() {
        Ok(Command::new("sudo")
            .args(["dnf", "install", "-y", name])
            .spawn()?)
    } else if which("pacman").is_ok() {
        Ok(Command::new("sudo")
            .args(["pacman", "-S", "--noconfirm", name])
            .spawn()?)
    } else {
        bail!("No supported package manager found to install {}", name);
    }
}
