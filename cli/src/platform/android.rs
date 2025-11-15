use std::path::PathBuf;

use color_eyre::eyre::Result;

use crate::{
    backend::{
        Backend,
        android::{AndroidBackend, build_android_apk},
    },
    doctor::AnyToolchainIssue,
    platform::Platform,
    project::{Android, Project},
};

#[derive(Debug, Clone)]
pub struct AndroidPlatform {
    backend: AndroidBackend,
    config: Android,
    skip_native: bool,
    hot_reload: bool,
    enable_sccache: bool,
    mold_requested: bool,
}

impl AndroidPlatform {
    #[must_use]
    pub const fn new(
        config: Android,
        skip_native: bool,
        hot_reload: bool,
        enable_sccache: bool,
        mold_requested: bool,
    ) -> Self {
        Self {
            backend: AndroidBackend,
            config,
            skip_native,
            hot_reload,
            enable_sccache,
            mold_requested,
        }
    }
}

impl Platform for AndroidPlatform {
    type ToolchainIssue = AnyToolchainIssue;
    type Backend = AndroidBackend;

    fn target_triple(&self) -> &'static str {
        "aarch64-linux-android"
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        self.backend.check_requirements(project)
    }

    fn package(&self, project: &Project, release: bool) -> Result<PathBuf> {
        build_android_apk(
            project.root(),
            &self.config,
            release,
            self.skip_native,
            self.hot_reload,
            project.bundle_identifier(),
            &project.config().package.name,
            self.enable_sccache,
            self.mold_requested,
        )
    }

    fn backend(&self) -> &Self::Backend {
        &self.backend
    }
}
