//! `water doctor` command implementation.

use std::future::Future;

use clap::Args as ClapArgs;
use dialoguer::{Confirm, theme::ColorfulTheme};
use eyre::Result;

use crate::shell::Shell;
use crate::{error, header, line, note, success, warn};
use waterui_cli::toolchain::{
    Host,
    doctor::{CheckStatus, DoctorItem, DoctorItemRecord, doctor},
};

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

fn emit_item(shell: &Shell, item: &DoctorItem) {
    if shell.is_json() {
        if let Ok(json) = serde_json::to_string(&DoctorItemRecord::from(item)) {
            let _ = shell.json_raw(&json);
        }
        return;
    }

    match item.status {
        CheckStatus::Ok => success!(shell, "{}", item.name),
        CheckStatus::Missing => print_missing_item(shell, item),
        CheckStatus::Skipped => print_skipped_item(shell, item),
    }
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

/// Install every fixable item; returns how many installations failed.
///
/// A skipped or failed install leaves the item missing, which the
/// re-diagnosis pass observes — the count exists only so the loop can stop
/// retrying when fixes themselves are failing.
async fn install_fixable_items(shell: &Shell, items: Vec<DoctorItem>) -> usize {
    let mut failures = 0usize;
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
                Err(e) => {
                    failures += 1;
                    error!(shell, "Failed to install {name}: {e}");
                }
            }
        }
    }
    failures
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
        emit_item(shell, &item);
        if fixable {
            remaining_fixable.push(item);
        }
    }

    (remaining_missing, remaining_manual, remaining_fixable)
}

/// Run the doctor command against the real machine.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let host = Host::current();
    let diagnose = || {
        let host = host.clone();
        async move { doctor(&host).await }
    };
    run_with_diagnose(shell, args.fix, diagnose).await
}

/// The doctor orchestration with an injectable diagnosis step.
///
/// `diagnose` produces a fresh report each call — the `--fix` loop re-runs it
/// between passes so a fix that unblocks another item is observed, and a test
/// can script the sequence of reports.
async fn run_with_diagnose<F, Fut>(shell: &Shell, fix: bool, diagnose: F) -> Result<()>
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = Vec<DoctorItem>> + Send,
{
    header!(shell, "Checking development environment...");

    let items = run_diagnostics(shell, &diagnose, "Running diagnostics...").await;
    let summary = print_diagnostics(shell, items);
    handle_doctor_result(shell, fix, &diagnose, summary).await;
    Ok(())
}

