//! Physical Apple devices (iPhone, iPad) reachable through CoreDevice.
//!
//! Discovery goes through `xcrun devicectl list devices --json-output -`;
//! install/launch go through `devicectl device install app` and
//! `devicectl device process launch`. Both USB and Wi-Fi ("Connect via
//! network") transports are transparent to `devicectl` — the
//! `connectionProperties.transportType` field reports which one a paired
//! device is currently reachable over.

use std::path::Path;
use std::time::Duration;

use eyre::{Context as _, bail, eyre};
use semver::Version;
use serde::Deserialize;
use smol::{
    channel::Sender,
    io::{AsyncBufReadExt, BufReader},
    process::Stdio,
    spawn,
    stream::StreamExt,
};
use tracing::info;

use crate::device::termination_grace;
#[cfg(unix)]
use crate::device::{TerminationOutcome, await_termination, within};
use crate::{
    device::{
        ApplicationExit, Artifact, Crash, CrashCause, Device, DeviceEvent, FailToRun, PanicInfo,
        Running, StopRequest, extract_panic, join_output, report_monitor_error,
    },
    toolchain::Host,
    utils::parse_semver_version,
};

/// A physical Apple device paired with this Mac (iPhone or iPad).
///
/// `devicectl` accepts any of `identifier` (the `CoreDevice` UUID), `udid`,
/// `ecid`, or `name` as its `--device` selector; the `CoreDevice` identifier is
/// the most stable across reboots and reconnects, so it is what
/// [`Self::selector`] returns and what `water run --device` should be given.
#[derive(Debug, Clone)]
pub struct ApplePhysicalDevice {
    /// `CoreDevice` identifier (e.g. `898E9834-79A1-5EAD-AA1A-C54E27F04456`).
    identifier: String,
    /// Hardware UDID (e.g. `00008140-00011C210CF3001C`).
    udid: String,
    /// User-assigned device name.
    name: String,
    /// Marketing name from `hardwareProperties` (e.g. `iPhone 16 Pro`).
    marketing_name: Option<String>,
    /// OS version running on the device (e.g. iOS `27.0`).
    os_version: Option<Version>,
    /// How the device is currently reachable (`wired` or `localNetwork`).
    transport: Transport,
    /// `CoreDevice` tunnel state (`connected`, `disconnected`, `unavailable`).
    ///
    /// `disconnected` is not a problem: device commands establish the tunnel
    /// on demand. `unavailable` means the device cannot be reached at all.
    tunnel_state: TunnelState,
    /// `deviceProperties.developerModeStatus` — must be `enabled` to run
    /// development-signed apps.
    developer_mode_enabled: bool,
    /// `deviceProperties.bootState` — the device is usable when `booted`.
    boot_state: String,
}

impl ApplePhysicalDevice {
    /// `CoreDevice` identifier.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Hardware UDID.
    #[must_use]
    pub fn udid(&self) -> &str {
        &self.udid
    }

    /// User-assigned device name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Marketing name from `hardwareProperties`.
    #[must_use]
    pub fn marketing_name(&self) -> Option<&str> {
        self.marketing_name.as_deref()
    }

    /// OS version running on the device.
    #[must_use]
    pub const fn os_version(&self) -> Option<&Version> {
        self.os_version.as_ref()
    }

    /// How the device is currently reachable.
    #[must_use]
    pub const fn transport(&self) -> Transport {
        self.transport
    }

    /// `CoreDevice` tunnel reachability state.
    #[must_use]
    pub const fn tunnel_state(&self) -> TunnelState {
        self.tunnel_state
    }

    /// Whether developer mode is enabled on the device.
    #[must_use]
    pub const fn developer_mode_enabled(&self) -> bool {
        self.developer_mode_enabled
    }

    /// `deviceProperties.bootState` — the device is usable when `booted`.
    #[must_use]
    pub fn boot_state(&self) -> &str {
        &self.boot_state
    }
}

#[cfg(all(test, target_os = "macos"))]
impl ApplePhysicalDevice {
    /// A fully populated device for the macOS test runner's fixtures.
    pub(crate) fn fixture() -> Self {
        Self {
            identifier: "CORE-DEVICE-ID".to_string(),
            udid: "00008140-0000ABCD00112233".to_string(),
            name: "test-ipad".to_string(),
            marketing_name: None,
            os_version: Some(Version::new(18, 0, 0)),
            transport: Transport::Wired,
            tunnel_state: TunnelState::Connected,
            developer_mode_enabled: true,
            boot_state: "booted".to_string(),
        }
    }
}

/// How a paired device is connected to this Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the terminal reports each device transport variant by variant"
)]
pub enum Transport {
    /// USB or Thunderbolt cable.
    Wired,
    /// "Connect via network" — the device is reachable over the LAN.
    LocalNetwork,
    /// `devicectl` reported a transport this build does not name.
    Other,
}

/// `CoreDevice` tunnel reachability state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TunnelState {
    /// The `CoreDevice` tunnel is up.
    Connected,
    /// Paired and reachable; the tunnel is established on demand.
    Disconnected,
    /// The device cannot be reached (unpaired, offline, or locked out).
    Unavailable,
}

#[derive(Deserialize)]
struct DeviceList {
    result: DeviceListResult,
}

