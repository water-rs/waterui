use core::fmt::{Debug, Display};
use std::future::Future;

use color_eyre::eyre;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    android::backend::AndroidBackend,
    project::{Project, manifest::AppleBackend},
};

/// A backend configured for a project.
///
///
/// `Backend` take the responsibility to build rust code.
///
/// One `Backend` may not support all platforms.
pub trait Backend: Display + Debug + Send + Sync + DeserializeOwned + Serialize {
    /// Initialize the backend within the given project.
    fn init(project: &Project) -> impl Future<Output = eyre::Result<()>> + Send;
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
