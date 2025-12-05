//! Device and platform selection for interactive CLI.
//!
//! This module provides user-facing prompts for selecting:
//! - Backend/platform (Web, Apple, Android)
//! - Apple platform variant (macOS, iOS, iPadOS, tvOS, visionOS)
//! - Specific device or simulator

use std::time::Instant;

use atty::Stream;
use color_eyre::eyre::{Result, bail, eyre};
use dialoguer::{Select, theme::ColorfulTheme};
use tracing::debug;

use crate::ui;
use waterui_cli::{
    device::{self, AndroidSelection, DeviceInfo, DeviceKind, DevicePlatformFilter},
    output,
    platform::PlatformKind,
    project::Config,
};

// =============================================================================
// Backend/Platform Selection
// =============================================================================

/// Backend choice for project run.
#[derive(Clone, Copy)]
pub enum BackendChoice {
    /// Direct platform selection
    Platform(PlatformKind),
    /// Apple aggregate (user will select specific platform)
    AppleAggregate,
}

/// Prompt user to select a backend for a configured project.
///
/// # Errors
/// Returns an error if no backends are configured or selection fails.
pub fn prompt_for_backend(config: &Config) -> Result<BackendChoice> {
    let mut options: Vec<(BackendChoice, String)> = Vec::new();

    if config.backends.web.is_some() {
        options.push((
            BackendChoice::Platform(PlatformKind::Web),
            "Web Browser".to_string(),
        ));
    }

    if config.backends.swift.is_some() {
        options.push((
            BackendChoice::AppleAggregate,
            "Apple (macOS, iOS, iPadOS, tvOS, visionOS)".to_string(),
        ));
    }

    if config.backends.android.is_some() {
        options.push((
            BackendChoice::Platform(PlatformKind::Android),
            "Android".to_string(),
        ));
    }

    if options.is_empty() {
        bail!(
            "No runnable targets found. Please connect a device, start a simulator, \
             or enable a backend in your Water.toml."
        );
    }

    let default_index = default_backend_index(&options);
    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();

    let selection = if is_interactive_terminal() {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a backend to run")
            .items(&labels)
            .default(default_index)
            .interact()?
    } else {
        if !output::global_output_format().is_json() {
            ui::info(format!(
                "Non-interactive terminal detected; using {}.",
                options[default_index].1
            ));
        }
        default_index
    };

    Ok(options[selection].0)
}

/// Prompt user to select a backend in playground mode (no pre-configured backends).
///
/// # Errors
/// Returns an error if selection fails.
pub fn prompt_for_playground_backend() -> Result<BackendChoice> {
    let mut options: Vec<(BackendChoice, String)> = Vec::new();

    options.push((
        BackendChoice::Platform(PlatformKind::Web),
        "Web Browser".to_string(),
    ));

    #[cfg(target_os = "macos")]
    options.push((
        BackendChoice::AppleAggregate,
        "Apple (macOS, iOS, iPadOS, tvOS, visionOS)".to_string(),
    ));

    options.push((
        BackendChoice::Platform(PlatformKind::Android),
        "Android".to_string(),
    ));

    let default_index = default_backend_index(&options);
    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();

    let selection = if is_interactive_terminal() {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a platform to run")
            .items(&labels)
            .default(default_index)
            .interact()?
    } else {
        if !output::global_output_format().is_json() {
            ui::info(format!(
                "Non-interactive terminal detected; using {}.",
                options[default_index].1
            ));
        }
        default_index
    };

    Ok(options[selection].0)
}

/// Resolve a backend choice to a concrete platform.
///
/// # Errors
/// Returns an error if Apple platform selection fails.
pub fn resolve_backend_choice(choice: BackendChoice, config: &Config) -> Result<PlatformKind> {
    match choice {
        BackendChoice::Platform(platform) => Ok(platform),
        BackendChoice::AppleAggregate => prompt_for_apple_platform(config),
    }
}

/// Resolve a playground backend choice to a concrete platform.
///
/// # Errors
/// Returns an error if Apple platform selection fails.
pub fn resolve_playground_backend_choice(choice: BackendChoice) -> Result<PlatformKind> {
    match choice {
        BackendChoice::Platform(platform) => Ok(platform),
        BackendChoice::AppleAggregate => prompt_for_playground_apple_platform(),
    }
}

