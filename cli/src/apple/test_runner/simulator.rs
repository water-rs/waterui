//! The iOS Simulator destination — the `nextest-ios-sim.sh` behaviour as
//! Rust, under the [`Host`] seam.

use std::io::Write;
use std::path::Path;

use eyre::{Context as _, eyre};

use crate::apple::app_bundle::codesign_path;
use crate::apple::device::{AppleSimulator, install_simulator_artifact};
use crate::device::Device as _;
use crate::toolchain::Host;

use super::lock::DestinationLock;
use super::status::{StatusFile, TestStatus};
use super::{
    HarnessBundle, Invocation, MachOPlatform, STATUS_PATH_VARIABLE, TestDestination, exit_code,
    forward_run, pass_through_exit,
};

/// The simulator runner: the device comes from `WATERUI_IOS_SIM_UDID` —
/// the runner never picks a device implicitly.
#[derive(Debug)]
pub struct IosSimulator {
    udid: String,
}

impl IosSimulator {
    /// A runner bound to the simulator `udid` names.
    #[must_use]
    pub const fn new(udid: String) -> Self {
        Self { udid }
    }
}

impl TestDestination for IosSimulator {
    fn platform(&self) -> MachOPlatform {
        MachOPlatform::IOSSIMULATOR
    }

    async fn run(
        &self,
        host: &Host,
        invocation: &Invocation,
        interrupts: smol::channel::Receiver<()>,
        stdout: &mut (impl Write + Send + ?Sized),
        stderr: &mut (impl Write + Send + ?Sized),
    ) -> eyre::Result<TestStatus> {
        let Invocation {
            binary,
            arguments,
            forwarded_env,
        } = invocation;
        binary.require_platform(self.platform())?;
        // The simulator renders through the host display, so a sleeping
        // display stops it presenting the frames the GPU suites assert on
        // (#3529). The assertion lives exactly as long as this runner.
        let _display_awake = host
            .command("caffeinate")
            .wrap_err("failed to spawn caffeinate")?
            .args(["-d", "-u", "-w"])
            .arg(std::process::id().to_string())
            .kill_on_drop(true)
            .spawn()
            .wrap_err("failed to hold the host display awake with caffeinate")?;
        let simulator = AppleSimulator::scan(host)
            .await
            .wrap_err("failed to list simulators")?
            .into_iter()
            .find(|simulator| simulator.udid() == self.udid)
            .ok_or_else(|| {
                eyre!(
                    "WATERUI_IOS_SIM_UDID names {} but no simulator reports it in `simctl list`",
                    self.udid
                )
            })?;
        let data_path = simulator.data_path().to_path_buf();

        // Boot if needed and wait for it to be ready; a no-op when already
        // booted. Concurrent invocations race to boot a shut-down device and
        // the loser fails with "Unable to boot device in current state:
        // Booted", so the boot runs under its own lock beside the launch
        // lock. nextest parses this runner's stdout, so bootstatus chatter
        // stays on stderr.
        {
            let _boot_lock =
                DestinationLock::acquire(&data_path.join(".waterui-nextest-ios-sim-boot.lock"))
                    .await?;
            let output = host
                .output("xcrun", ["simctl", "bootstatus", self.udid.as_str(), "-b"])
                .await
                .wrap_err("failed to run simctl bootstatus")?;
            stderr.write_all(&output.stdout)?;
            stderr.write_all(&output.stderr)?;
            if !output.status.success() {
                return Ok(TestStatus::new(exit_code(output.status)));
            }
        }

        // The bare route: a `--list` invocation, or a binary with no
        // application manifest, is spawned as a bare process with inherited
        // stdio; the spawned process's exit status is simctl's.
        if arguments.is_listing() || binary.manifest().is_none() {
            let mut command = host
                .command("xcrun")
                .wrap_err("failed to spawn xcrun simctl")?;
            command
                .args(["simctl", "spawn", self.udid.as_str()])
                .arg(binary.path())
                .args(arguments.as_slice());
            for (key, value) in forwarded_env.pairs() {
                command.env(format!("SIMCTL_CHILD_{key}"), value);
            }
            let status = command
                .status()
                .await
                .wrap_err("failed to run simctl spawn")?;
            return Ok(pass_through_exit(status));
        }

        let manifest = binary
            .manifest()
            .expect("the app route only runs a marked binary");
        let _lock =
            DestinationLock::acquire(&data_path.join(".waterui-nextest-ios-sim.lock")).await?;

        let bundle = HarnessBundle::stage(binary, manifest, "iphonesimulator").await?;
        codesign_path(host, bundle.layout().app_path(), "-", None, None).await?;

        // Install unless the device already holds a byte-identical
        // executable (the manifest is embedded in it).
        if !installed_matches(host, &self.udid, &bundle, manifest.executable()).await? {
            install_simulator_artifact(host, &self.udid, bundle.layout().app_path()).await?;
        }

        let status = StatusFile::host_absolute(&binary.state_dir()?);
        let mut env: Vec<(String, String)> = forwarded_env.pairs().to_vec();
        env.push((STATUS_PATH_VARIABLE.to_string(), status.env_value()));

        let launch_arguments: Vec<String> = arguments
            .as_slice()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        let running = crate::apple::device::launch_simulator_app_monitored(
            host,
            &self.udid,
            bundle.bundle_id(),
            &env,
            &launch_arguments,
        )?;
        forward_run(running, interrupts, stdout, stderr).await?;

        let path = status.local_path(&binary.state_dir()?);
        let content = std::fs::read(&path).ok();
        let _ = std::fs::remove_file(&path);
        StatusFile::conclude(content.as_deref(), bundle.bundle_id())
    }
}

