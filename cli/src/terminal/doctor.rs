use clap::Args;
use color_eyre::eyre::Result;
use console::style;
use core::time::Duration;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};

use crate::{
    output,
    toolchain::{self, CheckMode, CheckTarget, FixApplication, FixMode, FixSuggestion, Section},
};
use serde::Serialize;

#[derive(Args, Debug, Default)]
pub struct DoctorArgs {
    /// Attempt to fix required issues after running checks
    #[arg(long)]
    pub fix: bool,
}

pub fn run(args: DoctorArgs) -> Result<()> {
    let is_json = output::global_output_format().is_json();

    let mut spinner = if is_json {
        None
    } else {
        let pb = ProgressBar::new_spinner();
        pb.enable_steady_tick(Duration::from_millis(80));
        pb.set_style(
            ProgressStyle::with_template("{spinner:.blue} {msg}")
                .expect("template should be valid")
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.set_message(style("Preparing environment checks…").dim().to_string());
        pb.tick();
        Some(pb)
    };

    let targets = doctor_targets();
    let total = targets.len();
    let mut sections = Vec::new();
    let mut fix_suggestions = Vec::new();

    for (index, target) in targets.iter().enumerate() {
        let step = index + 1;
        if let Some(pb) = spinner.as_mut() {
            pb.set_message(progress_label(step, total, target.label()));
            pb.tick();
        }

        if let Some(outcome) = toolchain::perform_check(CheckMode::Full, *target) {
            if let Some(pb) = spinner.as_ref() {
                announce_section(pb, step, total, outcome.section());
            }
            let (section, mut section_fixes) = outcome.into_parts();
            sections.push(section);
            fix_suggestions.append(&mut section_fixes);
        }
    }

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    let has_failures = sections.iter().any(Section::has_failure);
    let has_warnings = sections.iter().any(Section::has_warning);
    let status = if has_failures {
        DoctorStatus::Fail
    } else if has_warnings {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };

    if !is_json {
        println!("{}", style("WaterUI doctor report").bold().underlined());
        for (index, section) in sections.iter().enumerate() {
            if index > 0 {
                println!();
            }
            for line in section.render() {
                println!("{line}");
            }
        }

        println!();
        println!("{}", style("Doctor check complete.").green());
        println!(
            "{}",
            style("Resolve ⚠ or ✘ entries to keep your toolchain healthy.").dim()
        );
    }

    let mut fix_mode = None;

    if has_failures {
        if args.fix {
            fix_mode = Some(FixMode::Automatic);
        } else if !is_json {
            println!();
            let requested = Confirm::new()
                .with_prompt("Critical issues detected. Apply automatic fixes now?")
                .default(true)
                .interact()?;

            if requested {
                fix_mode = Some(FixMode::Interactive);
            } else {
                println!(
                    "{}",
                    style("Tip: run `water doctor --fix` to attempt automatic repairs.").yellow()
                );
            }
        }
    } else if args.fix && !is_json {
        println!(
            "{}",
            style("All required checks already pass. Nothing to fix.").green()
        );
    }

    let mut applied_fixes = None;
    if let Some(mode) = fix_mode {
        let results = toolchain::apply_fixes(fix_suggestions.clone(), mode)?;
        applied_fixes = Some(results);
    } else if args.fix && !has_failures {
        applied_fixes = Some(Vec::new());
    }

    if is_json {
        let report = DoctorReport {
            status,
            sections,
            suggestions: fix_suggestions,
            applied_fixes,
        };
        output::emit_json(&report)?;
    }

    Ok(())
}

fn doctor_targets() -> Vec<CheckTarget> {
    let mut targets = vec![CheckTarget::Rust];
    if cfg!(target_os = "macos") {
        targets.push(CheckTarget::Swift);
    }
    targets.push(CheckTarget::Android);
    targets
}

fn announce_section(pb: &ProgressBar, step: usize, total: usize, section: &Section) {
    let line = format!(
        "{} {}",
        style(format!("[{step}/{total}]")).dim(),
        section.summary_line()
    );
    pb.suspend(|| println!("{line}"));
}

fn progress_label(step: usize, total: usize, message: &str) -> String {
    format!(
        "{} {}",
        style(format!("[{step}/{total}]")).dim(),
        style(message).bold()
    )
}

#[derive(Serialize)]
struct DoctorReport {
    status: DoctorStatus,
    sections: Vec<Section>,
    suggestions: Vec<FixSuggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    applied_fixes: Option<Vec<FixApplication>>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}
