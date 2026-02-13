//! `water assets` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use color_eyre::eyre::{Result, bail};

use crate::{header, line, success, warn};
use waterui_cli::{assets, project::Project};

/// Arguments for the assets command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Assets operation.
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate assets layout against the strict cross-platform convention.
    Doctor(ProjectArgs),
    /// Migrate legacy assets layout into the strict convention.
    Migrate(ProjectArgs),
}

/// Common project arguments.
#[derive(ClapArgs, Debug)]
struct ProjectArgs {
    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

/// Run the assets command.
pub async fn run(args: Args) -> Result<()> {
    match args.command {
        Command::Doctor(project_args) => run_doctor(project_args).await,
        Command::Migrate(project_args) => run_migrate(project_args).await,
    }
}

async fn run_doctor(args: ProjectArgs) -> Result<()> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path).await?;

    header!("Validating assets layout...");
    let report = assets::doctor_assets(&project).await?;

    line!("Assets root: {}", report.assets_root.display());
    line!("Scanned files: {}", report.scanned_files);

    if !report.warnings.is_empty() {
        line!();
        warn!("Warnings ({}):", report.warnings.len());
        for issue in &report.warnings {
            line!("  - {}: {}", issue.path.display(), issue.message);
            if let Some(suggestion) = &issue.suggestion {
                line!("    suggestion: {suggestion}");
            }
        }
    }

    if !report.errors.is_empty() {
        line!();
        warn!("Errors ({}):", report.errors.len());
        for issue in &report.errors {
            line!("  - {}: {}", issue.path.display(), issue.message);
            if let Some(suggestion) = &issue.suggestion {
                line!("    suggestion: {suggestion}");
            }
        }
        line!();
        line!(
            "Run `water assets migrate --path {}` to auto-migrate legacy files.",
            project_path.display()
        );
        bail!("Assets layout is invalid");
    }

    line!();
    success!("Assets layout is valid");
    Ok(())
}

async fn run_migrate(args: ProjectArgs) -> Result<()> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path).await?;

    header!("Migrating assets layout...");
    let report = assets::migrate_assets(&project).await?;

    line!("Assets root: {}", report.assets_root.display());
    line!("Moved files: {}", report.moved.len());
    line!("Unchanged files: {}", report.skipped.len());

    if !report.moved.is_empty() {
        line!();
        success!("Moved files:");
        for (from, to) in &report.moved {
            line!("  - {} -> {}", from.display(), to.display());
        }
    }

    line!();
    success!("Assets migration complete");
    Ok(())
}
