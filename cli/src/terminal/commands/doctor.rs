//! `water doctor` command implementation.

use std::future::Future;

use clap::Args as ClapArgs;
use dialoguer::{Confirm, theme::ColorfulTheme};
use eyre::Result;

use crate::shell::Shell;
use crate::{error, header, line, note, success, warn};
use waterui_cli::toolchain::{
    Host,
    doctor::{CheckStatus, DoctorItem, DoctorItemRecord, DoctorSection, doctor, sections},
};

/// Arguments for the doctor command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Attempt to fix issues automatically.
    #[arg(long)]
    fix: bool,
    /// Answer yes to installs that modify the system outside `~/.water`
    /// (Visual Studio Build Tools, the LLVM MSI). Required to run them on a
    /// non-interactive `doctor --fix`.
    #[arg(long)]
    yes: bool,
}

const MAX_AUTO_FIX_PASSES: usize = 3;

/// The in-scope outcome of a diagnosis: optional backends' items are
/// reported but never counted here.
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
        CheckStatus::Missing if item.optional => print_optional_missing_item(shell, item),
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

/// A missing piece of a backend that is out of scope: the install hint
/// without the warning, so it neither reads as a failure nor is counted.
fn print_optional_missing_item(shell: &Shell, item: &DoctorItem) {
    if let Some(message) = &item.message {
        line!(shell, "  - {} (not installed: {message})", item.name);
    } else {
        line!(shell, "  - {} (not installed)", item.name);
    }
}

fn print_section_heading(shell: &Shell, section: &DoctorSection) {
    line!(shell);
    if section.optional {
        header!(shell, "{} (optional)", section.group.title());
    } else {
        header!(shell, "{}", section.group.title());
    }
}

