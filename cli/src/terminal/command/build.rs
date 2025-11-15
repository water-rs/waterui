use std::path::PathBuf;

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, bail, eyre};
use serde::Serialize;

use crate::ui;
use waterui_cli::{
    backend::{
        android::{AndroidNativeBuildOptions, build_android_native_libraries},
        apple::{AppleRustBuildOptions, build_apple_static_library, ensure_macos_host},
    },
    output,
    project::Project,
};

#[derive(Subcommand, Debug)]
pub enum BuildCommands {
    /// Build the Android Rust libraries (JNI) without launching Gradle.
    Android(AndroidBuildArgs),
    /// Build the Apple static library that Xcode links against.
    Apple(AppleBuildArgs),
}

#[derive(Args, Debug)]
pub struct AndroidBuildArgs {
    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Explicit Rust target triples (comma-separated) to build
    #[arg(long, value_delimiter = ',')]
    pub targets: Vec<String>,

    /// Disable sccache acceleration
    #[arg(long)]
    pub no_sccache: bool,

    /// Enable experimental mold linker integration (Linux hosts only)
    #[arg(long)]
    pub mold: bool,
}

#[derive(Args, Debug)]
pub struct AppleBuildArgs {
    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Override the platform name that determines the Rust target (e.g. macosx, iphonesimulator)
    #[arg(long)]
    pub platform_name: Option<String>,

    /// Override the ARCHS value reported by Xcode (e.g. arm64, `x86_64`)
    #[arg(long)]
    pub arch: Option<String>,

    /// Destination directory for the static library
    #[arg(long)]
    pub output_dir: Option<PathBuf>,

    /// Disable sccache acceleration
    #[arg(long)]
    pub no_sccache: bool,

    /// Enable experimental mold linker integration (Linux hosts only)
    #[arg(long)]
    pub mold: bool,
}

#[derive(Debug, Serialize)]
pub struct BuildReport {
    pub project_dir: String,
    pub platform: String,
    pub profile: String,
    pub artifacts: Vec<BuildArtifact>,
}

#[derive(Debug, Serialize)]
pub struct BuildArtifact {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
}

pub fn run(cmd: BuildCommands) -> Result<BuildReport> {
    match cmd {
        BuildCommands::Android(args) => build_android(args),
        BuildCommands::Apple(args) => build_apple(args),
    }
}

fn build_android(args: AndroidBuildArgs) -> Result<BuildReport> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let project = Project::open(&project_dir)?;
    let config = project.config();
    let android_config = config.backends.android.clone().ok_or_else(|| {
        eyre!(
            "Android backend not configured for this project. Add it to Water.toml or recreate the project with the Android backend."
        )
    })?;

    let requested = if args.targets.is_empty() {
        None
    } else {
        Some(args.targets.clone())
    };

    let result = build_android_native_libraries(AndroidNativeBuildOptions {
        project_dir: project.root(),
        android_config: &android_config,
        crate_name: &config.package.name,
        release: args.release,
        requested_triples: requested,
        enable_sccache: !args.no_sccache,
        enable_mold: args.mold,
        hot_reload: false,
    })?;

    if !output::global_output_format().is_json() {
        ui::success(format!(
            "Android JNI libraries ready at {} ({})",
            result.jni_libs_dir.display(),
            result.targets.join(", ")
        ));
    }

    Ok(BuildReport {
        project_dir: project_dir.display().to_string(),
        platform: "android".to_string(),
        profile: result.profile,
        artifacts: vec![BuildArtifact {
            path: result.jni_libs_dir.display().to_string(),
            kind: Some("jniLibs".to_string()),
            targets: Some(result.targets),
        }],
    })
}

fn build_apple(args: AppleBuildArgs) -> Result<BuildReport> {
    ensure_macos_host("Rust static library build")?;

    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let project = Project::open(&project_dir)?;
    let config = project.config();
    if config.backends.swift.is_none() {
        bail!(
            "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the Apple backend."
        );
    }

    let result = build_apple_static_library(AppleRustBuildOptions {
        project_dir: project.root(),
        crate_name: &config.package.name,
        release: args.release,
        platform_name: args.platform_name.clone(),
        arch: args.arch.clone(),
        output_dir: args.output_dir.clone(),
        enable_sccache: !args.no_sccache,
        enable_mold: args.mold,
    })?;

    if !output::global_output_format().is_json() {
        ui::success(format!(
            "Apple static library ready at {} ({})",
            result.output_library.display(),
            result.target
        ));
    }

    Ok(BuildReport {
        project_dir: project_dir.display().to_string(),
        platform: "apple".to_string(),
        profile: result.profile,
        artifacts: vec![BuildArtifact {
            path: result.output_library.display().to_string(),
            kind: Some("staticlib".to_string()),
            targets: Some(vec![result.target]),
        }],
    })
}
