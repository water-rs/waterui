//! `water doctor` command implementation.

use clap::Args as ClapArgs;
use color_eyre::eyre::Result;

use crate::shell;
use crate::{header, line, note, success, warn};
use waterui_cli::toolchain::doctor::{CheckStatus, doctor};

/// Arguments for the doctor command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Attempt to fix issues automatically.
    #[arg(long)]
    fix: bool,
}

/// Run the doctor command.
pub async fn run(args: Args) -> Result<()> {
    header!("Checking development environment...");

    let spinner = shell::spinner("Running diagnostics...");
    let items = doctor().await;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    let mut all_ok = true;
    let mut fixable_count = 0;

    for item in &items {
        match item.status {
            CheckStatus::Ok => {
                success!("{}", item.name);
            }
            CheckStatus::Missing => {
                all_ok = false;
                if item.fixable {
                    fixable_count += 1;
                }
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
    } else if args.fix {
        if fixable_count > 0 {
            note!("Auto-fix is not yet implemented. Please fix issues manually.");
        } else {
            note!("Nothing to fix automatically. Please fix issues manually.");
        }
    } else if fixable_count > 0 {
        warn!("Some checks failed. Run `water doctor --fix` to attempt automatic fixes.");
    } else {
        warn!("Some checks failed. See above for details.");
    }

    Ok(())
}
