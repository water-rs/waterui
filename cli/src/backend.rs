use serde::{Deserialize, Serialize};

use crate::{android::backend::AndroidBackend, apple::backend::AppleBackend, project::Project};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Backends {
    android: Option<AndroidBackend>,
    apple: Option<AppleBackend>,
}

impl Backends {
    pub const fn is_empty(&self) -> bool {
        self.android.is_none() && self.apple.is_none()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FailToInitBackend {}

pub trait Backend: Sized + Send + Sync {
    fn init(project: &Project) -> impl Future<Output = Result<Self, FailToInitBackend>> + Send;
}
