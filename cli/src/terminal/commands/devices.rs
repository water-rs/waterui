//! `water devices` command implementation.

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::Result;

use crate::shell;
use crate::{header, line, warn};
use waterui_cli::{
    android::platform::AndroidPlatform,
    apple::{device::AppleDevice, platform::ApplePlatform},
    platform::Platform,
};

/// Target platform for device listing.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetPlatform {
    /// iOS devices and simulators.
    Ios,
    /// Android devices and emulators.
    Android,
    /// macOS (current machine).
    Macos,
    /// All platforms.
    All,
}

/// Arguments for the devices command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to list devices for.
    #[arg(short, long, value_enum, default_value = "all")]
    platform: TargetPlatform,
}

/// Run the devices command.
pub async fn run(args: Args) -> Result<()> {
    match args.platform {
        TargetPlatform::Ios => {
            list_ios_devices().await?;
        }
        TargetPlatform::Android => {
            list_android_devices().await?;
        }
        TargetPlatform::Macos => {
            list_macos_devices();
        }
        TargetPlatform::All => {
            list_ios_devices().await?;
            list_android_devices().await?;
            list_macos_devices();
        }
    }

    Ok(())
}

async fn list_ios_devices() -> Result<()> {
    let spinner = shell::spinner("Scanning iOS simulators...");

    let platform = ApplePlatform::ios_simulator();
    let ios_devices = platform.scan().await;

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    match ios_devices {
        Ok(devs) => {
            if !devs.is_empty() {
                header!("iOS Simulators");
            }

            for device in &devs {
                if let AppleDevice::Simulator(sim) = device {
                    let state_icon = if sim.state == "Booted" { "●" } else { "○" };
                    line!("  {} {} ({})", state_icon, sim.name, sim.udid);
                }
            }

            if devs.is_empty() {
                line!("  No iOS simulators available");
            }
        }
        Err(e) => {
            warn!("Failed to scan iOS simulators: {e}");
        }
    }

    Ok(())
}

async fn list_android_devices() -> Result<()> {
    let spinner = shell::spinner("Scanning Android devices...");

    let platform = AndroidPlatform::arm64();
    let android_devices = platform.scan().await;

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    match android_devices {
        Ok(devs) => {
            if !devs.is_empty() {
                header!("Android Devices");
            }

            for device in &devs {
                line!("  ● {} ({})", device.identifier(), device.abi());
            }

            if devs.is_empty() {
                line!("  No Android devices connected");
            }
        }
        Err(e) => {
            warn!("Failed to scan Android devices: {e}");
        }
    }

    Ok(())
}

fn list_macos_devices() {
    header!("macOS");
    line!("  ● Current Machine");
}
