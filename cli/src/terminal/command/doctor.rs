use clap::Args;
use color_eyre::eyre::Result;
use serde::Serialize;
use waterui_cli::doctor::toolchain::{
    self, CheckMode, CheckTarget, FixApplication, FixMode, FixSuggestion, Section,
};

#[derive(Args, Debug, Default)]
pub struct DoctorArgs {
    /// Attempt to fix required issues after running checks
    #[arg(long)]
    pub fix: bool,
}

/// Run toolchain diagnostics and optionally apply fixes.
///
/// # Errors
/// Returns an error if environment checks or fix applications fail.
///
/// # Panics
/// Panics only if the bundled progress template is invalid; this indicates a programming
/// error.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub fn run(args: DoctorArgs) -> Result<DoctorReport> {
    let targets = doctor_targets();
    let mut sections = Vec::new();
    let mut fix_suggestions = Vec::new();

    for target in &targets {
        if let Some(outcome) = toolchain::perform_check(CheckMode::Full, *target) {
            let (section, mut section_fixes) = outcome.into_parts();
            sections.push(section);
            fix_suggestions.append(&mut section_fixes);
        }
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

    let applied_fixes = if args.fix {
        Some(toolchain::apply_fixes(
            fix_suggestions.clone(),
            FixMode::Automatic,
        )?)
    } else {
        None
    };

    Ok(DoctorReport {
        status,
        sections,
        suggestions: fix_suggestions,
        applied_fixes,
    })
}

fn doctor_targets() -> Vec<CheckTarget> {
    let mut targets = vec![CheckTarget::Rust];
    if cfg!(target_os = "macos") {
        targets.push(CheckTarget::Swift);
    }
    targets.push(CheckTarget::Android);
    targets
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub status: DoctorStatus,
    pub sections: Vec<Section>,
    pub suggestions: Vec<FixSuggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_fixes: Option<Vec<FixApplication>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}
