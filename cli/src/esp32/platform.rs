//! ESP32 platform build, flash, emulation, and package utilities.
//!
//! The generated harness crate pins its own `esp` Rust toolchain via
//! `rust-toolchain.toml` and selects the Xtensa target via `.cargo/config.toml`,
//! so builds simply run `cargo build` inside the harness directory with the
//! Xtensa GCC and clang library paths exported.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use color_eyre::eyre::{self, Context as _, bail, eyre};
use smol::{fs, unblock};
use tracing::info;

use crate::{
    build::BuildOptions,
    device::Artifact,
    esp32::backend::Esp32Backend,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::{command, run_command_os, which},
};

const ESP32_INIT_HINT: &str = "water run --platform esp32s3";

/// Flash size passed to `espflash` when merging firmware images.
///
/// This matches the generated `sdkconfig.defaults` (`CONFIG_ESPTOOLPY_FLASHSIZE_8MB`)
/// and the generated `partitions.csv` app partition.
const ESP32_FLASH_SIZE: &str = "8mb";

/// USB vendor IDs commonly found on ESP32 development boards.
///
/// `0x303a` is Espressif's native USB (USB-Serial-JTAG); the others are the
/// `CP210x`, `CH34x`, and FTDI UART bridges used on classic devkits.
const ESP_USB_VENDOR_IDS: [u16; 4] = [0x303a, 0x10c4, 0x1a86, 0x0403];

/// Check if a platform is supported by the ESP32 backend.
#[must_use]
pub const fn is_esp32_platform(platform: TargetPlatform) -> bool {
    matches!(platform, TargetPlatform::Esp32S3)
}

/// Summary of a host serial port for device listing and board auto-detection.
#[derive(Debug, Clone)]
pub struct SerialPortSummary {
    /// Host path of the serial port (e.g. `/dev/cu.usbmodem101`).
    pub port_name: String,
    /// USB vendor/product identifiers when the port is a USB device.
    pub usb_vid_pid: Option<(u16, u16)>,
    /// USB product string when reported by the device.
    pub product: Option<String>,
    /// Whether the USB vendor matches a known ESP32 board or UART bridge.
    pub likely_esp: bool,
}

/// List host serial ports, marking ports that look like ESP32 boards.
///
/// # Errors
/// Returns an error when the host serial subsystem cannot be enumerated.
pub async fn scan_serial_ports() -> eyre::Result<Vec<SerialPortSummary>> {
    let ports = unblock(serialport::available_ports)
        .await
        .wrap_err("Failed to enumerate serial ports")?;

    Ok(ports
        .into_iter()
        .map(|port| {
            let (usb_vid_pid, product) = match port.port_type {
                serialport::SerialPortType::UsbPort(usb) => (Some((usb.vid, usb.pid)), usb.product),
                _ => (None, None),
            };
            let likely_esp = usb_vid_pid.is_some_and(|(vid, _)| ESP_USB_VENDOR_IDS.contains(&vid));
            SerialPortSummary {
                port_name: port.port_name,
                usb_vid_pid,
                product,
                likely_esp,
            }
        })
        .collect())
}

/// Pick the serial port of a connected ESP32 board, if any.
///
/// Espressif's native USB vendor ID and the usual UART bridges are
/// considered; on hosts exposing both `tty` and `cu` nodes the callout
/// (`cu`) node is preferred.
///
/// # Errors
/// Returns an error when the host serial subsystem cannot be enumerated.
pub async fn detect_esp_serial_port() -> eyre::Result<Option<String>> {
    let mut candidates: Vec<SerialPortSummary> = scan_serial_ports()
        .await?
        .into_iter()
        .filter(|port| port.likely_esp)
        .collect();
    candidates.sort_by_key(|port| {
        let is_callout = port.port_name.contains("/cu.");
        (!is_callout, port.port_name.clone())
    });
    Ok(candidates.into_iter().next().map(|port| port.port_name))
}

fn home_dir() -> eyre::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| eyre!("Failed to resolve the user home directory"))
}

/// Find the newest versioned subdirectory of `base` containing `relative`.
fn newest_toolchain_subpath(base: &Path, relative: &Path) -> Option<PathBuf> {
    let mut versions: Vec<PathBuf> = std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(relative).is_dir())
        .collect();
    versions.sort();
    versions.pop().map(|path| path.join(relative))
}

