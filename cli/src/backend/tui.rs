use color_eyre::eyre::Result;

use crate::{
    backend::Backend,
    doctor::AnyToolchainIssue,
    project::{Project, Tui},
};

#[derive(Debug, Clone, Copy)]
pub struct TuiBackend;

impl Backend for TuiBackend {
    type ToolchainIssue = AnyToolchainIssue;

    fn init(&self, _project: &Project, _dev: bool) -> Result<()> {
        Ok(())
    }

    fn is_existing(&self, _project: &Project) -> bool {
        true
    }

    fn clean(&self, _project: &Project) -> Result<()> {
        Ok(())
    }

    fn check_requirements(&self, _project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        Ok(())
    }
}

pub fn prepare_tui_project(_project: &Project, _config: &Tui) -> Result<()> {
    Ok(())
}
