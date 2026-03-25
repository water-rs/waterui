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

const MAX_AUTO_FIX_PASSES: usize = 3;

struct DoctorSummary {
    all_ok: bool,
    fixable_items: Vec<DoctorItem>,
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

async fn install_fixable_items(items: Vec<DoctorItem>) {
    for item in items {
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
                Ok(()) => success!("Installed {name}"),
                Err(e) => error!("Failed to install {name}: {e}"),
            }
        }
    }
}

fn collect_remaining_missing(items: Vec<DoctorItem>) -> (usize, usize, Vec<DoctorItem>) {
    let mut remaining_missing = 0usize;
    let mut remaining_manual = 0usize;
    let mut remaining_fixable = Vec::new();

    for item in items {
        if item.status != CheckStatus::Missing {
            continue;
        }
        remaining_missing += 1;
        let fixable = item.is_fixable();
        if !fixable {
            remaining_manual += 1;
        }
        print_missing_item(&item);
        if fixable {
            remaining_fixable.push(item);
        }
    }

    (remaining_missing, remaining_manual, remaining_fixable)
}

/// Run the doctor command.
pub async fn run(args: Args) -> Result<()> {
    header!("Checking development environment...");

    let items = run_diagnostics("Running diagnostics...").await;
    let summary = print_diagnostics(items);
    handle_doctor_result(args.fix, summary).await;
    Ok(())
}

async fn run_diagnostics(message: &str) -> Vec<DoctorItem> {
    let spinner = shell::spinner(message);
    let items = doctor().await;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    items
}

fn print_diagnostics(items: Vec<DoctorItem>) -> DoctorSummary {
    let mut summary = DoctorSummary {
        all_ok: true,
        fixable_items: Vec::new(),
    };

    for item in items {
        match item.status {
            CheckStatus::Ok => success!("{}", item.name),
            CheckStatus::Missing => {
                summary.all_ok = false;
                print_missing_item(&item);
                if item.is_fixable() {
                    summary.fixable_items.push(item);
                }
            }
            CheckStatus::Skipped => print_skipped_item(&item),
        }
    }
    summary
}

fn print_skipped_item(item: &DoctorItem) {
    if let Some(msg) = &item.message {
        line!("  - {} (skipped: {})", item.name, msg);
    } else {
        line!("  - {} (skipped)", item.name);
    }
}

async fn handle_doctor_result(fix: bool, summary: DoctorSummary) {
    line!();
    if summary.all_ok {
        success!("All checks passed!");
    } else if fix {
        if summary.fixable_items.is_empty() {
            note!("Nothing to fix automatically. Please fix issues manually.");
        } else {
            attempt_auto_fix_loop(summary.fixable_items).await;
        }
    } else if !summary.fixable_items.is_empty() {
        warn!(
            "Some checks failed. Run `water doctor --fix` to attempt automatic fixes for {} issue(s).",
            summary.fixable_items.len()
        );
    } else {
        warn!("Some checks failed. See above for details.");
    }
}

async fn attempt_auto_fix_loop(mut pending_fixable: Vec<DoctorItem>) {
    let mut pass = 1usize;

    loop {
        print_auto_fix_header(pass, pending_fixable.len());
        install_fixable_items(pending_fixable).await;
        line!();

        let verification_items = run_diagnostics("Re-running diagnostics...").await;
        let (remaining_missing, remaining_manual, next_fixable) =
            collect_remaining_missing(verification_items);

        if remaining_missing == 0 {
            success!("All detected issues were fixed.");
            break;
        }

        if should_stop_auto_fix(pass, remaining_missing, remaining_manual, &next_fixable) {
            break;
        }

        pending_fixable = next_fixable;
        pass += 1;
        line!();
    }
}

fn print_auto_fix_header(pass: usize, pending_count: usize) {
    if pass == 1 {
        header!("Attempting to fix {pending_count} issue(s)...");
    } else {
        header!(
            "Attempting to fix {pending_count} additional issue(s)... (pass {pass}/{MAX_AUTO_FIX_PASSES})"
        );
    }
}

fn should_stop_auto_fix(
    pass: usize,
    remaining_missing: usize,
    remaining_manual: usize,
    next_fixable: &[DoctorItem],
) -> bool {
    if next_fixable.is_empty() {
        if remaining_manual > 0 {
            warn!(
                "{remaining_missing} issue(s) remain, including {remaining_manual} issue(s) that require manual steps."
            );
            note!("Follow the [manual] next-step guidance above, then run `water doctor` again.");
        } else {
            warn!(
                "{remaining_missing} fixable issue(s) still remain. Re-run `water doctor --fix` or inspect failure logs above."
            );
        }
        return true;
    }

    if pass >= MAX_AUTO_FIX_PASSES {
        warn!("{remaining_missing} issue(s) remain after {MAX_AUTO_FIX_PASSES} auto-fix pass(es).");
        if remaining_manual > 0 {
            note!(
                "Some remaining issues require manual steps. Follow the [manual] guidance above, then re-run `water doctor --fix`."
            );
        } else {
            note!("Remaining issues are still fixable. Re-run `water doctor --fix` to continue.");
        }
        return true;
    }

    false
}
