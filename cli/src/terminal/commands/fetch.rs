//! `water fetch` command implementation.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use eyre::Result;

use super::TargetBackend;
use crate::shell::Shell;
use crate::{note, success, warn};
use waterui_cli::FetchOutcome;
use waterui_cli::project::{ManagedBackends, Project};

/// Arguments for the fetch command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Project directory whose fonts are fetched (defaults to the current
    /// directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Restrict the fetch to the crates one backend's builds scan for font
    /// declarations — the set that backend's `water package`/`water build`
    /// resolves. Without it the fetch covers every backend the project can
    /// build on this host, so a backend crate that cannot be produced (e.g.
    /// an unresolvable scaffold dependency graph) fails the fetch even when
    /// the caller only means to build another backend.
    #[arg(short, long, value_enum)]
    backend: Option<TargetBackend>,
}

/// Run the fetch command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path, ManagedBackends::NONE).await?;

    let spinner = shell.spinner("Fetching fonts...");
    let outcomes = match args.backend {
        Some(backend) => {
            waterui_cli::seed_font_cache_for_backend(&project, backend.lib_backend()).await
        }
        None => waterui_cli::seed_font_cache(&project).await,
    };
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    let outcomes = outcomes?;

    if outcomes.is_empty() {
        note!(shell, "No fonts are declared");
        return Ok(());
    }

    let mut unsatisfied = 0usize;
    for outcome in outcomes {
        match outcome {
            FetchOutcome::Satisfied { name, path } => {
                note!(
                    shell,
                    "Font '{name}' is already cached at {}",
                    path.display()
                );
            }
            FetchOutcome::Fetched { name, path } => {
                success!(shell, "Fetched font '{name}' to {}", path.display());
            }
            // Declarations fetching cannot fix — a name the registry does not
            // know, a missing crate-local file — come back with the same
            // report the build gives them.
            FetchOutcome::Unsatisfiable { error, .. } => {
                unsatisfied += 1;
                warn!(shell, "{error:#}");
            }
        }
    }

    if unsatisfied > 0 {
        eyre::bail!(
            "{unsatisfied} declared font(s) cannot be satisfied by fetching — the build \
             reports them the same way"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Args, TargetBackend};
    use clap::Parser as _;

    /// The fetch `Args` wrapped in a `Parser` so tests exercise the real
    /// flag surface instead of constructing the clap struct field by field.
    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    fn fetch_args(argv: &[&str]) -> Args {
        let mut full = vec!["water-fetch"];
        full.extend_from_slice(argv);
        TestCli::try_parse_from(full)
            .expect("fetch args parse")
            .args
    }

    /// `--backend android` must reach the scoped scan: the fetch seeds the
    /// cache for one backend's builds instead of scaffolding every backend
    /// the host could manage — which is what made the unscoped fetch fail on
    /// hosts where another backend's crate graph does not resolve.
    #[test]
    fn backend_scope_parses_to_the_shared_target_enum() {
        let args = fetch_args(&["--backend", "android"]);
        assert_eq!(
            args.backend.map(TargetBackend::lib_backend),
            Some(waterui_cli::platform::TargetBackend::Android)
        );
    }

    #[test]
    fn no_backend_keeps_the_unscoped_fetch() {
        assert_eq!(fetch_args(&[]).backend, None);
    }
}
