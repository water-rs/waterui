use std::path::Path;

use color_eyre::eyre;

use crate::backend::Backend;

pub enum DeviceState {
    Offline,
    Online,
}

pub trait Device: Send + Sync {
    // Run the packaged application on the device located at the specified path.
    fn run(&self, path: &Path) -> eyre::Result<()>;
    fn state(&self) -> eyre::Result<DeviceState>;
}

pub type AnyDevice = Box<dyn Device>;

pub fn scan() -> Vec<AnyDevice> {
    todo!("device scanning not implemented")
}
