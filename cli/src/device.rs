//! Device management and application running utilities for `WaterUI` CLI.

use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
};

use color_eyre::eyre;
use smol::{
    channel::{Receiver, Sender, unbounded},
    stream::Stream,
};

use crate::platform::Platform;

/// Minimum log level for streaming device logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Only errors
    Error,
    /// Warnings and errors
    Warn,
    /// Info, warnings, and errors
    #[default]
    Info,
    /// Debug and above
    Debug,
    /// All logs including verbose
    Verbose,
}

impl LogLevel {
    /// Convert to Android logcat priority character.
    #[must_use]
    pub const fn to_android_priority(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warn => 'W',
            Self::Info => 'I',
            Self::Debug => 'D',
            Self::Verbose => 'V',
        }
    }

    /// Convert to iOS/macOS `log stream --level` argument.
    ///
    /// Apple's unified logging `log stream --level` accepts: default, info, debug
    /// - `debug` includes all messages (debug, info, default, error, fault)
    /// - `info` includes info and above
    /// - `default` includes default (notice) and above
    ///
    /// Since we want to capture errors/warnings, we need at least `default` level.
    #[must_use]
    pub const fn to_apple_level(self) -> &'static str {
        match self {
            Self::Error | Self::Warn | Self::Info => "default",
            Self::Debug | Self::Verbose => "debug",
        }
    }
}

/// Options for running an application on a device
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// # Note
    ///
    /// Android do not support environment variables yet.
    /// iOS/macOS support environment variables via `xcrun simctl launch --env`.
    ///
    /// As a workaround, we would set system property `waterui.env.<KEY>` to `<VALUE>` on Android,
    /// and read them to set environment variables in the application.
    env_vars: HashMap<String, String>,

    /// If set, stream device logs at or above this level.
    log_level: Option<LogLevel>,
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

    /// Get an iterator over the environment variables
    pub fn env_vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Set the minimum log level to stream.
    pub fn set_log_level(&mut self, level: LogLevel) {
        self.log_level = Some(level);
    }

    /// Get the log level if set.
    #[must_use]
    pub const fn log_level(&self) -> Option<LogLevel> {
        self.log_level
    }
}

/// Represents a build artifact to be run on a device
#[derive(Debug)]
pub struct Artifact {
    bundle_id: String,
    path: PathBuf,
}

impl Artifact {
    /// Create a new artifact
    #[must_use]
    pub fn new(bundle_id: impl Into<String>, path: PathBuf) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            path,
        }
    }

    /// Get the bundle identifier of the artifact
    #[must_use]
    pub const fn bundle_id(&self) -> &str {
        self.bundle_id.as_str()
    }

    /// Get the path to the artifact
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Trait representing a device (e.g., emulator, simulator, physical device)
pub trait Device: Send {
    /// Associated platform type for the device.
    type Platform: Platform;
    /// Lanuch the device emulator or simulator.
    ///
    /// If the device is a physical device, this should do nothing.
    fn launch(&self) -> impl Future<Output = eyre::Result<()>> + Send;

    /// Run the given artifact on the device with the specified options.
    fn run(
        &self,
        artifact: Artifact,
        options: RunOptions,
    ) -> impl Future<Output = Result<Running, FailToRun>> + Send;

    /// Get the platform this device belongs to.
    fn platform(&self) -> Self::Platform;
}

/// Represents a running application on a device.
///
/// Drop the `Running` to terminate the application
pub struct Running {
    sender: Sender<DeviceEvent>,
    receiver: Receiver<DeviceEvent>,
    on_drop: Vec<Box<dyn FnOnce() + Send>>,
}

impl Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running").finish_non_exhaustive()
    }
}

impl Running {
    /// Create a new `Running` instance
    #[allow(clippy::missing_panics_doc)]
    pub fn new(on_drop: impl FnOnce() + Send + 'static) -> (Self, Sender<DeviceEvent>) {
        let (sender, receiver) = unbounded();
        sender.try_send(DeviceEvent::Started).unwrap(); // `unwrap` is safe here, as we just created the channel
        (
            Self {
                sender: sender.clone(),
                receiver,
                on_drop: vec![Box::new(on_drop)],
            },
            sender,
        )
    }

    pub fn retain<T: Send + 'static>(&mut self, value: T) {
        self.on_drop.push(Box::new(move || {
            drop(value);
        }));
    }
}

impl Stream for Running {
    type Item = DeviceEvent;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // SAFETY: We only project to the `receiver` field, which is safe to pin
        // because we never move out of it and the other fields don't affect pinning
        let receiver = unsafe { &mut self.get_unchecked_mut().receiver };
        unsafe { std::pin::Pin::new_unchecked(receiver) }.poll_next(cx)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.sender.try_send(DeviceEvent::Stopped);
        for f in self.on_drop.drain(..) {
            f();
        }
    }
}

/// Errors that can occur when running an application on a device
#[derive(Debug, thiserror::Error)]
pub enum FailToRun {
    /// Invalid artifact provided.
    #[error("Invalid artifact")]
    InvalidArtifact,

    /// Failed to install the application on the device.
    #[error("Failed to install application on device: {0}")]
    Install(eyre::Report),

    /// Failed to launch the device.
    #[error("Failed to launch device: {0}")]
    Launch(eyre::Report),
    /// Failed to run the application on the device.
    #[error("Failed to run application on device: {0}")]
    Run(eyre::Report),

    /// Failed to package the artifacts.
    #[error("Failed to package the artifacts: {0}")]
    Package(eyre::Report),

    /// Failed to build the project.
    #[error("Failed to build the project: {0}")]
    Build(eyre::Report),

    /// Failed to start hot reload server.
    #[error("Failed to start hot reload server: {0}")]
    HotReload(crate::debug::hot_reload::FailToLaunch),

    /// Application crashed.
    #[error("Application crashed: {0}")]
    Crashed(String),
}

/// Events emitted by a running application on a device
#[derive(Debug)]
pub enum DeviceEvent {
    /// Application has started
    Started,
    /// Application has stopped by CLI
    Stopped,
    /// Standard output from the application
    Stdout {
        /// The output message
        message: String,
    },

    /// Standard error from the application
    Stderr {
        /// The error message
        message: String,
    },
    /// Standard log from the application
    Log {
        /// The log level
        level: tracing::Level,
        /// The log message
        message: String,
    },

    /// Unexpected exit of the application, may triggered by user quitting
    Exited,

    /// Application crashed with error message
    Crashed(String),
}

/// Represents the kind of device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// Simulator device
    Simulator,
    /// Physical device
    Physical,
}

/// Represents the state of a device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Device is booted and ready
    Booted,
    /// Device is shutdown
    Shutdown,
    /// Device is disconnected (e.g., physical device unplugged)
    Disconnected,
}