fn espup_component_dir(component: &str, relative: &Path, what: &str) -> eyre::Result<PathBuf> {
    let base = home_dir()?.join(".rustup/toolchains/esp").join(component);
    newest_toolchain_subpath(&base, relative).ok_or_else(|| {
        eyre!(
            "{what} not found under {}. Install the Espressif Rust toolchain with `espup install`.",
            base.display()
        )
    })
}

/// Environment variables required to drive the Espressif Rust toolchain.
///
/// Prepends the Xtensa GCC `bin` directory to `PATH` and points
/// `LIBCLANG_PATH` at the Espressif clang libraries, both discovered under
/// `~/.rustup/toolchains/esp` without assuming a toolchain version.
///
/// # Errors
/// Returns an error when the Espressif toolchain components are not installed.
pub fn esp_toolchain_envs() -> eyre::Result<Vec<(String, OsString)>> {
    let gcc_bin = espup_component_dir(
        "xtensa-esp-elf",
        Path::new("xtensa-esp-elf/bin"),
        "Xtensa GCC toolchain",
    )?;
    let libclang = espup_component_dir(
        "xtensa-esp32-elf-clang",
        Path::new("esp-clang/lib"),
        "Espressif clang libraries",
    )?;

    let mut paths = vec![gcc_bin];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    let path_value =
        std::env::join_paths(paths).wrap_err("Failed to compose PATH for the ESP toolchain")?;

    Ok(vec![
        ("PATH".to_string(), path_value),
        ("LIBCLANG_PATH".to_string(), libclang.into_os_string()),
    ])
}

fn esp32_chip(project: &Project) -> String {
    project
        .esp32_backend()
        .cloned()
        .unwrap_or_default()
        .chip()
        .to_string()
}

fn esp32_target_triple(project: &Project) -> String {
    format!("xtensa-{}-espidf", esp32_chip(project))
}

async fn espflash_path() -> eyre::Result<PathBuf> {
    which("espflash")
        .await
        .map_err(|_| eyre!("espflash not found. Install it with `cargo install espflash`."))
}

fn command_failure_details(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.trim().is_empty() {
        stdout.to_string()
    } else {
        stderr.to_string()
    }
}

