use clap::Args;
use color_eyre::eyre::Result;
use console::style;
use core::time::Duration;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};

use crate::toolchain::{self, CheckMode, CheckTarget, FixMode, Section};

#[derive(Args, Debug, Default)]
pub struct DoctorArgs {
    /// Attempt to fix required issues after running checks
    #[arg(long)]
    pub fix: bool,
}

pub fn run(args: DoctorArgs) -> Result<()> {
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(80));
    pb.set_style(
        ProgressStyle::with_template("{spinner:.blue} {msg}")
            .expect("template should be valid")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );

    pb.set_message(style("Preparing environment checks…").dim().to_string());
    pb.tick();

    let targets = doctor_targets();
    let total = targets.len();
    let mut sections = Vec::new();
    let mut fixes = Vec::new();

    for (index, target) in targets.iter().enumerate() {
        let step = index + 1;
        pb.set_message(progress_label(step, total, target.label()));
        pb.tick();

        if let Some(outcome) = toolchain::perform_check(CheckMode::Full, *target) {
            announce_section(&pb, step, total, outcome.section());
            let (section, mut section_fixes) = outcome.into_parts();
            sections.push(section);
            fixes.append(&mut section_fixes);
        }
    }

    pb.finish_and_clear();

    let has_failures = sections.iter().any(Section::has_failure);

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

    let mut fix_mode = None;

    if has_failures {
        let requested = if args.fix {
            true
        } else {
            println!();
            Confirm::new()
                .with_prompt("Critical issues detected. Apply automatic fixes now?")
                .default(true)
                .interact()?
        };

        if requested {
            fix_mode = Some(if args.fix {
                FixMode::Automatic
            } else {
                FixMode::Interactive
            });
        } else if !args.fix {
            println!(
                "{}",
                style("Tip: run `water doctor --fix` to attempt automatic repairs.").yellow()
            );
        }
    } else if args.fix {
        println!(
            "{}",
            style("All required checks already pass. Nothing to fix.").green()
        );
    }

    if let Some(mode) = fix_mode {
        toolchain::apply_fixes(fixes, mode)?;
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