fn default_backend_index(options: &[(BackendChoice, String)]) -> usize {
    if !cfg!(target_os = "macos") {
        return 0;
    }

    options
        .iter()
        .position(|(choice, _)| matches!(choice, BackendChoice::AppleAggregate))
        .unwrap_or(0)
}

// =============================================================================
// Apple Platform Selection
// =============================================================================

/// Prompt user to select an Apple platform for a configured project.
///
/// # Errors
/// Returns an error if no Apple targets are available or selection fails.
pub fn prompt_for_apple_platform(config: &Config) -> Result<PlatformKind> {
    if config.backends.swift.is_none() {
        bail!("Apple backend is not configured for this project.");
    }

    let devices = scan_apple_devices()?;
    let options = build_apple_platform_options(&devices);

    if options.is_empty() {
        bail!("No Apple targets detected. Connect a device or install a simulator.");
    }

    if options.len() == 1 {
        return Ok(options[0].0);
    }

    select_from_options(&options, "Select an Apple target")
}

/// Prompt user to select an Apple platform in playground mode.
///
/// # Errors
/// Returns an error if selection fails.
pub fn prompt_for_playground_apple_platform() -> Result<PlatformKind> {
    let devices = scan_apple_devices()?;
    let options = build_apple_platform_options(&devices);

    select_from_options(&options, "Select an Apple platform")
}

fn build_apple_platform_options(devices: &[DeviceInfo]) -> Vec<(PlatformKind, String)> {
    let mut has_ios = false;
    let mut has_ipados = false;
    let mut has_tvos = false;
    let mut has_visionos = false;

    for device in devices {
        if let Ok(platform) = platform_from_device(device) {
            match platform {
                PlatformKind::Ios => has_ios = true,
                PlatformKind::Ipados => has_ipados = true,
                PlatformKind::Tvos => has_tvos = true,
                PlatformKind::Visionos => has_visionos = true,
                _ => {}
            }
        }
    }

    let mut options = vec![(PlatformKind::Macos, "Apple: macOS".to_string())];

    if has_ios {
        options.push((PlatformKind::Ios, "Apple: iOS".to_string()));
    }
    if has_ipados {
        options.push((PlatformKind::Ipados, "Apple: iPadOS".to_string()));
    }
    if has_tvos {
        options.push((PlatformKind::Tvos, "Apple: tvOS".to_string()));
    }
    if has_visionos {
        options.push((PlatformKind::Visionos, "Apple: visionOS".to_string()));
    }

    options
}

// =============================================================================
// Device Selection
// =============================================================================

/// Prompt user to select an Apple simulator for the given platform.
///
/// # Errors
/// Returns an error if no simulators are available or selection fails.
pub fn prompt_for_apple_device(platform: PlatformKind) -> Result<String> {
    let candidates = apple_simulator_candidates(platform)?;

    if candidates.len() == 1 {
        announce_auto_choice(&candidates[0].name, AutoSelectionReason::SingleCandidate);
        return Ok(candidates[0].name.clone());
    }

    if !is_interactive_terminal() {
        announce_auto_choice(&candidates[0].name, AutoSelectionReason::NonInteractive);
        return Ok(candidates[0].name.clone());
    }

    let options: Vec<String> = candidates
        .iter()
        .map(|d| {
            d.detail.as_ref().map_or_else(
                || d.name.clone(),
                |detail| format!("{} ({})", d.name, detail),
            )
        })
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a simulator")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(candidates[selection].name.clone())
}

