//! Framework channel inspection and explicit selection.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use color_eyre::eyre::{Result, bail};
use waterui_cli::framework::FrameworkChannel;
use waterui_cli::project::{Manifest, Project};

use crate::line;
use crate::shell::Shell;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Select or update dev, nightly or stable; omit to show the current selection.
    channel: Option<FrameworkChannel>,
    /// Project directory containing Water.toml.
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let manifest = if let Some(channel) = args.channel {
        Project::select_channel(&args.path, channel)
            .await?
            .manifest()
            .clone()
    } else {
        Manifest::open(args.path.join("Water.toml")).await?
    };
    if let Some(framework) = manifest.framework {
        line!(shell, "{}", toml::to_string_pretty(&framework)?);
    } else if let Some(path) = manifest.waterui_path {
        line!(shell, "Local framework: {path}");
    } else {
        bail!("no framework channel is selected; use water channel dev, nightly or stable");
    }
    Ok(())
}
