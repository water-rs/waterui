//! `water completions` command implementation.

use std::io::Write;

use clap::{Args as ClapArgs, CommandFactory};
use clap_complete::Shell as CompletionShell;
use eyre::Result;

use crate::Cli;

/// Arguments for the completions command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Shell to generate the completion script for.
    #[arg(value_enum)]
    shell: CompletionShell,
}

/// Print the completion script for the requested shell on stdout.
///
/// The script is the payload: it bypasses [`crate::shell::Shell`] so the
/// output stays byte-exact for redirection into a completions directory
/// (`water completions zsh > ~/.zfunc/_water`).
pub fn run(args: &Args) -> Result<()> {
    let mut command = Cli::command();
    let bin_name = command.get_name().to_owned();
    // Generate into memory: `clap_complete` panics on a failed write, which
    // would turn a truncated consumer (`| head`, a closing pager) into a
    // panic report.
    let mut script = Vec::new();
    clap_complete::generate(args.shell, &mut command, bin_name, &mut script);
    match std::io::stdout().write_all(&script) {
        Ok(()) => Ok(()),
        // A truncated consumer (`| head`, a closing pager) is not an error.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}
