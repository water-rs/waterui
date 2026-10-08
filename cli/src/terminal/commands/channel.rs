//! Framework channel inspection and explicit selection.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use eyre::{Result, bail};
use waterui_cli::framework::FrameworkChannel;
use waterui_cli::project::{Manifest, Project};

use crate::line;
use crate::shell::Shell;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Select or update dev, nightly or stable; omit to show the current selection.
    channel: Option<FrameworkChannel>,
    /// Pin dev to an exact commit of the framework repository's dev history.
    #[arg(long, value_name = "SHA")]
    rev: Option<String>,
    /// Project directory containing Water.toml.
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

impl Args {
    /// The channel to select, or `None` to report the current selection.
    /// `--rev` pins a commit of the dev channel — the certified channels are
    /// already exact revisions and an omitted channel selects nothing — so
    /// the flag is valid only beside `dev`.
    fn selection(&self) -> Result<Option<FrameworkChannel>> {
        if self.rev.is_some() && self.channel != Some(FrameworkChannel::Dev) {
            bail!("--rev pins a commit of the dev channel: `water channel dev --rev <sha>`");
        }
        Ok(self.channel)
    }
}

pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let manifest = if let Some(channel) = args.selection()? {
        Project::select_channel(
            &waterui_cli::toolchain::Host::current(),
            &args.path,
            channel,
            args.rev.as_deref(),
        )
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

#[cfg(test)]
mod tests {
    use super::Args;
    use clap::Parser as _;
    use waterui_cli::framework::FrameworkChannel;

    /// The channel `Args` wrapped in a `Parser` so tests exercise the real
    /// flag surface instead of constructing the clap struct field by field.
    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    fn channel_args(argv: &[&str]) -> Args {
        let mut full = vec!["water-channel"];
        full.extend_from_slice(argv);
        TestCli::try_parse_from(full)
            .expect("channel args parse")
            .args
    }

    #[test]
    fn dev_accepts_a_rev_pin() {
        let args = channel_args(&["dev", "--rev", "225259c80"]);
        assert_eq!(args.rev.as_deref(), Some("225259c80"));
        assert_eq!(args.selection().unwrap(), Some(FrameworkChannel::Dev));
    }

    #[test]
    fn rev_requires_the_dev_channel() {
        for argv in [
            &["--rev", "225259c80"][..],
            &["nightly", "--rev", "225259c80"][..],
            &["stable", "--rev", "225259c80"][..],
        ] {
            let args = channel_args(argv);
            assert!(args.selection().is_err(), "--rev beside {argv:?} must fail");
        }
    }

    #[test]
    fn a_flagless_channel_still_selects() {
        assert_eq!(
            channel_args(&["dev"]).selection().unwrap(),
            Some(FrameworkChannel::Dev)
        );
        assert_eq!(channel_args(&[]).selection().unwrap(), None);
    }
}
