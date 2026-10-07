//! Physical Apple devices (iPhone, iPad) reachable through CoreDevice.
//!
//! Discovery goes through `xcrun devicectl list devices --json-output -`;
//! install/launch go through `devicectl device install app` and
//! `devicectl device process launch`. Both USB and Wi-Fi ("Connect via
//! network") transports are transparent to `devicectl` — the
//! `connectionProperties.transportType` field reports which one a paired
//! device is currently reachable over.

use std::path::Path;
#[cfg(unix)]
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

#[cfg(unix)]
use crate::device::{TERMINATION_GRACE_PERIOD, TerminationOutcome, await_termination, within};
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
    pub identifier: String,
    /// Hardware UDID (e.g. `00008140-00011C210CF3001C`).
    pub udid: String,
    /// User-assigned device name.
    pub name: String,
    /// Marketing name from `hardwareProperties` (e.g. `iPhone 16 Pro`).
    pub marketing_name: Option<String>,
    /// OS version running on the device (e.g. iOS `27.0`).
    pub os_version: Option<Version>,
    /// How the device is currently reachable (`wired` or `localNetwork`).
    pub transport: Transport,
    /// `CoreDevice` tunnel state (`connected`, `disconnected`, `unavailable`).
    ///
    /// `disconnected` is not a problem: device commands establish the tunnel
    /// on demand. `unavailable` means the device cannot be reached at all.
    pub tunnel_state: TunnelState,
    /// `deviceProperties.developerModeStatus` — must be `enabled` to run
    /// development-signed apps.
    pub developer_mode_enabled: bool,
    /// `deviceProperties.bootState` — the device is usable when `booted`.
    pub boot_state: String,
}

/// How a paired device is connected to this Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// `devicectl device process launch` environment-variable payload.
///
/// The `-e` flag takes a JSON-encoded dictionary; every entry of
/// [`crate::device::RunOptions::env_vars`] travels through it, so
/// `WATERUI_DEV_URL`, `WATERUI_LOG`, `WATERUI_PROJECT_DIR` and
/// `WATERUI_APP_NAME` reach the process exactly as `SIMCTL_CHILD_*` delivers
/// them on a simulator.
fn environment_json<'a>(env_vars: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let map: serde_json::Map<String, serde_json::Value> = env_vars
        .map(|(key, value)| {
            (
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            )
        })
        .collect();
    serde_json::Value::Object(map).to_string()
}

async fn install_device_app(
    host: &Host,
    selector: &str,
    artifact_path: &Path,
) -> Result<(), FailToRun> {
    let output = host
        .command("xcrun")
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

/// Send a signal to the process owned by this monitor.
#[cfg(unix)]
fn signal_child(
    child: &smol::process::Child,
    signal: nix::sys::signal::Signal,
    sender: &Sender<DeviceEvent>,
) {
    let Ok(raw_pid) = i32::try_from(child.id()) else {
        report_monitor_error(
            sender,
            "devicectl PID does not fit in nix::unistd::Pid".to_string(),
        );
        return;
    };
    if let Err(error) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(raw_pid), signal) {
        report_monitor_error(
            sender,
            format!("Could not signal devicectl process: {error}"),
        );
    }
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
        let terminate = host
            .command("xcrun")
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
            .kill_on_drop(true)
            .output()
            .await;
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
                    await_termination(child, control, TERMINATION_GRACE_PERIOD, sender).await
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

        let env_json = environment_json(options.env_vars());
        let bundle_id = artifact.bundle_id().to_string();
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

        // `--console` attaches the app's standard streams to devicectl's and
        // waits for the app to exit: one child gives stdout/stderr streaming,
        // exit detection, and signal forwarding (a signal to devicectl is
        // delivered to the app) in a single process.
        //
        // The dev-server URL travels in the `-e` environment dictionary; a
        // `--waterui-dev-url=` process argument repeats it, so `dev_url()`
        // still finds it if a device-side launch path ever strips the
        // environment.
        #[cfg(unix)]
        let mut command = host.command_in_own_process_group("xcrun");
        #[cfg(not(unix))]
        let mut command = host.command("xcrun");
        command.args([
            "devicectl",
            "device",
            "process",
            "launch",
            "--device",
            self.selector(),
            "--environment-variables",
            &env_json,
            "--terminate-existing",
            "--console",
            &bundle_id,
        ]);
        if let Some((_, dev_url)) = options
            .env_vars()
            .find(|(key, _)| *key == "WATERUI_DEV_URL")
        {
            command.arg(format!("--waterui-dev-url={dev_url}"));
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
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
        ))
        .detach();

        Ok(running)
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

#[cfg(test)]
mod tests {
    use super::{ApplePhysicalDevice, DeviceUnusable, Transport, TunnelState, environment_json};
    use crate::device::Device as _;
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

    #[test]
    fn environment_json_encodes_all_vars() {
        let vars = [
            ("WATERUI_DEV_URL", "http://10.0.0.2:5173/"),
            ("WATERUI_LOG", "debug"),
        ];
        let json = environment_json(vars.iter().copied());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("env json parses");
        assert_eq!(parsed["WATERUI_DEV_URL"], "http://10.0.0.2:5173/");
        assert_eq!(parsed["WATERUI_LOG"], "debug");
    }

    #[cfg(unix)]
    fn console_run(
        machine: &crate::toolchain::testing::TestMachine,
    ) -> (
        crate::device::Running,
        async_channel::Sender<()>,
        async_channel::Receiver<()>,
        ProcessGroupGuard,
    ) {
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
            .command_in_own_process_group(&script)
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
        ))
        .detach();
        let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
        (running, interrupt_tx, interrupt_rx, group_guard)
    }

    #[cfg(unix)]
    #[test]
    fn console_grace_overrun_reports_an_error_then_kills() {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;
        use std::time::Duration;

        use crate::device::DeviceEvent;

        let machine = crate::toolchain::testing::TestMachine::new();
        let (running, interrupt_tx, interrupt_rx, group_guard) = console_run(&machine);
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
    }

    #[cfg(unix)]
    #[test]
    fn console_second_interrupt_kills_without_an_error() {
        use futures_util::future::{Either, select};
        use smol::stream::StreamExt as _;
        use std::time::Duration;

        use crate::device::DeviceEvent;

        let machine = crate::toolchain::testing::TestMachine::new();
        let (running, interrupt_tx, interrupt_rx, group_guard) = console_run(&machine);
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
    }
}
