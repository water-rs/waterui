use core::fmt::{Debug, Display};

use color_eyre::eyre;

pub trait ToolchainIssue: Debug + Display + 'static + Send + Sync {
    fn suggestion(&self) -> String {
        "No suggestion available.".to_string()
    }
    fn fix(&self) -> eyre::Result<()> {
        Err(eyre::eyre!("No automatic fix available."))
    }
}

pub type AnyToolchainIssue = Box<dyn ToolchainIssue>;