#[derive(Deserialize)]
struct DeviceListResult {
    #[serde(default)]
    devices: Vec<DeviceEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceEntry {
    identifier: String,
    #[serde(default)]
    connection_properties: ConnectionProperties,
    #[serde(default)]
    device_properties: DeviceProperties,
    #[serde(default)]
    hardware_properties: HardwareProperties,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionProperties {
    pairing_state: Option<String>,
    transport_type: Option<String>,
    tunnel_state: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceProperties {
    name: Option<String>,
    os_version_number: Option<String>,
    developer_mode_status: Option<String>,
    boot_state: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HardwareProperties {
    device_type: Option<String>,
    marketing_name: Option<String>,
    udid: Option<String>,
    /// `simulated` on a booted simulator, which Xcode 26's `devicectl` lists
    /// beside the paired hardware; absent on a physical device.
    reality: Option<String>,
}

impl DeviceEntry {
    /// iPhones and iPads are the devices `water run` can target. A booted
    /// simulator shows up here too, as a `simulated` device that
    /// `devicectl` cannot install to; it is the simulator list's, under the
    /// same UDID.
    fn is_ios_device(&self) -> bool {
        matches!(
            self.hardware_properties.device_type.as_deref(),
            Some("iPhone" | "iPad")
        ) && self.hardware_properties.reality.as_deref() != Some("simulated")
    }
}

impl ApplePhysicalDevice {
    /// Parse the `devicectl list devices` JSON output into physical iOS
    /// devices.
    ///
    /// Every paired iPhone/iPad is returned — including ones whose tunnel is
    /// currently `disconnected` (they come up on demand) — so callers can
    /// surface them in pickers and in `water devices`. Unpaired entries are
    /// dropped: `devicectl` cannot act on them at all.
    ///
    /// # Errors
    /// Returns an error when the JSON cannot be parsed.
    pub fn parse_list(json: &str) -> eyre::Result<Vec<Self>> {
        let list: DeviceList =
            serde_json::from_str(json).wrap_err("failed to parse `devicectl list devices` JSON")?;
        Ok(list
            .result
            .devices
            .into_iter()
            .filter(DeviceEntry::is_ios_device)
            .filter_map(|entry| {
                if entry.connection_properties.pairing_state.as_deref() != Some("paired") {
                    return None;
                }
                let os_version = entry
                    .device_properties
                    .os_version_number
                    .as_deref()
                    .map(|raw| {
                        parse_semver_version(raw).map_err(|error| {
                            tracing::warn!(
                                "device {} reported an unparseable osVersionNumber `{raw}`: {error}",
                                entry.identifier
                            );
                        })
                    })
                    .transpose()
                    .ok()
                    .flatten();
                Some(Self {
                    identifier: entry.identifier,
                    udid: entry.hardware_properties.udid.unwrap_or_default(),
                    name: entry
                        .device_properties
                        .name
                        .or_else(|| entry.hardware_properties.marketing_name.clone())
                        .unwrap_or_else(|| String::from("iOS device")),
                    marketing_name: entry.hardware_properties.marketing_name,
                    os_version,
                    transport: match entry.connection_properties.transport_type.as_deref() {
                        Some("wired") => Transport::Wired,
                        Some("localNetwork") => Transport::LocalNetwork,
                        _ => Transport::Other,
                    },
                    tunnel_state: match entry.connection_properties.tunnel_state.as_deref() {
                        Some("connected") => TunnelState::Connected,
                        Some("unavailable") => TunnelState::Unavailable,
                        _ => TunnelState::Disconnected,
                    },
                    developer_mode_enabled: entry
                        .device_properties
                        .developer_mode_status
                        .as_deref()
                        == Some("enabled"),
                    boot_state: entry.device_properties.boot_state.unwrap_or_default(),
                })
            })
            .collect())
    }

    /// Enumerate paired iOS devices through `devicectl`.
    ///
    /// # Errors
    /// Returns an error when `devicectl` fails or its output cannot be parsed.
    pub async fn scan(host: &Host) -> eyre::Result<Vec<Self>> {
        let output = host
            .output(
                "xcrun",
                ["devicectl", "list", "devices", "--json-output", "-"],
            )
            .await
            .wrap_err("failed to run `devicectl list devices`")?;
        if !output.status.success() {
            bail!(
                "`devicectl list devices` failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Self::parse_list(&String::from_utf8_lossy(&output.stdout))
    }

    /// The value `devicectl --device` selects this device by.
    #[must_use]
    pub fn selector(&self) -> &str {
        &self.identifier
    }

    /// Whether `water run` can put an app on this device right now.
    ///
    /// A disconnected tunnel is not a failure — commands bring it up — but an
    /// `unavailable` tunnel, an unbooted device, or Developer Mode being off
    /// each make the device unusable and each has a different remedy, so the
    /// reasons stay distinct for diagnostics.
    ///
    /// # Errors
    /// Returns the [`DeviceUnusable`] reason whose `remedy` text names the fix.
    pub fn usability(&self) -> Result<(), DeviceUnusable> {
        if matches!(self.tunnel_state, TunnelState::Unavailable) {
            return Err(DeviceUnusable::Unreachable);
        }
        if self.boot_state != "booted" {
            return Err(DeviceUnusable::NotBooted);
        }
        if !self.developer_mode_enabled {
            return Err(DeviceUnusable::DeveloperModeDisabled);
        }
        Ok(())
    }

    /// Whether the device's OS can run an app requiring `deployment_target`.
    ///
    /// A device whose OS version `devicectl` did not report is treated as
    /// incapable: selection must never pick a device it cannot prove satisfies
    /// the app's deployment target.
    #[must_use]
    pub fn supports_deployment_target(&self, deployment_target: &Version) -> bool {
        self.os_version
            .as_ref()
            .is_some_and(|os| os >= deployment_target)
    }
}

/// Why a paired device cannot run an app; each variant maps to the remedy the
/// error message should name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceUnusable {
    /// The `CoreDevice` tunnel is `unavailable` — the device is off, unplugged
    /// and off-LAN, or locked out.
    Unreachable,
    /// `bootState` is not `booted`.
    NotBooted,
    /// Developer Mode is off; development-signed apps cannot launch.
    DeveloperModeDisabled,
}

impl DeviceUnusable {
    /// The user-facing explanation and remedy for this state.
    #[must_use]
    pub fn remedy(self, device: &ApplePhysicalDevice) -> String {
        match self {
            Self::Unreachable => format!(
                "{} is paired but unreachable. Unlock it and check the USB cable, \
                 or enable “Connect via network” in Xcode → Devices and Simulators \
                 while the iPhone and this Mac share a LAN.",
                device.name
            ),
            Self::NotBooted => format!("{} is not booted.", device.name),
            Self::DeveloperModeDisabled => format!(
                "Developer Mode is disabled on {}. Enable it in \
                 Settings → Privacy & Security → Developer Mode, then restart the device.",
                device.name
            ),
        }
    }
}

/// `devicectl device process launch` for [`Device::run`] on a physical
/// device. `--console` attaches the app's standard streams to devicectl's
/// and waits for the app to exit: one child gives stdout/stderr streaming,
/// exit detection, and signal forwarding (a signal to devicectl is
/// delivered to the app) in a single process.
///
/// Environment variables never enter argv: `devicectl` forwards every
/// caller-side `DEVICECTL_CHILD_<key>` to the launched process, so each
/// `RunOptions` entry rides the host's own prefixed channel — dev-server
/// URL included. Any `DEVICECTL_CHILD_*` the host already carries is
/// cleared from the child first, so `RunOptions` owns the namespace;
/// the host's other variables pass through unchanged.
fn physical_device_launch_command(
    host: &Host,
    selector: &str,
    bundle_id: &str,
    options: &crate::device::RunOptions,
) -> std::io::Result<smol::process::Command> {
    #[cfg(unix)]
    let mut command = host.command_in_own_process_group("xcrun")?;
    #[cfg(not(unix))]
    let mut command = host.command("xcrun")?;
    command.args([
        "devicectl",
        "--quiet",
        "device",
        "process",
        "launch",
        "--device",
        selector,
        "--terminate-existing",
        "--console",
        bundle_id,
    ]);
    for argument in options.arguments() {
        command.arg(argument);
    }
    for (key, _) in host.envs() {
        if key.as_encoded_bytes().starts_with(b"DEVICECTL_CHILD_") {
            command.env_remove(key);
        }
    }
    for (key, value) in options.env_vars() {
        command.env(format!("DEVICECTL_CHILD_{key}"), value);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}

pub(crate) async fn install_device_app(
    host: &Host,
    selector: &str,
    artifact_path: &Path,
) -> Result<(), FailToRun> {
    let output = host
        .command("xcrun")
        .map_err(|error| FailToRun::Install(eyre!("Failed to install app: {error}")))?
        .args([
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            selector,
        ])
        .arg(artifact_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| FailToRun::Install(eyre!("Failed to install app: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(FailToRun::Install(eyre!(
        "Failed to install app on the device:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim(),
    )))
}

impl ApplePhysicalDevice {
    /// `devicectl device copy from` for the app's data container: pull
    /// `source` (relative to the container root) to `destination` on the
    /// host. A failed copy — the file is absent — is an error carrying
    /// devicectl's stderr.
    #[cfg(target_os = "macos")]
    pub(crate) async fn copy_from_data_container(
        &self,
        host: &Host,
        bundle_id: &str,
        source: &str,
        destination: &Path,
    ) -> eyre::Result<()> {
        let output = host
            .command("xcrun")
            .wrap_err("failed to run devicectl device copy from")?
            .args([
                "devicectl",
                "device",
                "copy",
                "from",
                "--device",
                self.selector(),
                "--domain-type",
                "appDataContainer",
                "--domain-identifier",
                bundle_id,
                "--source",
                source,
                "--destination",
            ])
            .arg(destination)
            .output()
            .await
            .wrap_err("failed to run devicectl device copy from")?;
        if !output.status.success() {
            bail!(
                "devicectl device copy from failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

/// Find the app's pid on the device by its executable name.
///
/// `devicectl device info processes` lists remote processes; the one running
/// our bundle has the app binary's name as the last component of its
/// executable path.
#[cfg(unix)]
async fn find_remote_pid(
    host: &Host,
    selector: &str,
    process_name: &str,
) -> eyre::Result<Option<u32>> {
    #[derive(Deserialize)]
    struct ProcessList {
        result: ProcessListResult,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ProcessListResult {
        #[serde(default)]
        running_processes: Vec<RemoteProcess>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RemoteProcess {
        executable: String,
        process_identifier: u32,
    }

    let output = host
        .command("xcrun")
        .wrap_err("Could not run devicectl device info processes")?
        .args([
            "devicectl",
            "device",
            "info",
            "processes",
            "--device",
            selector,
            "--json-output",
            "-",
        ])
        .kill_on_drop(true)
        .output()
        .await
        .wrap_err("Could not run devicectl device info processes")?;
    if !output.status.success() {
        bail!(
            "devicectl device info processes failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let list: ProcessList = serde_json::from_slice(&output.stdout)
        .wrap_err("Could not parse devicectl device info processes JSON")?;
    let suffix = format!("/{process_name}");
    Ok(list
        .result
        .running_processes
        .into_iter()
        .find(|process| process.executable.ends_with(&suffix))
        .map(|process| process.process_identifier))
}

/// Send a signal to the `devicectl` process this monitor owns.
#[cfg(unix)]
fn signal_child(
    child: &smol::process::Child,
    signal: nix::sys::signal::Signal,
    sender: &Sender<DeviceEvent>,
) {
    crate::device::signal_child(child, signal, sender, "devicectl");
}

/// Bounds finding the app's remote PID and killing it with
/// `devicectl device process terminate --kill`, together. The local SIGKILL
/// to `devicectl` and its reap follow.
#[cfg(unix)]
const KILL_BOUND: Duration = Duration::from_secs(10);

#[cfg(unix)]
async fn kill_console_session(
    child: &mut smol::process::Child,
    host: &Host,
    selector: &str,
    process_name: &str,
    sender: &Sender<DeviceEvent>,
) -> std::io::Result<std::process::ExitStatus> {
    let remote_kill = async {
        let pid = match find_remote_pid(host, selector, process_name).await {
            Ok(Some(pid)) => pid,
            Ok(None) => return,
            Err(error) => {
                report_monitor_error(
                    sender,
                    format!(
                        "Could not look up the app's pid on the device; it may still be running there: {error}"
                    ),
                );
                return;
            }
        };
        let terminate = match host.command("xcrun") {
            Ok(mut command) => {
                command
                    .args([
                        "devicectl",
                        "device",
                        "process",
                        "terminate",
                        "--device",
                        selector,
                        "--kill",
                        "--pid",
                        &pid.to_string(),
                    ])
                    .kill_on_drop(true);
                command.output().await
            }
            Err(error) => Err(error),
        };
        match terminate {
            Ok(output) if output.status.success() => {}
            Ok(output) => report_monitor_error(
                sender,
                format!(
                    "devicectl process terminate failed with status {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ),
            Err(error) => report_monitor_error(
                sender,
                format!("devicectl process terminate failed: {error}"),
            ),
        }
    };
    if within(KILL_BOUND, remote_kill).await.is_none() {
        report_monitor_error(
            sender,
            format!("Killing the app on the device did not finish within {KILL_BOUND:?}"),
        );
    }
    signal_child(child, nix::sys::signal::Signal::SIGKILL, sender);
    child.status().await
}

struct ConsoleMonitorChannels {
    control: smol::channel::Receiver<StopRequest>,
    eof: smol::channel::Receiver<()>,
    panic: smol::channel::Receiver<PanicInfo>,
}

async fn monitor_console_session(
    mut child: smol::process::Child,
    host: Host,
    selector: String,
    process_name: String,
    channels: ConsoleMonitorChannels,
    sender: Sender<DeviceEvent>,
    termination_grace: Duration,
) {
    let ConsoleMonitorChannels {
        control,
        eof,
        panic,
    } = channels;
    // Held until the terminal event is sent, so the supervisor's requests
    // keep being delivered while the session ends.
    let mut control = Some(control);
    let status = wait_for_console_status(
        &mut child,
        &host,
        &selector,
        &process_name,
        &mut control,
        &sender,
        termination_grace,
    )
    .await;
    let console_closed = async {
        let _ = eof.recv().await;
        let _ = eof.recv().await;
    };
    let _ = join_output(console_closed, &mut control).await;
    if let Err(error) = status {
        report_monitor_error(
            &sender,
            format!("Could not wait for devicectl console: {error}"),
        );
    }
    let event = panic.try_recv().map_or_else(
        |_| DeviceEvent::Exited(ApplicationExit::user_closed()),
        |panic| DeviceEvent::Crashed(Crash::new(CrashCause::Panic(panic))),
    );
    let _ = sender.try_send(event);
    drop(control);
}

/// Wait for the console session to end, carrying out stop requests.
///
/// Returns the session's exit status.
#[cfg(unix)]
async fn wait_for_console_status(
    child: &mut smol::process::Child,
    host: &Host,
    selector: &str,
    process_name: &str,
    control: &mut Option<smol::channel::Receiver<StopRequest>>,
    sender: &Sender<DeviceEvent>,
    termination_grace: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    use futures_util::future::{Either, select};

    loop {
        let status = std::pin::pin!(child.status());
        let Some(control_receiver) = control.as_mut() else {
            return status.await;
        };
        let request = std::pin::pin!(control_receiver.recv());
        match select(status, request).await {
            Either::Left((status, _)) => return status,
            Either::Right((Err(_), _)) => *control = None,
            Either::Right((Ok(StopRequest::Terminate), _)) => {
                signal_child(child, nix::sys::signal::Signal::SIGTERM, sender);
                if let TerminationOutcome::Exited(status) =
                    await_termination(child, control, termination_grace, sender).await
                {
                    return status;
                }
                return kill_console_session(child, host, selector, process_name, sender).await;
            }
            Either::Right((Ok(StopRequest::Kill), _)) => {
                return kill_console_session(child, host, selector, process_name, sender).await;
            }
        }
    }
}

/// Wait for the console session to end, carrying out stop requests.
///
/// Returns the session's exit status.
#[cfg(not(unix))]
async fn wait_for_console_status(
    child: &mut smol::process::Child,
    _host: &Host,
    _selector: &str,
    _process_name: &str,
    control: &mut Option<smol::channel::Receiver<StopRequest>>,
    _sender: &Sender<DeviceEvent>,
    _termination_grace: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    use futures_util::future::{Either, select};

    loop {
        let status = std::pin::pin!(child.status());
        let Some(control_receiver) = control.as_mut() else {
            return status.await;
        };
        let request = std::pin::pin!(control_receiver.recv());
        match select(status, request).await {
            Either::Left((status, _)) => return status,
            Either::Right((Err(_), _)) => *control = None,
            Either::Right((Ok(StopRequest::Terminate | StopRequest::Kill), _)) => {
                let _ = child.kill();
                return child.status().await;
            }
        }
    }
}

impl ApplePhysicalDevice {
    /// The `url` `devicectl` reports for the installed `bundle_id`, or
    /// `None` when no such app is installed. Parsed from `--json-output`
    /// — the human table is never scraped. `json_path` names the file the
    /// command writes its report to.
    ///
    /// devicectl's `--json-output` writes its report to a file, and for an
    /// installed app exposes only `bundleIdentifier`, `name` and `url`
    /// (`file:///private/var/containers/Bundle/Application/<UUID>/<name>.app/`).
    /// An overwrite install lands under a new UUID, so the URL changing is
    /// how the test runner detects that something else installed the app.
    #[cfg(target_os = "macos")]
    pub(crate) async fn installed_app_url(
        &self,
        host: &Host,
        bundle_id: &str,
        json_path: &std::path::Path,
    ) -> eyre::Result<Option<String>> {
        let output = host
            .output(
                "xcrun",
                [
                    "devicectl",
                    "device",
                    "info",
                    "apps",
                    "--device",
                    self.selector(),
                    "--bundle-id",
                    bundle_id,
                    "--json-output",
                    &json_path.to_string_lossy(),
                ],
            )
            .await
            .wrap_err("failed to query the device's installed applications")?;
        if !output.status.success() {
            bail!(
                "devicectl device info apps failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let text = std::fs::read_to_string(json_path)
            .wrap_err("devicectl wrote no --json-output report")?;
        installed_app_url_from_json(&text, bundle_id)
            .map_err(|error| eyre!("could not parse devicectl's app report: {error:#}: {text}"))
    }

    /// Launch an already-installed artifact with `--console`, streaming its
    /// output and owning its stop path — the install-free half of
    /// [`Device::run`]. `process_name` is the bundle's executable name —
    /// the name `devicectl device process list` reports — used to find the
    /// remote pid for an escalating stop.
    pub(crate) fn launch_monitored(
        &self,
        host: &Host,
        artifact: &Artifact,
        options: &crate::device::RunOptions,
        process_name: &str,
    ) -> Result<Running, FailToRun> {
        let bundle_id = artifact.bundle_id().to_string();
        let process_name = process_name.to_string();

        // `--console` attaches the app's standard streams to devicectl's and
        // waits for the app to exit: one child gives stdout/stderr streaming,
        // exit detection, and signal forwarding (a signal to devicectl is
        // delivered to the app) in a single process.
        let mut child = physical_device_launch_command(host, self.selector(), &bundle_id, options)
            .and_then(|mut command| command.spawn())
            .map_err(|error| FailToRun::Launch(eyre!("Failed to launch app: {error}")))?;

        let stdout = child
            .stdout
            .take()
            .expect("stdout is piped for the devicectl console");
        let stderr = child
            .stderr
            .take()
            .expect("stderr is piped for the devicectl console");

        let (running, sender, control) = Running::new();

        let (panic_rx, eof_rx) = spawn_console_streams(stdout, stderr, &sender);
        spawn(monitor_console_session(
            child,
            host.clone(),
            self.selector().to_string(),
            process_name,
            ConsoleMonitorChannels {
                control,
                eof: eof_rx,
                panic: panic_rx,
            },
            sender,
            termination_grace(),
        ))
        .detach();

        Ok(running)
    }
}

impl Device for ApplePhysicalDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_udid(&self) -> Option<&str> {
        Some(&self.udid)
    }

    fn launch(&self, _host: &Host) -> impl Future<Output = eyre::Result<()>> + Send {
        // A physical device needs no boot step — but surface a clear error
        // for the states that would make `run` fail anyway.
        std::future::ready(
            self.usability()
                .map_err(|reason| eyre!("{}", reason.remedy(self))),
        )
    }

    async fn run(
        &self,
        host: &Host,
        artifact: Artifact,
        options: crate::device::RunOptions,
    ) -> Result<Running, FailToRun> {
        if let Err(reason) = self.usability() {
            return Err(FailToRun::Run(eyre!("{}", reason.remedy(self))));
        }

        info!(
            "Installing {} on {} ({})",
            artifact.bundle_id(),
            self.name,
            self.identifier
        );
        install_device_app(host, self.selector(), artifact.path()).await?;
        // A `water run` bundle's stem is its CFBundleExecutable name.
        let process_name = artifact
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                FailToRun::Run(eyre!(
                    "Artifact path has no UTF-8 filename: {}",
                    artifact.path().display()
                ))
            })?
            .to_string();
        self.launch_monitored(host, &artifact, &options, &process_name)
    }

    async fn scan(host: &Host) -> eyre::Result<Vec<Self>> {
        Self::scan(host).await
    }
}

/// Forward devicectl's console pipes to `sender`.
///
/// devicectl writes the app's stdout to its stdout and the app's stderr to
/// its stderr. A panic message on the stderr side is reported as a crash
/// once the console detaches. Returns the panic channel and the end-of-file
/// signal each pipe sends once.
fn spawn_console_streams(
    stdout: smol::process::ChildStdout,
    stderr: smol::process::ChildStderr,
    sender: &Sender<DeviceEvent>,
) -> (
    smol::channel::Receiver<PanicInfo>,
    smol::channel::Receiver<()>,
) {
    let (panic_tx, panic_rx) = smol::channel::bounded::<PanicInfo>(1);
    let (eof_tx, eof_rx) = smol::channel::bounded::<()>(2);

    spawn(stream_console(
        stdout,
        ConsoleTarget {
            sender: sender.clone(),
            eof: eof_tx.clone(),
            panic: None,
            is_err: false,
        },
    ))
    .detach();
    spawn(stream_console(
        stderr,
        ConsoleTarget {
            sender: sender.clone(),
            eof: eof_tx,
            panic: Some(panic_tx),
            is_err: true,
        },
    ))
    .detach();
    (panic_rx, eof_rx)
}

/// Output channel for one console pipe: the user's event sender plus the
/// end-of-file signal the exit classifier waits on.
struct ConsoleTarget {
    sender: Sender<DeviceEvent>,
    eof: Sender<()>,
    panic: Option<Sender<PanicInfo>>,
    is_err: bool,
}

/// Stream one of devicectl's console pipes into device events, then report
/// end-of-file so the exit classifier can run once both pipes close.
async fn stream_console(stream: impl smol::io::AsyncRead + Unpin, target: ConsoleTarget) {
    let mut lines = BufReader::new(stream).lines();
    while let Some(Ok(line)) = lines.next().await {
        if target.is_err
            && let Some(panic) = &target.panic
            && let Some(info) = extract_panic(std::slice::from_ref(&line))
        {
            let _ = panic.try_send(info);
        }
        let event = if target.is_err {
            DeviceEvent::Stderr { message: line }
        } else {
            DeviceEvent::Stdout { message: line }
        };
        if target.sender.try_send(event).is_err() {
            break;
        }
    }
    let _ = target.eof.try_send(());
}

/// The `url` of `bundle_id` inside devicectl's `--json-output` report —
/// the only parse the runner trusts, never the human table. The report's
/// shape is `{"result": {"apps": [ … ]}}`; entries are matched on
/// `bundleIdentifier` exactly.
#[cfg(target_os = "macos")]
fn installed_app_url_from_json(text: &str, bundle_id: &str) -> eyre::Result<Option<String>> {
    #[derive(serde::Deserialize)]
    struct Report {
        result: ResultSection,
    }
    #[derive(serde::Deserialize)]
    struct ResultSection {
        #[serde(default)]
        apps: Vec<App>,
    }
    #[derive(serde::Deserialize)]
    struct App {
        #[serde(rename = "bundleIdentifier")]
        bundle_identifier: String,
        url: Option<String>,
    }

    let report: Report = serde_json::from_str(text)?;
    Ok(report
        .result
        .apps
        .into_iter()
        .find(|app| app.bundle_identifier == bundle_id)
        .and_then(|app| app.url))
}

#[cfg(test)]
mod tests {
    use super::{
        ApplePhysicalDevice, DeviceUnusable, Transport, TunnelState, physical_device_launch_command,
    };
    #[cfg(unix)]
    use crate::workflows::device::test_support::{ProcessGroupGuard, term_ignoring_fixture};

    const DEVICE_LIST_JSON: &str = include_str!("physical_list_sample.json");

    #[test]
    fn parses_paired_iphone() {
        let devices = ApplePhysicalDevice::parse_list(DEVICE_LIST_JSON).expect("parse list");
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.identifier, "898E9834-79A1-5EAD-AA1A-C54E27F04456");
        assert_eq!(device.udid, "00008140-00011C210CF3001C");
        assert_eq!(device.name(), "Lexo’s iPhone 16 Pro");
        assert_eq!(device.marketing_name.as_deref(), Some("iPhone 16 Pro"));
        assert_eq!(device.transport, Transport::Wired);
        assert_eq!(device.tunnel_state, TunnelState::Disconnected);
        assert!(device.developer_mode_enabled);
        assert!(device.usability().is_ok());
    }

    #[test]
    fn booted_simulator_is_not_a_physical_device() {
        let devices = ApplePhysicalDevice::parse_list(DEVICE_LIST_JSON).expect("parse list");
        assert!(
            devices
                .iter()
                .all(|device| device.udid != "3C6AEFDA-0324-4C6E-9352-4A2DAF059AF0"),
            "the simulated iPhone 17 entry must stay out of the physical device list"
        );
    }

    #[cfg(unix)]
    #[test]
    fn find_remote_pid_reports_invalid_json_and_missing_process() {
        let invalid_machine = crate::toolchain::testing::TestMachine::new();
        invalid_machine.install("xcrun");
        invalid_machine.respond("XCRUN_DEVICE_INFO_PROCESSES", "not json");
        let invalid_host = invalid_machine.host(Vec::<(String, String)>::new());
        assert!(
            smol::block_on(super::find_remote_pid(
                &invalid_host,
                "test-device",
                "test-app"
            ))
            .is_err()
        );

        let empty_machine = crate::toolchain::testing::TestMachine::new();
        empty_machine.install("xcrun");
        empty_machine.respond(
            "XCRUN_DEVICE_INFO_PROCESSES",
            r#"{"result":{"runningProcesses":[]}}"#,
        );
        let empty_host = empty_machine.host(Vec::<(String, String)>::new());
        assert_eq!(
            smol::block_on(super::find_remote_pid(
                &empty_host,
                "test-device",
                "test-app"
            ))
            .expect("the process-list JSON is valid"),
            None
        );
    }

    #[test]
    fn unavailable_tunnel_is_unusable() {
        let mut devices = ApplePhysicalDevice::parse_list(DEVICE_LIST_JSON).expect("parse list");
        devices[0].tunnel_state = TunnelState::Unavailable;
        assert_eq!(devices[0].usability(), Err(DeviceUnusable::Unreachable));
    }

    #[test]
    fn developer_mode_off_is_unusable() {
        let mut devices = ApplePhysicalDevice::parse_list(DEVICE_LIST_JSON).expect("parse list");
        devices[0].developer_mode_enabled = false;
        assert_eq!(
            devices[0].usability(),
            Err(DeviceUnusable::DeveloperModeDisabled)
        );
    }

    /// The command env folded last-wins-per-key, the way spawn resolves it.
    fn command_env(command: &smol::process::Command) -> std::collections::HashMap<String, String> {
        command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect()
    }

    /// RED for water-rs/waterui#2600: `RunOptions` values must ride the
    /// caller-side `DEVICECTL_CHILD_*` channel — argv must never see them.
    #[test]
    fn the_launch_environment_never_enters_the_device_argv() -> std::io::Result<()> {
        let machine = crate::toolchain::testing::TestMachine::new();
        machine.install("xcrun");
        let host = machine.host(Vec::<(String, String)>::new());
        let mut options = crate::device::RunOptions::new();
        options.insert_env_var(
            String::from("WATERUI_DUMMY_KEY"),
            String::from("dummy-secret-value"),
        );
        options.insert_env_var(
            String::from("WATERUI_DEV_URL"),
            String::from("http://10.0.0.2:5173/"),
        );

        let command =
            physical_device_launch_command(&host, "ABC123-SELECTOR", "com.example.app", &options)?;
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            !args.iter().any(|arg| arg.contains("dummy-secret-value")),
            "a declared value must never reach argv: {args:?}"
        );
        assert!(
            !args.iter().any(|arg| arg.contains("http://10.0.0.2:5173/")),
            "the dev URL is environment, not argv: {args:?}"
        );
        assert!(
            !args.iter().any(|arg| arg == "--environment-variables"),
            "{args:?}"
        );
        assert_eq!(
            args,
            [
                "devicectl",
                "--quiet",
                "device",
                "process",
                "launch",
                "--device",
                "ABC123-SELECTOR",
                "--terminate-existing",
                "--console",
                "com.example.app",
            ]
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
        );
        Ok(())
    }

    /// Every `RunOptions` pair lands as an exact `DEVICECTL_CHILD_<key>`
    /// value — spaces, newlines, `=`, non-ASCII and empty survive verbatim.
    #[test]
    fn declared_values_ride_the_child_prefixed_channel_verbatim() -> std::io::Result<()> {
        let machine = crate::toolchain::testing::TestMachine::new();
        machine.install("xcrun");
        let host = machine.host(Vec::<(String, String)>::new());
        let mut options = crate::device::RunOptions::new();
        for (key, value) in [
            ("SPACED", "a  b\nc"),
            ("QUOTED", "say \"hi\" $HOME"),
            ("EQUALS", "x=y"),
            ("NONASCII", "日本語"),
            ("EMPTY", ""),
        ] {
            options.insert_env_var(String::from(key), String::from(value));
        }

        let command = physical_device_launch_command(&host, "S", "com.example.app", &options)?;
        let env = command_env(&command);
        for (key, value) in [
            ("SPACED", "a  b\nc"),
            ("QUOTED", "say \"hi\" $HOME"),
            ("EQUALS", "x=y"),
            ("NONASCII", "日本語"),
            ("EMPTY", ""),
        ] {
            assert_eq!(
                env.get(&format!("DEVICECTL_CHILD_{key}"))
                    .map(String::as_str),
                Some(value),
                "{key}"
            );
        }
        Ok(())
    }

    /// A `DEVICECTL_CHILD_*` the host already carries is cleared from the
    /// child — `RunOptions` owns the namespace — while the host itself and
    /// its unrelated variables stay untouched.
    #[test]
    fn run_options_own_the_child_prefixed_namespace() -> std::io::Result<()> {
        let machine = crate::toolchain::testing::TestMachine::new();
        machine.install("xcrun");
        let host = machine.host([
            (
                String::from("DEVICECTL_CHILD_STALE"),
                String::from("host-stale"),
            ),
            (
                String::from("DEVICECTL_CHILD_OVERRIDDEN"),
                String::from("host-value"),
            ),
        ]);
        let mut options = crate::device::RunOptions::new();
        options.insert_env_var(String::from("OVERRIDDEN"), String::from("run-value"));

        let command = physical_device_launch_command(&host, "S", "com.example.app", &options)?;
        let env = command_env(&command);
        assert!(!env.contains_key("DEVICECTL_CHILD_STALE"), "{env:?}");
        assert_eq!(
            env.get("DEVICECTL_CHILD_OVERRIDDEN").map(String::as_str),
            Some("run-value")
        );
        assert_eq!(
            env.get("PATH").map(String::as_str),
            host.env("PATH")
                .map(|value| value.to_string_lossy().into_owned())
                .as_deref()
        );

        // The host is immutable: its own env map never observed the purge.
        assert_eq!(
            host.env("DEVICECTL_CHILD_STALE")
                .map(|value| value.to_string_lossy().into_owned()),
            Some(String::from("host-stale"))
        );
        Ok(())
    }

    #[cfg(unix)]
    fn console_run(
        machine: &crate::toolchain::testing::TestMachine,
        termination_grace: std::time::Duration,
    ) -> std::io::Result<(
        crate::device::Running,
        async_channel::Sender<()>,
        async_channel::Receiver<()>,
        ProcessGroupGuard,
    )> {
        use std::process::Stdio;

        use crate::device::Running;

        machine.install("xcrun");
        machine.respond(
            "XCRUN_DEVICE_INFO_PROCESSES",
            r#"{"result":{"runningProcesses":[]}}"#,
        );
        let script = term_ignoring_fixture(machine);
        let host = machine.host(Vec::<(String, String)>::new());
        let mut child = host
            .command_in_own_process_group(&script)?
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn the TERM-ignoring fixture");
        let pid = i32::try_from(child.id()).expect("child pid fits in i32");
        let group_guard = ProcessGroupGuard::default();
        group_guard.set(nix::unistd::Pid::from_raw(pid));
        let stdout = child.stdout.take().expect("fixture stdout is piped");
        let stderr = child.stderr.take().expect("fixture stderr is piped");
        let (running, sender, control) = Running::new();
        let (panic, eof) = super::spawn_console_streams(stdout, stderr, &sender);
        smol::spawn(super::monitor_console_session(
            child,
            host,
            "test-device".to_string(),
            "ignore-term".to_string(),
            super::ConsoleMonitorChannels {
                control,
                eof,
                panic,
            },
            sender,
            termination_grace,
        ))
        .detach();
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        Ok((running, interrupt_tx, interrupt_rx, group_guard))
    }

    #[cfg(unix)]
    #[test]
    fn console_grace_overrun_reports_an_error_then_kills() -> std::io::Result<()> {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;
        use std::time::Duration;

        use crate::device::DeviceEvent;

        let machine = crate::toolchain::testing::TestMachine::new();
        let (running, interrupt_tx, interrupt_rx, group_guard) =
            console_run(&machine, Duration::from_millis(200))?;
        let exercise = async move {
            let _group_guard = group_guard;
            let mut events = std::pin::pin!(running.supervise(interrupt_rx));
            let mut armed = false;
            let mut monitor_error = None;
            let mut stopped = false;
            let mut pid = None;
            while let Some(event) = events.next().await {
                match event {
                    DeviceEvent::Stderr { message } => {
                        if let Some(found) = message
                            .strip_prefix("pid=")
                            .and_then(|pid| pid.parse::<i32>().ok())
                        {
                            pid = Some(nix::unistd::Pid::from_raw(found));
                        }
                        if !armed && message.contains("trap-armed") {
                            armed = true;
                            interrupt_tx.try_send(()).expect("request termination");
                        }
                    }
                    DeviceEvent::MonitorError { message } => monitor_error = Some(message),
                    DeviceEvent::Stopped => {
                        stopped = true;
                        break;
                    }
                    DeviceEvent::Exited(_) | DeviceEvent::Crashed(_) => break,
                    _ => {}
                }
            }
            (armed, monitor_error, stopped, pid)
        };
        let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(30)));
        let (armed, monitor_error, stopped, pid) = smol::block_on(async {
            match select(Box::pin(exercise), deadline).await {
                Either::Left((result, _)) => result,
                Either::Right(_) => panic!("console grace overrun exceeded 30 seconds"),
            }
        });
        assert!(armed, "fixture trap arms before the interrupt");
        let monitor_error = monitor_error.expect("the grace overrun is reported before Stopped");
        assert!(
            monitor_error.contains("termination grace period"),
            "the error names the grace overrun: {monitor_error}"
        );
        assert!(stopped, "the overrun still ends in Stopped");
        let pid = pid.expect("fixture prints its pid");
        assert_eq!(
            nix::sys::signal::kill(pid, None),
            Err(nix::errno::Errno::ESRCH),
            "Stopped follows the confirmed child exit and reap"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn console_second_interrupt_kills_without_an_error() -> std::io::Result<()> {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;
        use std::time::Duration;

        use crate::device::DeviceEvent;

        let machine = crate::toolchain::testing::TestMachine::new();
        let (running, interrupt_tx, interrupt_rx, group_guard) =
            console_run(&machine, Duration::from_secs(3600))?;
        let exercise = async move {
            let _group_guard = group_guard;
            let mut events = std::pin::pin!(running.supervise(interrupt_rx));
            let mut armed = false;
            let mut term_seen = false;
            let mut monitor_error = false;
            let mut stopped = false;
            let mut pid = None;
            while let Some(event) = events.next().await {
                match event {
                    DeviceEvent::Stderr { message } => {
                        if let Some(found) = message
                            .strip_prefix("pid=")
                            .and_then(|pid| pid.parse::<i32>().ok())
                        {
                            pid = Some(nix::unistd::Pid::from_raw(found));
                        }
                        if !armed && message.contains("trap-armed") {
                            armed = true;
                            interrupt_tx.try_send(()).expect("request termination");
                        }
                        if !term_seen && message.contains("term-seen") {
                            term_seen = true;
                            interrupt_tx.try_send(()).expect("request immediate kill");
                        }
                    }
                    DeviceEvent::MonitorError { .. } => monitor_error = true,
                    DeviceEvent::Stopped => {
                        stopped = true;
                        break;
                    }
                    DeviceEvent::Exited(_) | DeviceEvent::Crashed(_) => break,
                    _ => {}
                }
            }
            (armed, term_seen, monitor_error, stopped, pid)
        };
        let deadline = std::pin::pin!(smol::Timer::after(Duration::from_secs(30)));
        let (armed, term_seen, monitor_error, stopped, pid) = smol::block_on(async {
            match select(Box::pin(exercise), deadline).await {
                Either::Left((result, _)) => result,
                Either::Right(_) => panic!("console TERM escalation exceeded 30 seconds"),
            }
        });
        assert!(armed, "fixture trap arms before the first interrupt");
        assert!(term_seen, "fixture receives SIGTERM and remains alive");
        assert!(stopped, "second interrupt waits for the monitor's kill ack");
        assert!(
            !monitor_error,
            "a kill the user asked for is not a monitor error"
        );
        let pid = pid.expect("fixture prints its pid");
        assert_eq!(
            nix::sys::signal::kill(pid, None),
            Err(nix::errno::Errno::ESRCH),
            "Stopped follows the confirmed child exit and reap"
        );
        Ok(())
    }
}