/// Build the ESP32 firmware ELF for the configured chip.
///
/// Runs `cargo build` inside the generated harness directory so its
/// `rust-toolchain.toml` (channel `esp`) and `.cargo/config.toml` (Xtensa
/// target, `build-std`, ESP-IDF environment) take effect.
///
/// # Errors
/// Returns an error if the harness is missing, the Espressif toolchain is not
/// installed, or Cargo fails.
pub async fn build_esp32(project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
    let backend_path = project.backend_path::<Esp32Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.backend_target_dir("esp32").await?;

    if !cargo_toml.exists() {
        bail!(
            "ESP32 backend not found at {}. Run `{ESP32_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    let profile = if options.is_release() {
        "release"
    } else {
        "debug"
    };

    let mut cargo = smol::process::Command::new("cargo");
    let cargo = command(&mut cargo);
    cargo.current_dir(&backend_path);
    cargo.arg("build");
    cargo.arg("--target-dir").arg(&backend_target_dir);
    if let Some(sccache_path) = options.sccache_path() {
        cargo.env("RUSTC_WRAPPER", sccache_path);
    }
    for (key, value) in esp_toolchain_envs()? {
        cargo.env(key, value);
    }
    if options.is_release() {
        cargo.arg("--release");
    }

    let output = cargo.output().await?;
    if !output.status.success() {
        bail!(
            "Failed to build ESP32 firmware with cargo (status {}):\n{}",
            output.status,
            command_failure_details(&output)
        );
    }

    Ok(backend_target_dir
        .join(esp32_target_triple(project))
        .join(profile))
}

/// Resolve the built ESP32 firmware ELF path for the given profile.
///
/// # Errors
/// Returns an error if neither the canonical nor underscored firmware binary can be found.
pub async fn built_esp32_binary_path(project: &Project, profile: &str) -> eyre::Result<PathBuf> {
    let target_dir = project
        .backend_target_dir("esp32")
        .await?
        .join(esp32_target_triple(project))
        .join(profile);
    let binary_name = project.esp32_backend_crate_name();
    let binary_path = target_dir.join(binary_name.as_str());
    if binary_path.exists() {
        return Ok(binary_path);
    }

    let underscored_path = target_dir.join(binary_name.as_str().replace('-', "_"));
    if underscored_path.exists() {
        return Ok(underscored_path);
    }

    bail!(
        "Built ESP32 firmware not found at {} or {}",
        binary_path.display(),
        underscored_path.display()
    );
}

/// Build, then flash and monitor the firmware on a board, or emulate it.
///
/// `device` selects the run target: `Some("qemu")` forces the QEMU emulator,
/// `Some(port)` flashes the given serial port, and `None` flashes the first
/// connected ESP32 board, falling back to QEMU when no board is connected but
/// an ESP32-capable `qemu-system-xtensa` is installed.
///
/// # Errors
/// Returns an error when building, flashing, or emulation fails, or when
/// neither a board nor QEMU is available.
pub async fn run_esp32(
    project: &Project,
    options: BuildOptions,
    device: Option<&str>,
) -> eyre::Result<()> {
    let profile = if options.is_release() {
        "release"
    } else {
        "debug"
    };
    build_esp32(project, options).await?;
    let elf = built_esp32_binary_path(project, profile).await?;

    match device {
        Some("qemu") => qemu_esp32(project, &elf).await,
        Some(port) => flash_and_monitor(project, &elf, Some(port)).await,
        None => {
            if let Some(port) = detect_esp_serial_port().await? {
                info!("Flashing ESP32 board on {port}");
                return flash_and_monitor(project, &elf, Some(&port)).await;
            }
            if locate_qemu().await.is_some() {
                info!("No ESP32 board connected; running under QEMU");
                return qemu_esp32(project, &elf).await;
            }
            bail!(
                "No ESP32 board connected and no ESP32-capable QEMU installed.\n\
                 Connect a board (see `water devices --platform esp32`), pass --device <port>,\n\
                 or install Espressif's QEMU fork (qemu-system-xtensa with the esp32s3 machine)."
            )
        }
    }
}

async fn flash_and_monitor(project: &Project, elf: &Path, port: Option<&str>) -> eyre::Result<()> {
    let backend_path = project.backend_path::<Esp32Backend>();
    let espflash = espflash_path().await?;

    let mut espflash_cmd = smol::process::Command::new(espflash);
    espflash_cmd
        .current_dir(&backend_path)
        .arg("flash")
        .arg("--partition-table")
        .arg(backend_path.join("partitions.csv"))
        .arg("--monitor");
    if let Some(port) = port {
        espflash_cmd.arg("--port").arg(port);
    }
    espflash_cmd
        .arg(elf)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    let status = espflash_cmd.status().await?;
    if !status.success() {
        bail!("espflash flash failed with status {status}");
    }
    Ok(())
}

/// Locate an ESP32-capable QEMU binary.
///
/// Prefers the Espressif QEMU fork at `~/.local/esp-qemu/qemu/bin/qemu-system-xtensa`,
/// falling back to `qemu-system-xtensa` on `PATH`.
async fn locate_qemu() -> Option<PathBuf> {
    if let Ok(home) = home_dir() {
        let bundled = home.join(".local/esp-qemu/qemu/bin/qemu-system-xtensa");
        if bundled.exists() {
            return Some(bundled);
        }
    }
    which("qemu-system-xtensa").await.ok()
}

/// eFuse image for QEMU: ADC calibration version 1 (BLK2 word 4 bits 0..3).
///
/// Without it the firmware hangs at startup in hardware ADC self-calibration,
/// which QEMU does not emulate; version 1 makes startup read the (zeroed)
/// calibration codes from eFuse instead.
fn qemu_efuse_image() -> Vec<u8> {
    let mut data = vec![0u8; 1024];
    data[64] = 0x01;
    data
}

/// Run the built firmware ELF under Espressif's QEMU, streaming serial output.
///
/// Builds a merged flash image with `espflash save-image`, generates the
/// eFuse image QEMU needs to skip unemulated ADC self-calibration, and runs
/// `qemu-system-xtensa` with the chip's machine model.
///
/// # Errors
/// Returns an error when QEMU or espflash is missing, image generation fails,
/// or the emulator exits with a failure status.
pub async fn qemu_esp32(project: &Project, elf: &Path) -> eyre::Result<()> {
    let qemu = locate_qemu().await.ok_or_else(|| {
        eyre!(
            "ESP32-capable QEMU not found. Expected ~/.local/esp-qemu/qemu/bin/qemu-system-xtensa \
             or qemu-system-xtensa on PATH (Espressif fork with the esp32s3 machine)."
        )
    })?;
    let backend_path = project.backend_path::<Esp32Backend>();
    let chip = esp32_chip(project);

    let staging = tempfile::Builder::new()
        .prefix("waterui-esp32-qemu")
        .tempdir()
        .wrap_err("Failed to create QEMU staging directory")?;
    let flash_image = staging.path().join("flash.bin");
    let efuse_image = staging.path().join("efuse.bin");

    save_flash_image(&backend_path, &chip, elf, &flash_image).await?;
    fs::write(&efuse_image, qemu_efuse_image()).await?;

    let mut qemu_cmd = smol::process::Command::new(qemu);
    qemu_cmd
        .arg("-nographic")
        .arg("-machine")
        .arg(&chip)
        .arg("-drive")
        .arg(format!("file={},if=mtd,format=raw", flash_image.display()))
        .arg("-drive")
        .arg(format!(
            "file={},if=none,format=raw,id=efuse",
            efuse_image.display()
        ))
        .arg("-global")
        .arg(format!(
            "driver=nvram.{chip}.efuse,property=drive,value=efuse"
        ))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    let status = qemu_cmd.status().await?;
    if !status.success() {
        bail!("qemu-system-xtensa exited with status {status}");
    }
    Ok(())
}

async fn save_flash_image(
    backend_path: &Path,
    chip: &str,
    elf: &Path,
    image_path: &Path,
) -> eyre::Result<()> {
    let espflash = espflash_path().await?;
    let mut save = smol::process::Command::new(espflash);
    let save = command(&mut save);
    save.current_dir(backend_path)
        .arg("save-image")
        .arg("--chip")
        .arg(chip)
        .arg("--merge")
        .arg("--flash-size")
        .arg(ESP32_FLASH_SIZE)
        .arg("--partition-table")
        .arg(backend_path.join("partitions.csv"))
        .arg(elf)
        .arg(image_path);

    let output = save.output().await?;
    if !output.status.success() {
        bail!(
            "espflash save-image failed with status {}:\n{}",
            output.status,
            command_failure_details(&output)
        );
    }
    Ok(())
}

/// Package the built firmware as a flashable merged image.
///
/// # Errors
/// Returns an error when the built ELF is missing or image merging fails.
pub async fn package_esp32(project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let elf = built_esp32_binary_path(project, profile).await?;
    let backend_path = project.backend_path::<Esp32Backend>();
    let chip = esp32_chip(project);

    let dist_dir = backend_path.join("dist");
    fs::create_dir_all(&dist_dir).await?;
    let image_path = dist_dir.join(format!("{}.bin", project.esp32_backend_crate_name()));
    save_flash_image(&backend_path, &chip, &elf, &image_path).await?;

    Ok(Artifact::new(project.bundle_identifier(), image_path))
}

/// Clean Cargo build artifacts and packaged images for the ESP32 harness.
///
/// # Errors
/// Returns an error if `cargo clean` fails or the dist directory cannot be removed.
pub async fn clean_esp32(project: &Project) -> eyre::Result<()> {
    let backend_path = project.backend_path::<Esp32Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.backend_target_dir("esp32").await?;

    if !cargo_toml.exists() {
        return Ok(());
    }

    let args: Vec<OsString> = vec![
        "clean".into(),
        "--manifest-path".into(),
        cargo_toml.as_os_str().to_owned(),
        "--target-dir".into(),
        backend_target_dir.as_os_str().to_owned(),
    ];
    run_command_os("cargo", args).await?;

    let dist_dir = backend_path.join("dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir).await?;
    }
    Ok(())
}
