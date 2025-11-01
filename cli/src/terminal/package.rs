use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use dialoguer::{Select, theme::ColorfulTheme};
use serde::Serialize;
use tracing::{debug, info};

use crate::{android, apple, config::Config, output, util};

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
    fn display_name(self) -> &'static str {
        match self {
            PackagePlatform::Android => "Android",
            PackagePlatform::Ios => "iOS",
        }
    }
}

pub fn run(args: PackageArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;
    let is_json = output::global_output_format().is_json();

    if args.all && args.platform.is_some() {
        bail!("Cannot specify a platform when using --all");
    }

    let available = available_platforms(&config);
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
                let android_config = config.backends.android.as_ref().ok_or_else(|| {
                    eyre!(
                        "Android backend not configured for this project. Add it to Water.toml or recreate the project with the Android backend.",
                    )
                })?;
                let apk_path = android::build_android_apk(
                    &project_dir,
                    android_config,
                    args.release,
                    args.skip_native,
                    false,
                    &config.package.bundle_identifier,
                )?;
                info!("Android package ready: {}", apk_path.display());
                if is_json {
                    artifacts.push(PackageArtifact {
                        platform: "android".to_string(),
                        path: apk_path.display().to_string(),
                    });
                }
            }
            PackagePlatform::Ios => {
                let swift_config = config.backends.swift.as_ref().ok_or_else(|| {
                    eyre!(
                        "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the Apple backend.",
                    )
                })?;
                let app_bundle = package_ios(&project_dir, swift_config, args.release)?;
                info!("iOS package ready: {}", app_bundle.display());
                if is_json {
                    artifacts.push(PackageArtifact {
                        platform: "ios".to_string(),
                        path: app_bundle.display().to_string(),
                    });
                }
            }
        }
    }

    if is_json {
        let report = PackageReport {
            project_dir: project_dir.display().to_string(),
            release: args.release,
            skip_native: args.skip_native,
            artifacts,
        };
        output::emit_json(&report)?;
    }

    Ok(())
}

#[derive(Serialize)]
struct PackageReport {
    project_dir: String,
    release: bool,
    skip_native: bool,
    artifacts: Vec<PackageArtifact>,
}

#[derive(Serialize)]
struct PackageArtifact {
    platform: String,
    path: String,
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

fn package_ios(
    project_dir: &Path,
    swift_config: &crate::config::Swift,
    release: bool,
) -> Result<PathBuf> {
    apple::ensure_macos_host("iOS packaging")?;
    util::require_tool(
        "xcodebuild",
        "Install Xcode and command line tools (xcode-select --install)",
    )?;

    let project = apple::resolve_xcode_project(project_dir, swift_config)?;
    let derived_root = apple::derived_data_dir(project_dir);
    apple::prepare_derived_data_dir(&derived_root)?;

    let configuration = if release { "Release" } else { "Debug" };

    let mut build_cmd = apple::xcodebuild_base(&project, configuration, &derived_root);
    build_cmd.arg("-destination").arg("generic/platform=iOS");

    info!("Building iOS app with xcodebuild...");
    debug!("Executing command: {:?}", build_cmd);
    let status = build_cmd.status().context("failed to invoke xcodebuild")?;
    if !status.success() {
        bail!("xcodebuild failed with status {}", status);
    }

    let products_dir = derived_root.join(format!("Build/Products/{}-iphoneos", configuration));
    let app_bundle = products_dir.join(format!("{}.app", project.scheme));
    if !app_bundle.exists() {
        bail!("Expected app bundle at {}", app_bundle.display());
    }

    Ok(app_bundle)
}
