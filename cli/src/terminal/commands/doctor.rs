//! `water doctor` command implementation.

use clap::Args as ClapArgs;
use color_eyre::eyre::Result;
use dialoguer::{Confirm, theme::ColorfulTheme};

use crate::shell::Shell;
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

fn print_missing_item(shell: &Shell, item: &DoctorItem) {
    let is_fixable = item.is_fixable();
    if let Some(message) = &item.message {
        if is_fixable {
            warn!(shell, "{} ({message}) [fixable]", item.name);
        } else {
            warn!(shell, "{} ({message}) [manual]", item.name);
        }
    } else if is_fixable {
        warn!(shell, "{} [fixable]", item.name);
    } else {
        warn!(shell, "{} [manual]", item.name);
    }
}

async fn install_fixable_items(shell: &Shell, items: Vec<DoctorItem>) {
    for item in items {
        let name = item.name;
        if let Some(install_fn) = item.install_fn {
            let should_install = if shell.is_interactive() {
                Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!("Install {name}?"))
                    .default(true)
                    .interact()
                    .unwrap_or(false)
            } else {
                true
            };

            if !should_install {
                note!(shell, "Skipped installation for {name}");
                continue;
            }

            let spinner = shell.spinner(format!("Installing {name}..."));
            let result = install_fn().await;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }

            match result {
                Ok(()) => success!(shell, "Installed {name}"),
                Err(e) => error!(shell, "Failed to install {name}: {e}"),
            }
        }
    }
}

fn collect_remaining_missing(
    shell: &Shell,
    items: Vec<DoctorItem>,
) -> (usize, usize, Vec<DoctorItem>) {
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
        print_missing_item(shell, &item);
        if fixable {
            remaining_fixable.push(item);
        }
    }

    (remaining_missing, remaining_manual, remaining_fixable)
}

/// Run the doctor command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    header!(shell, "Checking development environment...");

    let items = run_diagnostics(shell, "Running diagnostics...").await;
    let summary = print_diagnostics(shell, items);
    handle_doctor_result(shell, args.fix, summary).await;
    Ok(())
}

async fn run_diagnostics(shell: &Shell, message: &str) -> Vec<DoctorItem> {
    let spinner = shell.spinner(message);
    let items = doctor().await;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    items
}

fn print_diagnostics(shell: &Shell, items: Vec<DoctorItem>) -> DoctorSummary {
    let mut summary = DoctorSummary {
        all_ok: true,
        fixable_items: Vec::new(),
    };

    for item in items {
        match item.status {
            CheckStatus::Ok => success!(shell, "{}", item.name),
            CheckStatus::Missing => {
                summary.all_ok = false;
                print_missing_item(shell, &item);
                if item.is_fixable() {
                    summary.fixable_items.push(item);
                }
            }
            CheckStatus::Skipped => print_skipped_item(shell, &item),
        }
    }
    summary
}

fn print_skipped_item(shell: &Shell, item: &DoctorItem) {
    if let Some(msg) = &item.message {
        line!(shell, "  - {} (skipped: {})", item.name, msg);
    } else {
        line!(shell, "  - {} (skipped)", item.name);
    }
}

async fn handle_doctor_result(shell: &Shell, fix: bool, summary: DoctorSummary) {
    line!(shell);
    if summary.all_ok {
        success!(shell, "All checks passed!");
    } else if fix {
        if summary.fixable_items.is_empty() {
            note!(
                shell,
                "Nothing to fix automatically. Please fix issues manually."
            );
        } else {
            attempt_auto_fix_loop(shell, summary.fixable_items).await;
        }
    } else if !summary.fixable_items.is_empty() {
        warn!(
            shell,
            "Some checks failed. Run `water doctor --fix` to attempt automatic fixes for {} issue(s).",
            summary.fixable_items.len()
        );
    } else {
        warn!(shell, "Some checks failed. See above for details.");
    }
}

async fn attempt_auto_fix_loop(shell: &Shell, mut pending_fixable: Vec<DoctorItem>) {
    let mut pass = 1usize;

    loop {
        print_auto_fix_header(shell, pass, pending_fixable.len());
        install_fixable_items(shell, pending_fixable).await;
        line!(shell);

        let verification_items = run_diagnostics(shell, "Re-running diagnostics...").await;
        let (remaining_missing, remaining_manual, next_fixable) =
            collect_remaining_missing(shell, verification_items);

        if remaining_missing == 0 {
            success!(shell, "All detected issues were fixed.");
            break;
        }

        if should_stop_auto_fix(
            shell,
            pass,
            remaining_missing,
            remaining_manual,
            &next_fixable,
        ) {
            break;
        }

        pending_fixable = next_fixable;
        pass += 1;
        line!(shell);
    }
}

fn print_auto_fix_header(shell: &Shell, pass: usize, pending_count: usize) {
    if pass == 1 {
        header!(shell, "Attempting to fix {pending_count} issue(s)...");
    } else {
        header!(
            shell,
            "Attempting to fix {pending_count} additional issue(s)... (pass {pass}/{MAX_AUTO_FIX_PASSES})"
        );
    }
}

fn should_stop_auto_fix(
    shell: &Shell,
    pass: usize,
    remaining_missing: usize,
    remaining_manual: usize,
    next_fixable: &[DoctorItem],
) -> bool {
    if next_fixable.is_empty() {
        if remaining_manual > 0 {
            warn!(
                shell,
                "{remaining_missing} issue(s) remain, including {remaining_manual} issue(s) that require manual steps."
            );
            note!(
                shell,
                "Follow the [manual] next-step guidance above, then run `water doctor` again."
            );
        } else {
            warn!(
                shell,
                "{remaining_missing} fixable issue(s) still remain. Re-run `water doctor --fix` or inspect failure logs above."
            );
        }
        return true;
    }

    if pass >= MAX_AUTO_FIX_PASSES {
        warn!(
            shell,
            "{remaining_missing} issue(s) remain after {MAX_AUTO_FIX_PASSES} auto-fix pass(es)."
        );
        if remaining_manual > 0 {
            note!(
                shell,
                "Some remaining issues require manual steps. Follow the [manual] guidance above, then re-run `water doctor --fix`."
            );
        } else {
            note!(
                shell,
                "Remaining issues are still fixable. Re-run `water doctor --fix` to continue."
            );
        }
        return true;
    }

    false
}
