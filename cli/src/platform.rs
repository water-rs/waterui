use color_eyre::eyre;

use crate::{
    doctor::{AnyToolchainIssue, ToolchainIssue},
    project::Project,
};

pub trait Platform: Send + Sync {
    type ToolchainIssue: ToolchainIssue;

    fn target_triple(&self) -> &'static str;
    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>>;
    fn package(&self, project: &Project, release: bool) -> eyre::Result<()>;
}

impl<T: Platform> Platform for &T {
    type ToolchainIssue = T::ToolchainIssue;

    fn target_triple(&self) -> &'static str {
        (*self).target_triple()
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        (*self).check_requirements(project)
    }

    fn package(&self, project: &Project, release: bool) -> eyre::Result<()> {
        (*self).package(project, release)
    }
}

pub type AnyPlatform = Box<dyn Platform<ToolchainIssue = AnyToolchainIssue>>;
