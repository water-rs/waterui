//! ESP32 platform build, flash, emulation, and package utilities.
//!
//! The generated harness crate pins its own `esp` Rust toolchain via
//! `rust-toolchain.toml` and selects the Xtensa target via `.cargo/config.toml`,
//! so builds simply run `cargo build` inside the harness directory with the
//! Xtensa GCC and clang library paths exported.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Context as _, bail, eyre};
use smol::fs;
#[cfg(feature = "esp32")]
use smol::unblock;
use tracing::info;

use crate::{
    build::{BuildOptions, BuildProgress, BuiltTarget, SharedExecutable},
    device::Artifact,
    esp32::{backend::Esp32Backend, chip::Esp32Chip},
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::command,
};

const ESP32_INIT_HINT: &str = "water run --platform esp32s3";

/// USB vendor IDs commonly found on ESP32 development boards.
///
/// `0x303a` is Espressif's native USB (USB-Serial-JTAG); the others are the
/// `CP210x`, `CH34x`, and FTDI UART bridges used on classic devkits.
#[cfg(feature = "esp32")]
const ESP_USB_VENDOR_IDS: [u16; 4] = [0x303a, 0x10c4, 0x1a86, 0x0403];

/// Check if a platform is supported by the ESP32 backend.
#[must_use]
pub const fn is_esp32_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::Esp32S3 | TargetPlatform::Esp32C3 | TargetPlatform::Esp32P4
    )
}

/// Summary of a host serial port for device listing and board auto-detection.
#[cfg(feature = "esp32")]
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
#[cfg(feature = "esp32")]
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
#[cfg(feature = "esp32")]
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

fn home_dir(host: &crate::toolchain::Host) -> eyre::Result<PathBuf> {
    host.home_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| eyre!("Failed to resolve the user home directory"))
}

/// Find the newest versioned subdirectory of `base` containing `relative`.
pub(crate) fn newest_toolchain_subpath(base: &Path, relative: &Path) -> Option<PathBuf> {
    let mut versions: Vec<PathBuf> = std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(relative).is_dir())
        .collect();
    versions.sort();
    versions.pop().map(|path| path.join(relative))
}

fn espup_component_dir(
    host: &crate::toolchain::Host,
    component: &str,
    relative: &Path,
    what: &str,
) -> eyre::Result<PathBuf> {
    let base = home_dir(host)?
        .join(".rustup/toolchains/esp")
        .join(component);
    newest_toolchain_subpath(&base, relative).ok_or_else(|| {
        eyre!(
            "{what} not found under {}. Install the Espressif Rust toolchain with `espup install`.",
            base.display()
        )
    })
}

/// Locate the GCC `bin` directory for `chip`'s architecture.
///
/// Xtensa GCC ships inside the espup `esp` toolchain
/// (`~/.rustup/toolchains/esp/xtensa-esp-elf/...`); the RISC-V GCC is installed
/// by ESP-IDF under `~/.espressif/tools/riscv32-esp-elf/...`. Both are
/// version-discovered rather than pinned.
fn gcc_bin_dir(host: &crate::toolchain::Host, chip: Esp32Chip) -> eyre::Result<PathBuf> {
    let component = chip.gcc_component();
    match chip.arch() {
        crate::esp32::chip::Esp32Arch::Xtensa => espup_component_dir(
            host,
            component.component,
            Path::new(component.bin_subpath),
            component.what,
        ),
        crate::esp32::chip::Esp32Arch::RiscV => {
            let base = home_dir(host)?
                .join(".espressif/tools")
                .join(component.component);
            newest_toolchain_subpath(&base, Path::new(component.bin_subpath)).ok_or_else(|| {
                eyre!(
                    "{} not found under {}. Install it with ESP-IDF tools (`idf_tools.py install`) \
                     or by building an esp-idf-svc project once.",
                    component.what,
                    base.display()
                )
            })
        }
    }
}