/// Install every fixable item; returns the `(id, name)` of each install that
/// failed.
///
/// Every item is attempted even when an earlier install failed — the fixes
/// are independent, so one broken package manager transaction must degrade
/// only its own item, not the rest of the queue. A skipped or failed install
/// leaves the item missing, which the re-diagnosis pass observes; the loop
/// uses the returned ids to keep a failed item out of the retry set instead
/// of giving up on everything else.
///
/// `system_wide` items modify the machine outside `~/.water`, so before
/// installing one the exact payload is stated again (the item's message
/// carries what will be installed and roughly how large it is) and consent
/// is required: `--yes`, or the interactive prompt. A non-interactive run
/// without `--yes` skips them.
async fn install_fixable_items(
    shell: &Shell,
    items: Vec<DoctorItem>,
    yes: bool,
) -> Vec<(&'static str, &'static str)> {
    let mut failures = Vec::new();
    for item in items {
        let name = item.name;
        if let Some(install_fn) = item.install_fn {
            if item.system_wide
                && let Some(message) = &item.message
            {
                note!(
                    shell,
                    "{name} modifies the system outside ~/.water: {message}"
                );
            }
            if item.system_wide && !yes && !shell.is_interactive() {
                note!(
                    shell,
                    "Skipped {name}: pass `--yes` to allow system-wide installs on a non-interactive run."
                );
                continue;
            }

            let should_install = yes
                || !shell.is_interactive()
                || Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!("Install {name}?"))
                    .default(true)
                    .interact()
                    .unwrap_or(false);

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
                    failures.push((item.id, name));
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
        if item.status != CheckStatus::Missing || item.optional {
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
    run_with_diagnose(shell, args.fix, args.yes, diagnose).await
}

/// The doctor orchestration with an injectable diagnosis step.
///
/// `diagnose` produces a fresh report each call — the `--fix` loop re-runs it
/// between passes so a fix that unblocks another item is observed, and a test
/// can script the sequence of reports.
async fn run_with_diagnose<F, Fut>(shell: &Shell, fix: bool, yes: bool, diagnose: F) -> Result<()>
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = Vec<DoctorItem>> + Send,
{
    header!(shell, "Checking development environment...");

    let items = run_diagnostics(shell, &diagnose, "Running diagnostics...").await;
    let summary = print_diagnostics(shell, items);
    handle_doctor_result(shell, fix, yes, &diagnose, summary).await;
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

/// Print the report and collect the in-scope summary. JSON output keeps the
/// `ids::ALL` emission order; the terminal groups items under one heading
/// per [`DoctorSection`].
fn print_diagnostics(shell: &Shell, items: Vec<DoctorItem>) -> DoctorSummary {
    let mut summary = DoctorSummary {
        all_ok: true,
        fixable_items: Vec::new(),
    };

    if shell.is_json() {
        for item in items {
            emit_item(shell, &item);
            summarize_item(&mut summary, item);
        }
        return summary;
    }

    for section in sections(items) {
        print_section_heading(shell, &section);
        for item in section.items {
            emit_item(shell, &item);
            summarize_item(&mut summary, item);
        }
    }
    summary
}

fn summarize_item(summary: &mut DoctorSummary, item: DoctorItem) {
    if item.status != CheckStatus::Missing || item.optional {
        return;
    }
    summary.all_ok = false;
    if item.is_fixable() {
        summary.fixable_items.push(item);
    }
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
    yes: bool,
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
            attempt_auto_fix_loop(shell, diagnose, summary.fixable_items, yes).await;
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
    yes: bool,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = Vec<DoctorItem>> + Send,
{
    let mut pass = 1usize;
    // Every install that ever failed, so a failed item degrades out of the
    // retry set while the fixes queued behind it keep running.
    let mut failed: Vec<(&'static str, &'static str)> = Vec::new();

    loop {
        print_auto_fix_header(shell, pass, pending_fixable.len());
        failed.extend(install_fixable_items(shell, pending_fixable, yes).await);
        line!(shell);

        let verification_items =
            run_diagnostics(shell, diagnose, "Re-running diagnostics...").await;
        let (remaining_missing, remaining_manual, mut next_fixable) =
            collect_remaining_missing(shell, verification_items);
        // A system-wide item this run cannot consent to was already reported
        // as still missing; keeping it in the retry set would only re-skip
        // it on every remaining pass.
        next_fixable.retain(|item| !item.system_wide || yes || shell.is_interactive());
        // An install that failed fails deterministically on retry (the same
        // package manager, the same package list), so it leaves the retry
        // set — the re-diagnosis still reports it missing, and the failures
        // are named in the report after the loop.
        next_fixable.retain(|item| !failed.iter().any(|(id, _)| *id == item.id));

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

    if !failed.is_empty() {
        let names = failed
            .iter()
            .map(|(_, name)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        warn!(
            shell,
            "{} installation(s) failed: {names}. Inspect the errors above, then re-run `water doctor --fix`.",
            failed.len()
        );
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
    use waterui_cli::toolchain::doctor::{BoxedInstallFn, DoctorGroup};

    use super::*;

    /// JSON-mode shells are never interactive, so installs never prompt.
    fn test_shell() -> Shell {
        Shell::new(true)
    }

    fn ok_item(id: &'static str) -> DoctorItem {
        DoctorItem {
            id,
            name: id,
            group: DoctorGroup::Helpers,
            optional: false,
            system_wide: false,
            status: CheckStatus::Ok,
            message: None,
            install_fn: None,
        }
    }

    fn manual_item(id: &'static str) -> DoctorItem {
        DoctorItem {
            id,
            name: id,
            group: DoctorGroup::Helpers,
            optional: false,
            system_wide: false,
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
            group: DoctorGroup::Helpers,
            optional: false,
            system_wide: false,
            status: CheckStatus::Missing,
            message: Some(String::from("fixable")),
            install_fn: Some(install_fn),
        }
    }

    /// A missing, fixable item that modifies the system outside `~/.water`
    /// — it needs `--yes` (or an interactive prompt) to install.
    fn system_wide_item(id: &'static str, calls: &Arc<Mutex<Vec<&'static str>>>) -> DoctorItem {
        let mut item = fixable_item(id, calls, Ok(()));
        item.system_wide = true;
        item
    }

    /// A missing, fixable item of an out-of-scope backend; its install must
    /// never run.
    fn optional_item(id: &'static str, calls: &Arc<Mutex<Vec<&'static str>>>) -> DoctorItem {
        let mut item = fixable_item(id, calls, Ok(()));
        item.group = DoctorGroup::Android;
        item.optional = true;
        item
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
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
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
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
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
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(
            taken(&installs),
            vec!["a"],
            "a failed install leaves the retry set instead of being retried"
        );
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn fix_loop_continues_past_a_failed_install() {
        let installs = install_log();
        // "a" fails while "b" succeeds and "c" only surfaces on re-diagnosis;
        // the failure must degrade just "a", not abandon "b" and "c".
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![
                fixable_item("a", &installs, Err(eyre::eyre!("boom"))),
                fixable_item("b", &installs, Ok(())),
            ],
            vec![
                fixable_item("a", &installs, Err(eyre::eyre!("boom"))),
                ok_item("b"),
                fixable_item("c", &installs, Ok(())),
            ],
            vec![
                fixable_item("a", &installs, Err(eyre::eyre!("boom"))),
                ok_item("b"),
                ok_item("c"),
            ],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(
            taken(&installs),
            vec!["a", "b", "c"],
            "every fixable item is attempted; only the failed one is never retried"
        );
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn fix_loop_manual_issues_stop_without_installing() {
        let (diagnose, diagnose_calls) = scripted(vec![vec![manual_item("m")]]);
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
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
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
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
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
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
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs).len(), MAX_AUTO_FIX_PASSES);
        assert_eq!(
            diagnose_calls.load(Ordering::SeqCst),
            MAX_AUTO_FIX_PASSES + 1
        );
    }

    /// An optional backend's missing pieces are reported but never fixed or
    /// counted: with only optional items missing the run is all-ok and
    /// diagnoses once.
    #[test]
    fn fix_loop_ignores_optional_items() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![vec![
            ok_item("a"),
            optional_item("android-sdk", &installs),
        ]]);
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs), [] as [&str; 0]);
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn without_fix_flag_no_installs_run() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![vec![fixable_item("a", &installs, Ok(()))]]);
        smol::block_on(run_with_diagnose(&test_shell(), false, false, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs), [] as [&str; 0]);
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 1);
    }

    /// A system-wide install on a non-interactive run without `--yes` is
    /// skipped — and dropped from the retry set so it is not re-skipped on
    /// every pass.
    #[test]
    fn system_wide_item_is_skipped_without_yes() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![system_wide_item("a", &installs)],
            vec![system_wide_item("a", &installs)],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, false, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs), [] as [&str; 0]);
        assert_eq!(
            diagnose_calls.load(Ordering::SeqCst),
            2,
            "one install pass plus one re-diagnosis, then the loop stops"
        );
    }

    /// `--yes` is the consent a non-interactive run cannot ask for: the
    /// system-wide install runs.
    #[test]
    fn system_wide_item_installs_with_yes() {
        let installs = install_log();
        let (diagnose, diagnose_calls) = scripted(vec![
            vec![system_wide_item("a", &installs)],
            vec![ok_item("a")],
        ]);
        smol::block_on(run_with_diagnose(&test_shell(), true, true, diagnose))
            .expect("doctor run must succeed");
        assert_eq!(taken(&installs), vec!["a"]);
        assert_eq!(diagnose_calls.load(Ordering::SeqCst), 2);
    }
}
