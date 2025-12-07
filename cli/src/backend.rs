use core::fmt::{Debug, Display};
use std::{future::Future, path::PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};
use target_lexicon::Triple;

use crate::{
    android::backend::AndroidBackend,
    device::Device,
    project::{Project, manifest::AppleBackend},
    toolchain::Toolchain,
};

pub struct BuildOptions {}
pub struct PackageOptions {}

/// A backend for building and packaging `WaterUI` projects.
///
/// Implementors should provide methods for initializing, cleaning,
/// checking requirements, and scanning for available devices.
pub trait Backend: Display + Debug + Send + Sync {
    type Toolchain: Toolchain;
    type Device: Device;

    fn init(project: &Project) -> impl Future<Output = eyre::Result<()>> + Send;

    fn toolchain(&self) -> &Self::Toolchain;

    fn clean(&self, project: &Project) -> impl Future<Output = eyre::Result<()>> + Send;

    fn scan(&self) -> impl Future<Output = eyre::Result<Vec<Self::Device>>> + Send;

    fn triple(&self) -> Triple;

    fn build(&self, project: &Project, options: &BuildOptions) -> eyre::Result<PathBuf>;

    fn package(&self, project: &Project, options: &PackageOptions) -> eyre::Result<PathBuf>;
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Backends {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apple: Option<AppleBackend>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android: Option<AndroidBackend>,
}

impl Backends {
    /// Check if no backends are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.swift.is_none() && self.android.is_none()
    }

    /// Get the Apple backend, if configured.
    pub fn apple_backend(&self) -> Option<&AppleBackend> {
        self.apple.as_ref()
    }

    /// Get the Android backend, if configured.
    pub fn android_backend(&self) -> Option<&AndroidBackend> {
        self.android.as_ref()
    }
}