/// Prompt user to select an Android device or emulator.
///
/// # Errors
/// Returns an error if no devices are available or selection fails.
pub fn prompt_for_android_device() -> Result<AndroidSelection> {
    let devices = scan_android_devices()?;

    let mut candidates: Vec<DeviceInfo> = devices
        .into_iter()
        .filter(|d| {
            d.raw_platform.as_deref() == Some("android-device")
                || d.raw_platform.as_deref() == Some("android-emulator")
        })
        .collect();

    if candidates.is_empty() {
        bail!("No Android devices or emulators detected. Connect a device or create an AVD.");
    }

    // Sort: physical devices first, then emulators, then by name
    candidates.sort_by(|a, b| {
        let kind_order = |k: &DeviceKind| match k {
            DeviceKind::Device => 0,
            DeviceKind::Emulator => 1,
            DeviceKind::Simulator => 2,
        };
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });

    if candidates.len() == 1 {
        let desc = describe_android_candidate(&candidates[0]);
        announce_auto_choice(&desc, AutoSelectionReason::SingleCandidate);
        return Ok(selection_from_device_info(&candidates[0]));
    }

    if !is_interactive_terminal() {
        let desc = describe_android_candidate(&candidates[0]);
        announce_auto_choice(&desc, AutoSelectionReason::NonInteractive);
        return Ok(selection_from_device_info(&candidates[0]));
    }

    let options: Vec<String> = candidates
        .iter()
        .map(|d| {
            let kind = match d.kind {
                DeviceKind::Device => "device",
                DeviceKind::Emulator => "emulator",
                DeviceKind::Simulator => "simulator",
            };
            d.state.as_ref().map_or_else(
                || format!("{} ({})", d.name, kind),
                |state| format!("{} ({}, {})", d.name, kind, state),
            )
        })
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select an Android device/emulator")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(selection_from_device_info(&candidates[selection]))
}

/// Resolve an Android device by name or identifier.
///
/// # Errors
/// Returns an error if the device is not found.
pub fn resolve_android_device(name: &str) -> Result<AndroidSelection> {
    let devices = device::list_devices_filtered(DevicePlatformFilter::Android)?;

    if let Some(device) = devices
        .iter()
        .find(|d| d.identifier == name || d.name == name)
    {
        return Ok(AndroidSelection {
            name: device.name.clone(),
            identifier: if device.kind == DeviceKind::Device {
                Some(device.identifier.clone())
            } else {
                None
            },
            kind: device.kind.clone(),
        });
    }

    bail!(
        "Android device or emulator '{}' not found. Run `water devices` to list available targets.",
        name
    );
}

// =============================================================================
// Helpers
// =============================================================================

fn scan_apple_devices() -> Result<Vec<DeviceInfo>> {
    let mut spinner = ui::spinner("Scanning Apple devices...");
    let scan_started = Instant::now();
    debug!("Starting Apple device scan");

    match device::list_devices_filtered(DevicePlatformFilter::Apple) {
        Ok(list) => {
            if let Some(guard) = spinner.take() {
                guard.finish();
            }
            debug!(
                "Apple device scan completed in {:?} with {} device(s)",
                scan_started.elapsed(),
                list.len()
            );
            Ok(list)
        }
        Err(err) => {
            if let Some(guard) = spinner.take() {
                guard.finish();
            }
            debug!(
                "Apple device scan failed after {:?}: {err:?}",
                scan_started.elapsed()
            );
            Err(err)
        }
    }
}

fn scan_android_devices() -> Result<Vec<DeviceInfo>> {
    let mut spinner = ui::spinner("Scanning Android devices...");
    let scan_started = Instant::now();
    debug!("Starting Android device scan");

    match device::list_devices_filtered(DevicePlatformFilter::Android) {
        Ok(list) => {
            if let Some(guard) = spinner.take() {
                guard.finish();
            }
            debug!(
                "Android device scan completed in {:?} with {} device(s)",
                scan_started.elapsed(),
                list.len()
            );
            Ok(list)
        }
        Err(err) => {
            if let Some(guard) = spinner.take() {
                guard.finish();
            }
            debug!(
                "Android device scan failed after {:?}: {err:?}",
                scan_started.elapsed()
            );
            Err(err)
        }
    }
}

fn apple_simulator_candidates(platform: PlatformKind) -> Result<Vec<DeviceInfo>> {
    let raw_platform = apple_simulator_platform_id(platform);
    if raw_platform.is_empty() {
        return Ok(Vec::new());
    }

    let devices = scan_apple_devices()?;

    let mut candidates: Vec<DeviceInfo> = devices
        .into_iter()
        .filter(|d| d.kind == DeviceKind::Simulator)
        .filter(|d| d.raw_platform.as_deref() == Some(raw_platform))
        .collect();

    if candidates.is_empty() {
        bail!(
            "No simulators found for {}. Install one using Xcode's Devices window.",
            apple_platform_display_name(platform)
        );
    }

    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(candidates)
}

