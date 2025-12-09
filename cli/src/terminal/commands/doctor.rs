//! `water doctor` command implementation.

use clap::Args as ClapArgs;
use color_eyre::eyre::Result;

use crate::shell;
use crate::{header, line, success, warn};
use waterui_cli::toolchain::doctor::{CheckStatus, doctor};

/// Arguments for the doctor command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    // No arguments for now
}

/// Run the doctor command.
pub async fn run(_args: Args) -> Result<()> {
    header!("Checking development environment...");

    let spinner = shell::spinner("Running diagnostics...");
    let items = doctor().await;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    let mut all_ok = true;

    for item in items {
        match item.status {
            CheckStatus::Ok => {
                success!("{}", item.name);
            }
            CheckStatus::Missing => {
                all_ok = false;
                if let Some(msg) = &item.message {
                    warn!("{} ({})", item.name, msg);
                } else {
                    warn!("{}", item.name);
                }
            }
            CheckStatus::Skipped => {
                line!("  ○ {} (skipped)", item.name);
            }
        }
    }

    line!();
    if all_ok {
        success!("All checks passed!");
    } else {
        warn!("Some checks failed. See above for details.");
    }

    Ok(())
}
