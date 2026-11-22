//! `water test-runner` — the cargo/nextest target runner plumbing for the
//! Apple destinations. Hidden: it is a pure function of its inputs for
//! cargo to call, not a user command.

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use eyre::{Context as _, eyre};
use smol::channel::Receiver;

use waterui_cli::apple::test_runner::{
    DeviceIdentity, ForwardedEnv, Invocation as RunnerInvocation, IosDevice, IosSimulator,
    MacCatalyst, Run, TestArguments, TestBinary,
};
use waterui_cli::toolchain::Host;

/// The environment variables the terminal layer reads.
const SIM_UDID: &str = "WATERUI_IOS_SIM_UDID";
const DEVICE: &str = "WATERUI_IOS_DEVICE";
const DEVICE_BUNDLE_ID: &str = "WATERUI_IOS_DEVICE_BUNDLE_ID";
const DEVICE_TEAM_ID: &str = "WATERUI_IOS_DEVICE_TEAM_ID";

/// `water test-runner <destination> <test-binary> [test args…]`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    destination: Destination,
}

#[derive(Subcommand, Debug)]
enum Destination {
    /// iOS Simulator; the device comes from `WATERUI_IOS_SIM_UDID`.
    Simulator(Invocation),
    /// Physical iOS device; the device comes from `WATERUI_IOS_DEVICE` and
    /// the signing identity from `WATERUI_IOS_DEVICE_BUNDLE_ID` and
    /// `WATERUI_IOS_DEVICE_TEAM_ID`.
    Device(Invocation),
    /// Mac Catalyst; the host Mac launches the staged `.app` through
    /// `LaunchServices`.
    Catalyst(Invocation),
}

#[derive(ClapArgs, Debug)]
struct Invocation {
    /// The test binary cargo built.
    binary: PathBuf,
    /// Arguments forwarded to the test process.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    arguments: Vec<OsString>,
}

fn required(host: &Host, name: &str, guidance: &str) -> eyre::Result<String> {
    host.env(name)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| eyre!("{name} is unset — {guidance}"))
}

/// Runs one target-runner invocation. Returns the harness status byte.
pub async fn run(args: &Args, interrupts: Receiver<()>) -> eyre::Result<u8> {
    let host = Host::current();
    let (invocation, destination): (&Invocation, Box<dyn Run + Send>) = match &args.destination {
        Destination::Simulator(invocation) => (
            invocation,
            Box::new(IosSimulator::new(required(
                &host,
                SIM_UDID,
                "this runner requires the UDID of a simulator the caller owns and creates \
                 (CI provisions one per run). Create one, e.g.:\n  xcrun simctl create \
                 dev.waterui-tests <SimDeviceType> <SimRuntime>",
            )?)),
        ),
        Destination::Device(invocation) => (
            invocation,
            Box::new(
                IosDevice::resolve(
                    &host,
                    &required(
                        &host,
                        DEVICE,
                        "this runner never picks a device implicitly — set it to the CoreDevice \
                         identifier, UDID, or name of the paired device to run on",
                    )?,
                    DeviceIdentity {
                        bundle_id: required(
                            &host,
                            DEVICE_BUNDLE_ID,
                            "the bundle identifier every device launch is signed under",
                        )?,
                        team_id: required(
                            &host,
                            DEVICE_TEAM_ID,
                            "the development team identifier the installed profile is selected \
                             under",
                        )?,
                    },
                )
                .await?,
            ),
        ),
        Destination::Catalyst(invocation) => (invocation, Box::new(MacCatalyst::new())),
    };
    let binary = TestBinary::read(&invocation.binary)?;
    let arguments = TestArguments::new(invocation.arguments.clone());
    let forwarded = ForwardedEnv::new(
        host.env(ForwardedEnv::REFERENCE_METRICS)
            // The old script forwarded the metrics path only when set
            // *and* non-empty.
            .filter(|value| !value.is_empty())
            .map(|value| {
                (
                    ForwardedEnv::REFERENCE_METRICS.to_string(),
                    value.to_string_lossy().into_owned(),
                )
            }),
    );
    let invocation = RunnerInvocation {
        binary,
        arguments,
        forwarded_env: forwarded,
    };
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let out: &mut (dyn Write + Send) = &mut stdout;
    let err: &mut (dyn Write + Send) = &mut stderr;
    let status = destination
        .run(&host, &invocation, interrupts, out, err)
        .await
        .wrap_err("the test-runner invocation failed")?;
    Ok(status.code())
}