fn select_from_options(options: &[(PlatformKind, String)], prompt: &str) -> Result<PlatformKind> {
    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();

    let selection = if is_interactive_terminal() {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .items(&labels)
            .default(0)
            .interact()?
    } else {
        if !output::global_output_format().is_json() {
            ui::info(format!(
                "Non-interactive terminal detected; using {}.",
                options[0].1
            ));
        }
        0
    };

    Ok(options[selection].0)
}

/// Derive platform from device info.
///
/// # Errors
/// Returns an error if the platform is not recognized.
pub fn platform_from_device(device: &DeviceInfo) -> Result<PlatformKind> {
    match device.platform.as_str() {
        "Web" => Ok(PlatformKind::Web),
        "macOS" => Ok(PlatformKind::Macos),
        p if p.starts_with("iOS") => Ok(PlatformKind::Ios),
        p if p.starts_with("iPadOS") => Ok(PlatformKind::Ipados),
        p if p.starts_with("watchOS") => Ok(PlatformKind::Watchos),
        p if p.starts_with("tvOS") => Ok(PlatformKind::Tvos),
        p if p.starts_with("visionOS") => Ok(PlatformKind::Visionos),
        "Android" => Ok(PlatformKind::Android),
        _ => Err(eyre!("Unsupported platform: {}", device.platform)),
    }
}

/// Find a device by name or identifier.
pub fn find_device<'a>(devices: &'a [DeviceInfo], query: &str) -> Option<&'a DeviceInfo> {
    devices
        .iter()
        .find(|device| device.identifier == query || device.name == query)
}

fn selection_from_device_info(info: &DeviceInfo) -> AndroidSelection {
    AndroidSelection {
        name: info.name.clone(),
        identifier: if info.kind == DeviceKind::Device {
            Some(info.identifier.clone())
        } else {
            None
        },
        kind: info.kind.clone(),
    }
}

fn describe_android_candidate(info: &DeviceInfo) -> String {
    let kind = match info.kind {
        DeviceKind::Device => "device",
        DeviceKind::Emulator => "emulator",
        DeviceKind::Simulator => "simulator",
    };
    match &info.state {
        Some(state) if !state.is_empty() => format!("{} ({kind}, {state})", info.name),
        _ => format!("{} ({kind})", info.name),
    }
}

/// Get the simulator platform identifier for device filtering.
pub const fn apple_simulator_platform_id(platform: PlatformKind) -> &'static str {
    match platform {
        PlatformKind::Ios | PlatformKind::Ipados => "com.apple.platform.iphonesimulator",
        PlatformKind::Watchos => "com.apple.platform.watchsimulator",
        PlatformKind::Tvos => "com.apple.platform.appletvsimulator",
        PlatformKind::Visionos => "com.apple.platform.visionossimulator",
        PlatformKind::Macos | PlatformKind::Android | PlatformKind::Web => "",
    }
}

/// Get the display name for an Apple platform.
pub const fn apple_platform_display_name(platform: PlatformKind) -> &'static str {
    match platform {
        PlatformKind::Ios => "iOS",
        PlatformKind::Ipados => "iPadOS",
        PlatformKind::Watchos => "watchOS",
        PlatformKind::Tvos => "tvOS",
        PlatformKind::Visionos => "visionOS",
        _ => "Apple",
    }
}

#[derive(Copy, Clone)]
enum AutoSelectionReason {
    SingleCandidate,
    NonInteractive,
}

fn announce_auto_choice(name: &str, reason: AutoSelectionReason) {
    if output::global_output_format().is_json() {
        return;
    }

    let prefix = match reason {
        AutoSelectionReason::SingleCandidate => "Using",
        AutoSelectionReason::NonInteractive => "Non-interactive terminal detected; using",
    };

    ui::info(format!("{prefix} {name}."));
}

fn is_interactive_terminal() -> bool {
    atty::is(Stream::Stdin) && atty::is(Stream::Stdout)
}
