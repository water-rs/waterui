use std::path::PathBuf;

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Result, eyre};
use tracing::info;

use crate::{config::Config, run::build_android_apk};

#[derive(Args, Debug)]
pub struct PackageArgs {
    /// Target platform to package
    #[arg(long, value_enum)]
    pub platform: PackagePlatform,

    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Skip running the generated build-rust.sh script for Android projects
    #[arg(long)]
    pub skip_native: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum PackagePlatform {
    Android,
}

pub fn run(args: PackageArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;

    match args.platform {
        PackagePlatform::Android => {
            let android_config = config.backends.android.as_ref().ok_or_else(|| {
                eyre!(
                    "Android backend not configured for this project. Add it to waterui.toml or recreate the project with the Android backend."
                )
            })?;
            let apk_path =
                build_android_apk(&project_dir, android_config, args.release, args.skip_native)?;
            info!("Android package ready: {}", apk_path.display());
        }
    }

    Ok(())
}