/// Whether the simulator already holds this bundle's exact executable.
async fn installed_matches(
    host: &Host,
    udid: &str,
    bundle: &HarnessBundle,
    executable: &str,
) -> eyre::Result<bool> {
    let output = host
        .output(
            "xcrun",
            [
                "simctl",
                "get_app_container",
                udid,
                bundle.bundle_id(),
                "app",
            ],
        )
        .await
        .wrap_err("failed to run simctl get_app_container")?;
    if !output.status.success() {
        return Ok(false);
    }
    let container = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let installed = Path::new(&container).join(executable);
    let staged = bundle.layout().executable_file(executable);
    Ok(installed.is_file() && std::fs::read(&installed).ok() == std::fs::read(&staged).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::test_runner::{ForwardedEnv, Invocation, TestArguments, TestBinary};
    use crate::toolchain::testing::TestMachine;

    /// The `simctl list --json` a scan parses.
    fn simctl_devices_json(machine: &TestMachine) -> String {
        format!(
            r#"{{"devices":{{"com.apple.CoreSimulator.SimRuntime.iOS-18-0":[{{"udid":"SIM-UDID","name":"test-sim","isAvailable":true,"deviceTypeIdentifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-16","state":"Shutdown","dataPath":"{0}","logPath":"{0}/logs"}}]}},"runtimes":[{{"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-18-0","version":"18.0"}}]}}"#,
            machine.dir("sim-data").display()
        )
    }

    #[test]
    fn the_bare_route_passes_the_spawned_exit_through() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        machine.install("caffeinate");
        machine.respond("XCRUN_SIMCTL_DEVICES", &simctl_devices_json(&machine));
        let log = machine.file("fake.log", "");
        let binary = crate::apple::test_runner::binary::tests::stage_binary(
            machine.root(),
            MachOPlatform::IOSSIMULATOR.0,
            None,
        );
        let host = machine.host([
            ("WATERUI_FAKE_LOG", log.as_os_str()),
            (
                "WATERUI_FAKE_SIMCTL_SPAWN_STATUS",
                std::ffi::OsStr::new("17"),
            ),
        ]);
        let (_tx, interrupts) = smol::channel::unbounded();
        let binary = TestBinary::read(&binary).expect("read fixture");
        let arguments = TestArguments::new(vec!["suite::case".into()]);
        let forwarded = ForwardedEnv::new([]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = smol::block_on(IosSimulator::new("SIM-UDID".to_string()).run(
            &host,
            &Invocation {
                binary,
                arguments,
                forwarded_env: forwarded,
            },
            interrupts,
            &mut stdout,
            &mut stderr,
        ))
        .expect("the spawn route returns a status");
        assert_eq!(status.code(), 17);
    }

    #[test]
    fn the_app_route_skips_install_when_the_executables_match() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        machine.install("caffeinate");
        machine.install("codesign");
        machine.respond("XCRUN_SIMCTL_DEVICES", &simctl_devices_json(&machine));
        let plist = crate::apple::test_runner::binary::tests::manifest_plist();
        let binary_path = crate::apple::test_runner::binary::tests::stage_binary(
            machine.root(),
            MachOPlatform::IOSSIMULATOR.0,
            Some(&plist),
        );
        // The simulator already holds this exact executable.
        let container = machine.dir("sim-app-container");
        std::fs::write(
            container.join("harness"),
            std::fs::read(&binary_path).expect("fixture bytes"),
        )
        .expect("installed executable");
        machine.respond(
            "XCRUN_SIMCTL_APP_CONTAINER",
            &container.display().to_string(),
        );
        let log = machine.file("fake.log", "");
        let host = machine.host([("WATERUI_FAKE_LOG", log.as_os_str())]);
        let (_tx, interrupts) = smol::channel::unbounded();
        let binary = TestBinary::read(&binary_path).expect("read fixture");
        let arguments = TestArguments::new(vec!["suite::case".into()]);
        let forwarded = ForwardedEnv::new([]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = smol::block_on(IosSimulator::new("SIM-UDID".to_string()).run(
            &host,
            &Invocation {
                binary,
                arguments,
                forwarded_env: forwarded,
            },
            interrupts,
            &mut stdout,
            &mut stderr,
        ))
        .expect("the app route returns the harness status");
        assert_eq!(status.code(), 0);
        // Identical executable: no install ran.
        let log = std::fs::read_to_string(&log).expect("read log");
        assert!(
            !log.contains("simctl install"),
            "install was skipped, log: {log}"
        );
        assert!(log.contains("simctl launch"), "the app launched: {log}");
    }
}
