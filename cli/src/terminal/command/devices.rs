use clap::Args;
use color_eyre::eyre::Result;
use waterui_cli::device::DeviceInfo;

#[derive(Args, Debug, Default)]
pub struct DevicesArgs;

/// List connected simulators and devices.
///
/// # Errors
/// Returns an error if device discovery fails or JSON output cannot be emitted.
pub fn run(_args: DevicesArgs) -> Result<Vec<DeviceInfo>> {
    waterui_cli::device::list_devices()
}
