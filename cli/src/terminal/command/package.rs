use std::path::PathBuf;

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Result, bail, eyre};
use dialoguer::{Select, theme::ColorfulTheme};
use serde::Serialize;

use crate::ui;
use waterui_cli::{
    output,
    platform::{
        android::AndroidPlatform,
        apple::{ApplePlatform, AppleTarget},
    },
    project::{Config, Project},
};

#[derive(Args, Debug)]
pub struct PackageArgs {
    /// Target platform to package
    #[arg(value_enum)]
    pub platform: Option<PackagePlatform>,

    /// Package all configured platforms
    #[arg(long)]
    pub all: bool,

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
    Ios,
}

impl PackagePlatform {
    const fn display_name(self) -> &'static str {
        match self {
            Self::Android => "Android",
            Self::Ios => "iOS",
        }
    }
}

/// Package the configured targets for distribution.
///
/// # Errors
/// Returns an error if user input fails, required backends are missing, or packaging fails.
///
/// # Panics
/// Panics if the current working directory cannot be read when `--project` is omitted.
#[allow(clippy::needless_pass_by_value)]
pub fn run(args: PackageArgs) -> Result<PackageReport> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let project = Project::open(&project_dir)?;
    let config = project.config();
    let is_json = output::global_output_format().is_json();

    if args.all && args.platform.is_some() {
        bail!("Cannot specify a platform when using --all");
    }

    let available = available_platforms(config);
    if available.is_empty() {
        bail!(
            "No packageable platforms configured. Add a backend in Water.toml or run 'water add-backend'.",
        );
    }

    let platforms = if args.all {
        available
    } else if let Some(platform) = args.platform {
        if !available.contains(&platform) {
            bail!(
                "{} backend not configured for this project. Add it to Water.toml or recreate the project with this backend.",
                platform.display_name(),
            );
        }
        vec![platform]
    } else {
        if is_json {
            bail!(
                "JSON output requires specifying --platform or --all to avoid interactive prompts."
            );
        }
        let options: Vec<String> = available
            .iter()
            .map(|platform| platform.display_name().to_string())
            .collect();
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a platform to package")
            .items(&options)
            .default(0)
            .interact()?;
        vec![available[selection]]
    };

    let mut artifacts = Vec::new();

    for platform in platforms {
        match platform {
            PackagePlatform::Android => {
                let android_config = config.backends.android.clone().ok_or_else(|| {
                    eyre!(
                        "Android backend not configured for this project. Add it to Water.toml or recreate the project with the Android backend.",
                    )
                })?;
                let platform_impl =
                    AndroidPlatform::new(android_config, args.skip_native, false, true, false);
                let artifact = project
                    .package(&platform_impl, args.release)
                    .map_err(|err| eyre!(err))?;
                if !is_json {
                    ui::success(format!("Android package ready: {}", artifact.display()));
                }
                artifacts.push(PackageArtifact {
                    platform: "android".to_string(),
                    path: artifact.display().to_string(),
                });
            }
            PackagePlatform::Ios => {
                let swift_config = config.backends.swift.clone().ok_or_else(|| {
                    eyre!(
                        "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the Apple backend.",
                    )
                })?;
                let platform_impl = ApplePlatform::new(swift_config, AppleTarget::IosDevice);
                let artifact = project
                    .package(&platform_impl, args.release)
                    .map_err(|err| eyre!(err))?;
                if !is_json {
                    ui::success(format!("iOS package ready: {}", artifact.display()));
                }
                artifacts.push(PackageArtifact {
                    platform: "ios".to_string(),
                    path: artifact.display().to_string(),
                });
            }
        }
    }

    let report = PackageReport {
        project_dir: project_dir.display().to_string(),
        release: args.release,
        skip_native: args.skip_native,
        artifacts,
    };

    Ok(report)
}

#[derive(Debug, Serialize)]
pub struct PackageReport {
    pub project_dir: String,
    pub release: bool,
    pub skip_native: bool,
    pub artifacts: Vec<PackageArtifact>,
}

#[derive(Debug, Serialize)]
pub struct PackageArtifact {
    pub platform: String,
    pub path: String,
}

fn available_platforms(config: &Config) -> Vec<PackagePlatform> {
    let mut platforms = Vec::new();
    if config.backends.android.is_some() {
        platforms.push(PackagePlatform::Android);
    }
    if config.backends.swift.is_some() {
        platforms.push(PackagePlatform::Ios);
    }
    platforms
}
