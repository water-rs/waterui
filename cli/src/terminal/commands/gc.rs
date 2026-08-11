//! `water gc` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use color_eyre::eyre::Result;

use crate::shell::Shell;
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

    /// Report what the managed build cache is holding without removing anything.
    #[arg(long)]
    dry_run: bool,
}

/// Run the gc command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    match args.command {
        GcCommand::BuildCache(args) => run_build_cache(shell, args).await,
    }
}

async fn run_build_cache(shell: &Shell, args: BuildCacheArgs) -> Result<()> {
    let project_path = crate::project_path::canonicalize(&args.path)?;

    if args.dry_run {
        return report_build_cache_usage(shell, &project_path).await;
    }

    header!(
        shell,
        "Cleaning stale managed build cache for {}...",
        project_path.display()
    );

    match water_dir::cleanup_stale_build_caches_for_project(&project_path).await? {
        BuildCacheGcOutcome::Ran(summary) => {
            success!(
                shell,
                "Build-cache GC complete: scanned {} cache entries, removed {} stale entries",
                summary.scanned_entries,
                summary.removed_entries
            );
        }
        BuildCacheGcOutcome::SkippedAlreadyRunning => {
            note!(
                shell,
                "Build-cache GC skipped because another cleanup process is already running"
            );
        }
    }

    Ok(())
}

async fn report_build_cache_usage(shell: &Shell, project_path: &std::path::Path) -> Result<()> {
    header!(shell, "Managed build cache usage");

    let report = water_dir::survey_build_cache_usage(project_path).await?;
    if report.entries.is_empty() {
        note!(shell, "The managed build cache is empty");
        return Ok(());
    }

    for entry in &report.entries {
        let label = if entry.active {
            " (active)"
        } else if entry.stale {
            " (stale)"
        } else {
            ""
        };
        note!(
            shell,
            "{:>10}  {}{label}",
            human_bytes(entry.bytes),
            entry.project_root.display()
        );
    }

    success!(
        shell,
        "{} across {} entries, {} reclaimable by `water gc build-cache`",
        human_bytes(report.total_bytes),
        report.entries.len(),
        human_bytes(report.reclaimable_bytes)
    );
    Ok(())
}

/// Render a byte count the way a person reads disk usage.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    #[expect(
        clippy::cast_precision_loss,
        reason = "a human-readable size only needs three significant digits"
    )]
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::human_bytes;

    #[test]
    fn renders_sizes_at_human_scale() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(human_bytes(114 * 1024 * 1024 * 1024), "114.0 GB");
    }
}
