//! `water gc` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use color_eyre::eyre::Result;

use crate::{header, note, success};
use waterui_cli::water_dir::{self, BuildCacheGcOutcome};

/// Arguments for the gc command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// GC action.
    #[command(subcommand)]
    command: GcCommand,
}

#[derive(Subcommand, Debug)]
enum GcCommand {
    /// Remove stale managed build-cache entries under `~/.water/build_cache`.
    BuildCache(BuildCacheArgs),
}

/// Arguments for `water gc build-cache`.
#[derive(ClapArgs, Debug)]
struct BuildCacheArgs {
    /// Project directory whose managed build cache should be preserved as active.
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

/// Run the gc command.
pub async fn run(args: Args) -> Result<()> {
    match args.command {
        GcCommand::BuildCache(args) => run_build_cache(args).await,
    }
}

async fn run_build_cache(args: BuildCacheArgs) -> Result<()> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    header!(
        "Cleaning stale managed build cache for {}...",
        project_path.display()
    );

    match water_dir::cleanup_stale_build_caches_for_project(&project_path).await? {
        BuildCacheGcOutcome::Ran(summary) => {
            success!(
                "Build-cache GC complete: scanned {} cache entries, removed {} stale entries",
                summary.scanned_entries,
                summary.removed_entries
            );
        }
        BuildCacheGcOutcome::SkippedAlreadyRunning => {
            note!("Build-cache GC skipped because another cleanup process is already running");
        }
    }

    Ok(())
}