/// Environment variables required to drive the Espressif Rust toolchain for
/// `chip`.
///
/// Prepends the chip architecture's GCC `bin` directory to `PATH` and points
/// `LIBCLANG_PATH` at the Espressif clang libraries (shared across
/// architectures), discovered under the espup `esp` toolchain without assuming
/// a toolchain version.
///
/// # Errors
/// Returns an error when the Espressif toolchain components are not installed.
pub fn esp_toolchain_envs(
    host: &crate::toolchain::Host,
    chip: Esp32Chip,
) -> eyre::Result<Vec<(String, OsString)>> {
    let gcc_bin = gcc_bin_dir(host, chip)?;
    let libclang = espup_component_dir(
        host,
        "xtensa-esp32-elf-clang",
        Path::new("esp-clang/lib"),
        "Espressif clang libraries",
    )?;

    let mut paths = vec![gcc_bin];
    paths.extend(host.path_entries());
    let path_value =
        std::env::join_paths(paths).wrap_err("Failed to compose PATH for the ESP toolchain")?;

    Ok(vec![
        ("PATH".to_string(), path_value),
        ("LIBCLANG_PATH".to_string(), libclang.into_os_string()),
    ])
}

/// Resolve the configured chip for `project`'s ESP32 backend.
fn esp32_chip(project: &Project) -> eyre::Result<Esp32Chip> {
    project
        .esp32_config()
        .cloned()
        .unwrap_or_default()
        .resolved_chip()
}

