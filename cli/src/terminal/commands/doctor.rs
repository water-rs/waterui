//! `water doctor` command implementation.

use clap::Args as ClapArgs;
use color_eyre::eyre::Result;

use crate::shell;
use crate::{error, header, line, note, success, warn};
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
    let mut fixable_items = Vec::new();

    for item in items {
        match item.status {
            CheckStatus::Ok => {
                success!("{}", item.name);
            }
            CheckStatus::Missing => {
                all_ok = false;
                let is_fixable = item.is_fixable();
                if let Some(msg) = &item.message {
                    if is_fixable {
                        warn!("{} ({}) [fixable]", item.name, msg);
                    } else {
                        warn!("{} ({})", item.name, msg);
                    }
                } else if is_fixable {
                    warn!("{} [fixable]", item.name);
                } else {
                    warn!("{}", item.name);
                }

                // Collect fixable items
                if is_fixable {
                    fixable_items.push(item);
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
        if fixable_items.is_empty() {
            note!("Nothing to fix automatically. Please fix issues manually.");
        } else {
            header!("Attempting to fix {} issue(s)...", fixable_items.len());

            for item in fixable_items {
                let name = item.name;
                if let Some(install_fn) = item.install_fn {
                    let spinner = shell::spinner(format!("Installing {name}..."));
                    let result = install_fn().await;
                    if let Some(pb) = spinner {
                        pb.finish_and_clear();
                    }

                    match result {
                        Ok(()) => {
                            success!("Installed {name}");
                        }
                        Err(e) => {
                            error!("Failed to install {name}: {e}");
                        }
                    }
                }
            }

            line!();
            note!("Run `water doctor` again to verify fixes.");
        }
    } else if !fixable_items.is_empty() {
        warn!(
            "Some checks failed. Run `water doctor --fix` to attempt automatic fixes for {} issue(s).",
            fixable_items.len()
        );
    } else {
        warn!("Some checks failed. See above for details.");
    }

    Ok(())
}
