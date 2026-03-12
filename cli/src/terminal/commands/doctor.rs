//! `water doctor` command implementation.

use clap::Args as ClapArgs;
use color_eyre::eyre::Result;
use dialoguer::{Confirm, theme::ColorfulTheme};

use crate::shell;
use crate::{error, header, line, note, success, warn};
use waterui_cli::toolchain::doctor::{CheckStatus, DoctorItem, doctor};

/// Arguments for the doctor command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Attempt to fix issues automatically.
    #[arg(long)]
    fix: bool,
}

fn print_missing_item(item: &DoctorItem) {
    let is_fixable = item.is_fixable();
    if let Some(message) = &item.message {
        if is_fixable {
            warn!("{} ({message}) [fixable]", item.name);
        } else {
            warn!("{} ({message}) [manual]", item.name);
        }
    } else if is_fixable {
        warn!("{} [fixable]", item.name);
    } else {
        warn!("{} [manual]", item.name);
    }
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
                print_missing_item(&item);
                if item.is_fixable() {
                    fixable_items.push(item);
                }
            }
            CheckStatus::Skipped => {
                if let Some(msg) = &item.message {
                    line!("  - {} (skipped: {})", item.name, msg);
                } else {
                    line!("  - {} (skipped)", item.name);
                }
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
            let mut round = 1usize;
            let mut current_fixable_items = fixable_items;
            loop {
                header!(
                    "Attempting to fix {} issue(s){}...",
                    current_fixable_items.len(),
                    if round > 1 {
                        format!(" (round {round})")
                    } else {
                        String::new()
                    }
                );

                let mut installed_any = false;
                for item in current_fixable_items {
                    let name = item.name;
                    if let Some(install_fn) = item.install_fn {
                        let should_install = if shell::is_interactive() {
                            Confirm::with_theme(&ColorfulTheme::default())
                                .with_prompt(format!("Install {name}?"))
                                .default(true)
                                .interact()
                                .unwrap_or(false)
                        } else {
                            true
                        };

                        if !should_install {
                            note!("Skipped installation for {name}");
                            continue;
                        }

                        let spinner = shell::spinner(format!("Installing {name}..."));
                        let result = install_fn().await;
                        if let Some(pb) = spinner {
                            pb.finish_and_clear();
                        }

                        match result {
                            Ok(()) => {
                                installed_any = true;
                                success!("Installed {name}");
                            }
                            Err(e) => {
                                error!("Failed to install {name}: {e}");
                            }
                        }
                    }
                }

                line!();
                let verify_spinner = shell::spinner("Re-running diagnostics...");
                let verification_items = doctor().await;
                if let Some(pb) = verify_spinner {
                    pb.finish_and_clear();
                }

                let mut remaining_missing = 0usize;
                let mut remaining_manual = 0usize;
                let mut next_fixable_items = Vec::new();
                for item in verification_items {
                    if item.status == CheckStatus::Missing {
                        remaining_missing += 1;
                        let is_fixable = item.is_fixable();
                        print_missing_item(&item);
                        if is_fixable {
                            next_fixable_items.push(item);
                        } else {
                            remaining_manual += 1;
                        }
                    }
                }

                if remaining_missing == 0 {
                    success!("All detected issues were fixed.");
                    break;
                }
                if next_fixable_items.is_empty() {
                    warn!(
                        "{remaining_missing} issue(s) remain, including {remaining_manual} issue(s) that require manual steps."
                    );
                    note!(
                        "Follow the [manual] next-step guidance above, then run `water doctor` again."
                    );
                    break;
                }
                if !installed_any {
                    warn!(
                        "{} fixable issue(s) remain, but no automatic fixes completed successfully in the last round.",
                        next_fixable_items.len()
                    );
                    note!(
                        "Re-run `water doctor --fix` after resolving the failed installation above."
                    );
                    break;
                }
                if round >= 4 {
                    warn!(
                        "{} fixable issue(s) still remain after multiple auto-fix rounds.",
                        next_fixable_items.len()
                    );
                    note!(
                        "Re-run `water doctor --fix` or inspect the remaining diagnostics above."
                    );
                    break;
                }

                note!(
                    "Continuing auto-fix for {} newly-unblocked issue(s)...",
                    next_fixable_items.len()
                );
                line!();
                current_fixable_items = next_fixable_items;
                round = round
                    .checked_add(1)
                    .expect("doctor fix round counter overflow");
            }
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