async fn run_diagnostics<F, Fut>(shell: &Shell, diagnose: &F, message: &str) -> Vec<DoctorItem>
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = Vec<DoctorItem>> + Send,
{
    let spinner = shell.spinner(message);
    let items = diagnose().await;
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
        if item.status == CheckStatus::Missing {
            summary.all_ok = false;
            if item.is_fixable() {
                emit_item(shell, &item);
                summary.fixable_items.push(item);
                continue;
            }
        }
        emit_item(shell, &item);
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

async fn handle_doctor_result<F, Fut>(
    shell: &Shell,
    fix: bool,
    diagnose: &F,
    summary: DoctorSummary,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = Vec<DoctorItem>> + Send,
{
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
            attempt_auto_fix_loop(shell, diagnose, summary.fixable_items).await;
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

async fn attempt_auto_fix_loop<F, Fut>(
    shell: &Shell,
    diagnose: &F,
    mut pending_fixable: Vec<DoctorItem>,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = Vec<DoctorItem>> + Send,
{
    let mut pass = 1usize;

    loop {
        print_auto_fix_header(shell, pass, pending_fixable.len());
        let failures = install_fixable_items(shell, pending_fixable).await;
        line!(shell);

        let verification_items =
            run_diagnostics(shell, diagnose, "Re-running diagnostics...").await;
        let (remaining_missing, remaining_manual, next_fixable) =
            collect_remaining_missing(shell, verification_items);

        if remaining_missing == 0 {
            success!(shell, "All detected issues were fixed.");
            break;
        }

        if failures > 0 {
            warn!(
                shell,
                "Stopping auto-fix: {failures} installation(s) failed this pass. Inspect the errors above, then re-run `water doctor --fix`."
            );
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use color_eyre::eyre;
    use waterui_cli::toolchain::doctor::BoxedInstallFn;

    use super::*;

    /// JSON-mode shells are never interactive, so installs never prompt.
    fn test_shell() -> Shell {
        Shell::new(true)
    }

    fn ok_item(id: &'static str) -> DoctorItem {
        DoctorItem {
            id,
            name: id,
            status: CheckStatus::Ok,
            message: None,
            install_fn: None,
        }
    }

    fn manual_item(id: &'static str) -> DoctorItem {
        DoctorItem {
            id,
            name: id,
            status: CheckStatus::Missing,
            message: Some(String::from("manual steps required")),
            install_fn: None,
        }
    }

    /// A missing item whose install records its id into `calls` and yields
    /// `outcome`.
    fn fixable_item(
        id: &'static str,
        calls: &Arc<Mutex<Vec<&'static str>>>,
        outcome: Result<()>,
    ) -> DoctorItem {
        let calls = Arc::clone(calls);
        let install_fn: BoxedInstallFn = Box::new(move || {
            calls.lock().expect("install call log").push(id);
            Box::pin(async move { outcome })
        });
        DoctorItem {
            id,
            name: id,
            status: CheckStatus::Missing,
            message: Some(String::from("fixable")),
            install_fn: Some(install_fn),
        }
    }

    /// A scripted `diagnose` step: replays one report per call.
    type ScriptedDiagnose =
        Box<dyn Fn() -> Pin<Box<dyn Future<Output = Vec<DoctorItem>> + Send>> + Sync>;

    /// A diagnose closure replaying `reports` in order; each entry is one
    /// diagnosis. Calling more times than scripted fails the test, so the
    /// call count itself is the assertion on stop conditions.
    fn scripted(reports: Vec<Vec<DoctorItem>>) -> (ScriptedDiagnose, Arc<AtomicUsize>) {
        let queue = Mutex::new(VecDeque::from(reports));
        let calls = Arc::new(AtomicUsize::new(0));
        let diagnose_calls = Arc::clone(&calls);
        let diagnose = move || {
            diagnose_calls.fetch_add(1, Ordering::SeqCst);
            let next = queue
                .lock()
                .expect("diagnose script lock")
                .pop_front()
                .expect("diagnose was called more times than the script provides");
            Box::pin(async move { next }) as Pin<Box<dyn Future<Output = Vec<DoctorItem>> + Send>>
        };
        (Box::new(diagnose), calls)
    }

    fn install_log() -> Arc<Mutex<Vec<&'static str>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn taken(log: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
        log.lock().expect("install call log").clone()
    }

    #[test]
    fn all_ok_reports_never_attempts_fixes() {
        let (diagnose, diagnose_calls) = scripted(vec![vec![ok_item("a"), ok_item("b")]]);
        smol::block_on(run_with_diagnose(&test_shell(), true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fix_loop_repairs_and_rediagnoses_once() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![fixable_item("a", &installs, Ok(()))],
            vec![ok_item("a")],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs), vec!["a"]);
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn fix_loop_stops_after_install_failure() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![fixable_item("a", &installs, Err(eyre::eyre!("boom")))],
            vec![fixable_item("a", &installs, Ok(()))],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(
            taken(&installs),
            vec!["a"],
            "a failed install must stop the loop instead of retrying"
        );
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn fix_loop_manual_issues_stop_without_installing() {
        let (diagnose, diagnose_calls) = scripted(vec![vec![manual_item("m")]]);
        smol::block_on(run_with_diagnose(&test_shell(), true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fix_loop_stops_when_only_manual_issues_remain() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![fixable_item("a", &installs, Ok(())), manual_item("m")],
            vec![ok_item("a"), manual_item("m")],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs), vec!["a"]);
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn fix_loop_rediagnosis_can_surface_new_fixable_items() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![fixable_item("a", &installs, Ok(())), manual_item("m")],
            vec![
                ok_item("a"),
                fixable_item("b", &installs, Ok(())),
                manual_item("m"),
            ],
            vec![ok_item("a"), ok_item("b"), manual_item("m")],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs), vec!["a", "b"]);
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn fix_loop_caps_at_three_passes() {
        let installs = install_log();
        // Install "succeeds" but the item stays missing on re-diagnosis —
        // the loop must give up after MAX_AUTO_FIX_PASSES, not spin forever.
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![fixable_item("a", &installs, Ok(()))],
            vec![fixable_item("a", &installs, Ok(()))],
            vec![fixable_item("a", &installs, Ok(()))],
            vec![fixable_item("a", &installs, Ok(()))],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs).len(), MAX_AUTO_FIX_PASSES);
        assert_eq!(
            diagnose_calls.load(Ordering::SeqCst),
            MAX_AUTO_FIX_PASSES + 1
        );
    }

    #[test]
    fn without_fix_flag_no_installs_run() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![vec![fixable_item("a", &installs, Ok(()))]]);
        smol::block_on(run_with_diagnose(&test_shell(), false, diagnose))
            .expect("doctor run must succeed");
        assert!(taken(&installs).is_empty());
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 1);
    }
}
