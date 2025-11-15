use std::path::PathBuf;

use color_eyre::eyre::{Result, eyre};

use crate::{
    backend::{Backend, tui::TuiBackend},
    doctor::AnyToolchainIssue,
    platform::Platform,
    project::{Project, Tui},
};

#[derive(Debug, Clone)]
pub struct TuiPlatform {
    backend: TuiBackend,
    config: Tui,
}

impl TuiPlatform {
    #[must_use]
    pub const fn new(config: Tui) -> Self {
        Self {
            backend: TuiBackend,
            config,
        }
    }
}

impl Platform for TuiPlatform {
    type ToolchainIssue = AnyToolchainIssue;
    type Backend = TuiBackend;

    fn target_triple(&self) -> &'static str {
        ""
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        self.backend.check_requirements(project)
    }

    fn package(&self, _project: &Project, _release: bool) -> Result<PathBuf, eyre::Report> {
        Err(eyre!("Packaging not supported for the TUI backend yet"))
    }

    fn backend(&self) -> &Self::Backend {
        &self.backend
    }
}
