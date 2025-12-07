use std::{
    collections::HashMap,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use color_eyre::eyre;
use smol::stream::Stream;

use crate::platform::Platform;

/// Options for running an application on a device
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    env_vars: HashMap<String, String>,
}

impl RunOptions {
    /// Create new run options
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an environment variable to be set when running the application
    pub fn insert_env_var(&mut self, key: String, value: String) {
        self.env_vars.insert(key, value);
    }
}

/// Trait representing a device (e.g., emulator, simulator, physical device)
pub trait Device: Send {
    /// Lanuch the device emulator or simulator.
    ///
    /// If the device is a physical device, this should do nothing.
    fn launch(&self) -> impl Future<Output = eyre::Result<()>> + Send;

    /// Run the given artifact on the device with the specified options.
    fn run(
        &self,
        artifact: &Path,
        options: RunOptions,
    ) -> impl Future<Output = Result<Running, FailToRun>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum FailToRun {
    /// Failed to launch the device.
    #[error("Failed to launch device: {0}")]
    Launch(eyre::Report),
    /// Failed to run the application on the device.
    #[error("Failed to run application on device: {0}")]
    Run(eyre::Report),
}

/// A handler representing a running application on a device
pub struct Running {}

impl Stream for Running {
    type Item = DeviceEvent;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        todo!()
    }
}

pub enum DeviceEvent {
    Started,
    Stopped,
    Log { level: u8, message: String },
    Crashed(String),
}
