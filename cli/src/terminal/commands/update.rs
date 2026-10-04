//! `water update` — self-update receipt-installed binaries in place, and
//! name the owning package manager's command for every other channel.

use clap::Args as ClapArgs;
use eyre::Result;
use waterui_cli::{
    self_update::{self, CheckOutcome, UpdateOutcome},
    toolchain::Host,
};

use crate::shell::Shell;
use crate::{line, note, success};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Report the newest release without installing it.
    #[arg(long)]
    check: bool,
}

pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let host = Host::current();
    if args.check {
        match self_update::check(&host).await? {
            CheckOutcome::UpToDate { current } => {
                success!(shell, "water {current} is up to date");
            }
            CheckOutcome::Available {
                current,
                latest,
                command,
            } => {
                note!(shell, "water {latest} is available (installed: {current})");
                line!(shell, "Update with `{command}`");
            }
        }
    } else {
        let report = self_update::update(&host).await?;
        line!(
            shell,
            "Install source: {} ({})",
            report.source.label(),
            report.install_dir.display()
        );
        match report.outcome {
            UpdateOutcome::Updated {
                previous,
                installed,
            } => match previous {
                Some(previous) => success!(shell, "Updated water {previous} → {installed}"),
                None => success!(shell, "Updated water to {installed}"),
            },
            UpdateOutcome::UpToDate { current } => {
                success!(shell, "water {current} is already up to date");
            }
            UpdateOutcome::ExternallyManaged { command } => {
                note!(
                    shell,
                    "this `water` is managed by a package manager — nothing was changed"
                );
                line!(shell, "Update it with `{command}`");
            }
        }
    }
    Ok(())
}
