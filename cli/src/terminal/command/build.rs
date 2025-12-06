//! Terminal command for `water build`.
//!
//! This is a thin wrapper around the library's build functionality.
//! All heavy logic is in `waterui_cli::build`.

use std::path::PathBuf;

use clap::Args;
use color_eyre::eyre::{Result, bail};
use serde::Serialize;

use crate::ui;
use waterui_cli::{
    build::{self, BuildOptions},
    output,
    project::Project,
};

#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Target triple to build for (e.g., aarch64-linux-android, aarch64-apple-ios)
    pub target: String,

    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Disable sccache acceleration
    #[arg(long)]
    pub no_sccache: bool,

    /// Enable experimental mold linker integration (Linux hosts only)
    #[arg(long)]
    pub mold: bool,
}

#[derive(Debug, Serialize)]
pub struct BuildReport {
    pub project_dir: String,
    pub target: String,
    pub profile: String,
    pub artifact_path: String,
    pub artifact_kind: String,
}

waterui_cli::impl_report!(BuildReport, |r| {
    format!(
        "Built {} for {} at {}",
        r.artifact_kind, r.target, r.artifact_path
    )
});

pub fn run(args: BuildArgs) -> Result<BuildReport> {
    // Validate target format
    if !build::is_valid_target(&args.target) {
        bail!(
            "Invalid target triple: '{}'\n\n\
             Target should be in format: <arch>-<vendor>-<os>[-<env>]\n\
             Examples:\n\
             - aarch64-linux-android\n\
             - aarch64-apple-ios\n\
             - x86_64-apple-darwin",
            args.target
        );
    }

    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));

    let project = Project::open(&project_dir)?;

    // Playground projects cannot be built directly via CLI
    if project.config().is_playground() {
        bail!(
            "Cannot build playground projects directly.\n\n\
             Playground projects automatically build when running. Use `water run` instead."
        );
    }

    let options = BuildOptions::new()
        .with_release(args.release)
        .with_speedups(!args.no_sccache, args.mold);

    let result = build::build_for_target(&project, &args.target, &options)?;

    if !output::global_output_format().is_json() {
        ui::success(format!(
            "Built {} for {} at {}",
            result.artifact_kind.as_str(),
            result.target,
            result.artifact_path.display()
        ));
    }

    Ok(BuildReport {
        project_dir: project_dir.display().to_string(),
        target: result.target,
        profile: result.profile,
        artifact_path: result.artifact_path.display().to_string(),
        artifact_kind: result.artifact_kind.as_str().to_string(),
    })
}