async fn espflash_path(host: &crate::toolchain::Host) -> eyre::Result<PathBuf> {
    host.which("espflash")
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
pub async fn build_esp32(project: &Project, options: BuildOptions) -> eyre::Result<BuiltTarget> {
    let backend_path = project.backend_path::<Esp32Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.toolchain_target_dir("esp32").await?;

    if !cargo_toml.exists() {
        bail!(
            "ESP32 backend not found at {}. Run `{ESP32_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    let chip = esp32_chip(project)?;
    let host = project.host();

    let mut cargo = host.command("cargo");
    cargo.current_dir(&backend_path);
    cargo.arg("build");
    // The harness lives outside the project, so Cargo's discovery never
    // reaches `<project>/.cargo/config.toml`; the args restore the project's
    // hierarchy. The harness's own `.cargo/config.toml` (Xtensa target,
    // ESP-IDF env) is listed last and keeps precedence.
    cargo.args(crate::toolchain::cargo_project_config::cargo_config_args(
        project.root(),
        &backend_path,
    )?);
    cargo.arg("--message-format=json-render-diagnostics");
    cargo.arg("--target-dir").arg(&backend_target_dir);
    crate::build::configure_generated_crate_compilation(&mut cargo);
    if let Some(sccache_path) = options.sccache_path() {
        crate::toolchain::sccache::configure_compilation_cache(host, &mut cargo, sccache_path)
            .await?;
    }
    for (key, value) in esp_toolchain_envs(host, chip)? {
        cargo.env(key, value);
    }
    if options.is_release() {
        cargo.arg("--release");
    }
    // Piped stdio strips rustc diagnostics of their colors; restore cargo's
    // coloring while the terminal renders the output.
    if host.std_output() && host.env("CARGO_TERM_COLOR").is_none() {
        cargo.env("CARGO_TERM_COLOR", "always");
    }

    let output = crate::build::command_output_with_progress(
        &mut cargo,
        options.progress().cloned(),
        host.std_output(),
    )
    .await?;
    if !output.status.success() {
        let details = command_failure_details(&output);
        // A live-rendered stream is tailed rather than re-dumped in full.
        let details = if options
            .progress()
            .is_some_and(BuildProgress::shows_all_lines)
        {
            crate::build::output_tail(&details)
        } else {
            details
        };
        bail!(
            "Failed to build ESP32 firmware with cargo (status {}):\n{}",
            output.status,
            details
        );
    }

    // The ELF path comes from cargo's own artifact report — the shared
    // toolchain target directory hosts other projects' builds, so a bare
    // `<profile>/<name>` lookup is not evidence the file is this project's.
    let crate_name = project.esp32_backend_crate_name();
    let artifact = crate::build::reported_artifact(
        &output.stdout,
        &backend_path,
        crate::build::CargoTarget::Binary(crate_name.as_str()),
        None,
    )
    .map_err(|error| eyre!("failed to resolve the built ESP32 firmware: {error}"))?;
    let profile_dir = artifact.parent().ok_or_else(|| {
        eyre!(
            "ESP32 firmware artifact has no profile directory: {}",
            artifact.display()
        )
    })?;
    Ok(BuiltTarget {
        profile_dir: profile_dir.to_path_buf(),
        artifact: artifact.clone(),
        executable: Some(SharedExecutable::unlocked(artifact)),
        entry_binary: None,
        shared_runtime: None,
        app_library: crate::build::app_library_artifact(
            &output.stdout,
            &project.root().join("Cargo.toml"),
        )?,
        cef_helper: None,
    })
}

/// Build, then flash and monitor the firmware on a board, or emulate it.
///
/// `device` selects the run target: `Some("qemu")` forces the QEMU emulator,
/// `Some(port)` flashes the given serial port, and `None` flashes the first
/// connected ESP32 board, falling back to QEMU when no board is connected but
/// the chip's QEMU emulator is installed.
///
/// # Errors
/// Returns an error when building, flashing, or emulation fails, or when
/// neither a board nor QEMU is available.
pub async fn run_esp32(
    project: &Project,
    options: BuildOptions,
    device: Option<&str>,
) -> eyre::Result<()> {
    let chip = esp32_chip(project)?;
    let elf = build_esp32(project, options).await?;

    match device {
        Some("qemu") => qemu_esp32(project, chip, &elf.artifact).await,
        Some(port) => flash_and_monitor(project, &elf.artifact, Some(port)).await,
        None => {
            #[cfg(feature = "esp32")]
            {
                if let Some(port) = detect_esp_serial_port().await? {
                    info!("Flashing ESP32 board on {port}");
                    return flash_and_monitor(project, &elf.artifact, Some(&port)).await;
                }
            }
            if locate_qemu(project.host(), chip).await.is_some() {
                info!("No ESP32 board connected; running under QEMU");
                return qemu_esp32(project, chip, &elf.artifact).await;
            }
            bail!(
                "No ESP32 board connected and no QEMU for {chip_id} installed.\n\
                 Connect a board (see `water devices --platform esp32`), pass --device <port>,\n\
                 or install Espressif's QEMU fork ({qemu} with the {machine} machine).",
                chip_id = chip.id(),
                qemu = chip.qemu_binary(),
                machine = chip.qemu_machine(),
            );
        }
    }
}

async fn flash_and_monitor(project: &Project, elf: &Path, port: Option<&str>) -> eyre::Result<()> {
    let backend_path = project.backend_path::<Esp32Backend>();
    let espflash = espflash_path(project.host()).await?;

    // `--monitor` attaches espflash's serial console to the user's terminal.
    let mut espflash_cmd =
        smol::process::Command::from(project.host().interactive_command(&espflash));
    espflash_cmd
        .current_dir(&backend_path)
        .arg("flash")
        .arg("--partition-table")
        .arg(backend_path.join("partitions.csv"))
        .arg("--monitor");
    if let Some(port) = port {
        espflash_cmd.arg("--port").arg(port);
    }
    espflash_cmd.arg(elf).kill_on_drop(true);

    let status = espflash_cmd.status().await?;
    if !status.success() {
        bail!("espflash flash failed with status {status}");
    }
    Ok(())
}

/// Locate the QEMU binary that emulates `chip`'s architecture.
///
/// Prefers the Espressif QEMU fork bundled under `~/.local/esp-qemu/qemu/bin`,
/// falling back to the binary on `PATH`.
async fn locate_qemu(host: &crate::toolchain::Host, chip: Esp32Chip) -> Option<PathBuf> {
    let binary = chip.qemu_binary();
    if let Some(home) = host.home_dir() {
        let bundled = home.join(".local/esp-qemu/qemu/bin").join(binary);
        if bundled.exists() {
            return Some(bundled);
        }
    }
    host.which(binary).await.ok()
}

/// eFuse image for QEMU: ADC calibration version 1 (BLK2 word 4 bits 0..3).
///
/// Without it Xtensa firmware hangs at startup in hardware ADC
/// self-calibration, which QEMU does not emulate; version 1 makes startup read
/// the (zeroed) calibration codes from eFuse instead.
fn qemu_efuse_image() -> Vec<u8> {
    let mut data = vec![0u8; 1024];
    data[64] = 0x01;
    data
}

/// Run the built firmware ELF under the chip's QEMU, streaming serial output.
///
/// Builds a merged flash image with `espflash save-image` and runs the chip's
/// QEMU with its machine model. Xtensa chips additionally need an eFuse image
/// to skip unemulated ADC self-calibration; RISC-V chips boot without it.
///
/// # Errors
/// Returns an error when QEMU or espflash is missing, image generation fails,
/// or the emulator exits with a failure status.
pub async fn qemu_esp32(project: &Project, chip: Esp32Chip, elf: &Path) -> eyre::Result<()> {
    let host = project.host();
    let qemu = locate_qemu(host, chip).await.ok_or_else(|| {
        eyre!(
            "QEMU for {} not found. Expected ~/.local/esp-qemu/qemu/bin/{binary} \
             or {binary} on PATH (Espressif fork with the {machine} machine).",
            chip.id(),
            binary = chip.qemu_binary(),
            machine = chip.qemu_machine(),
        )
    })?;
    let backend_path = project.backend_path::<Esp32Backend>();

    let staging = tempfile::Builder::new()
        .prefix("waterui-esp32-qemu")
        .tempdir_in(host.temp_dir())
        .wrap_err("Failed to create QEMU staging directory")?;
    let flash_image = staging.path().join("flash.bin");

    save_flash_image(host, &backend_path, chip, elf, &flash_image).await?;

    // `-nographic` puts the firmware's serial console on the user's terminal.
    let mut qemu_cmd = smol::process::Command::from(host.interactive_command(&qemu));
    qemu_cmd
        .arg("-nographic")
        .arg("-machine")
        .arg(chip.qemu_machine())
        .arg("-drive")
        .arg(format!("file={},if=mtd,format=raw", flash_image.display()));

    let efuse_image = staging.path().join("efuse.bin");
    if chip.needs_qemu_efuse_workaround() {
        fs::write(&efuse_image, qemu_efuse_image()).await?;
        qemu_cmd
            .arg("-drive")
            .arg(format!(
                "file={},if=none,format=raw,id=efuse",
                efuse_image.display()
            ))
            .arg("-global")
            .arg(format!(
                "driver=nvram.{}.efuse,property=drive,value=efuse",
                chip.id()
            ));
    }

    qemu_cmd.kill_on_drop(true);

    let status = qemu_cmd.status().await?;
    if !status.success() {
        bail!("{} exited with status {status}", chip.qemu_binary());
    }
    Ok(())
}

async fn save_flash_image(
    host: &crate::toolchain::Host,
    backend_path: &Path,
    chip: Esp32Chip,
    elf: &Path,
    image_path: &Path,
) -> eyre::Result<()> {
    let espflash = espflash_path(host).await?;
    let mut save = host.command(espflash);
    let save = command(&mut save, host.std_output());
    save.current_dir(backend_path)
        .arg("save-image")
        .arg("--chip")
        .arg(chip.id())
        .arg("--merge")
        .arg("--flash-size")
        .arg(chip.firmware_params().flash_size_arg())
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
pub async fn package_esp32(
    project: &Project,
    options: PackageOptions,
    built: &BuiltTarget,
) -> eyre::Result<Artifact> {
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let elf = &built.artifact;
    let backend_path = project.backend_path::<Esp32Backend>();
    let chip = esp32_chip(project)?;

    let dist_dir = crate::platforming::packaging::dist_dir(&backend_path, "esp32", Some(profile));
    fs::create_dir_all(&dist_dir).await?;
    // The image ships under the product name; the tagged crate name is
    // internal to the shared Cargo target directory.
    let image_path = dist_dir.join(format!("{}.bin", project.esp32_binary_name()));
    save_flash_image(project.host(), &backend_path, chip, elf, &image_path).await?;

    Ok(Artifact::new(project.bundle_identifier(), image_path))
}

/// Clean Cargo build artifacts and packaged images for the ESP32 harness.
///
/// # Errors
/// Returns an error if `cargo clean` fails or the dist directory cannot be removed.
pub async fn clean_esp32(project: &Project) -> eyre::Result<()> {
    let backend_path = project.backend_path::<Esp32Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.toolchain_target_dir("esp32").await?;

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
    project.host().run("cargo", args).await?;

    let dist_dir = backend_path.join("dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir).await?;
    }
    Ok(())
}
