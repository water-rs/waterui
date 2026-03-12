//! Hydrolysis platform build and package utilities.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, bail};
use smol::fs;
use tracing::info;

use crate::{
    assets,
    build::BuildOptions,
    device::Artifact,
    hydrolysis::backend::HydrolysisBackend,
    macos_bundle::package_binary_as_app,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    toolchain::{ToolchainError, windows_arm64_llvm::WindowsArm64LlvmToolchain},
    utils::{command, run_command_os},
};

#[cfg(target_os = "macos")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform macos --backend hydrolysis";
#[cfg(target_os = "linux")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform linux --backend hydrolysis";
#[cfg(target_os = "windows")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform windows --backend hydrolysis";
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const HYDROLYSIS_INIT_HINT: &str = "initialize hydrolysis backend on macOS, Linux, or Windows";

/// Build hydrolysis binary for the host platform.
pub async fn build_hydrolysis(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<PathBuf> {
    if !is_hydrolysis_platform(platform) {
        bail!("Hydrolysis backend is only supported on macOS, Linux, and Windows");
    }

    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.backend_target_dir("hydrolysis");

    if !cargo_toml.exists() {
        bail!(
            "Hydrolysis backend not found at {}. Run `{HYDROLYSIS_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    let profile = if options.is_release() {
        "release"
    } else {
        "debug"
    };

    let mut cargo = smol::process::Command::new("cargo");
    let cargo = command(&mut cargo);
    cargo.arg("build").arg("--manifest-path").arg(&cargo_toml);
    cargo.arg("--target-dir").arg(&backend_target_dir);
    let llvm_envs = WindowsArm64LlvmToolchain
        .cargo_envs()
        .await
        .map_err(|error| match error {
            ToolchainError::Fixable(_) => eyre::eyre!(
                "Windows ARM64 LLVM toolchain is missing. Run `water doctor --fix` to install it automatically."
            ),
            ToolchainError::Unfixable(unfixable) => {
                eyre::eyre!("Windows ARM64 LLVM toolchain check failed: {unfixable}")
            }
        })?;
    for (key, value) in llvm_envs {
        cargo.env(key, value);
    }
    if options.is_release() {
        cargo.arg("--release");
    }

    let output = cargo.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let details = if !stderr.is_empty() {
            stderr.to_string()
        } else {
            stdout.to_string()
        };
        bail!(
            "Failed to build hydrolysis backend with cargo (status {}):\n{}",
            output.status,
            details
        );
    }

    Ok(backend_target_dir.join(profile))
}

/// Clean Cargo build artifacts for hydrolysis.
pub async fn clean_hydrolysis(project: &Project) -> eyre::Result<()> {
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.backend_target_dir("hydrolysis");

    if !cargo_toml.exists() {
        return Ok(());
    }

    let args: Vec<OsString> = vec![
        "clean".into(),
        "--manifest-path".into(),
        cargo_toml.as_os_str().to_owned(),
        "--target-dir".into(),
        backend_target_dir.as_os_str().to_owned(),
    ];
    run_command_os("cargo", args).await?;
    Ok(())
}

/// Package a hydrolysis app.
///
/// Linux/Windows return a binary artifact path.
/// macOS returns a `.app` bundle path.
pub async fn package_hydrolysis(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> eyre::Result<Artifact> {
    if !is_hydrolysis_platform(platform) {
        bail!("Hydrolysis backend is only supported on macOS, Linux, and Windows");
    }

    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let backend_path = project.backend_path::<HydrolysisBackend>();
    copy_assets_and_fonts(project, &backend_path).await?;

    let target_dir = project.backend_target_dir("hydrolysis").join(profile);
    let binary_name = project.hydrolysis_backend_crate_name();
    let binary_path = if cfg!(windows) {
        target_dir.join(format!("{binary_name}.exe"))
    } else {
        target_dir.join(&binary_name)
    };

    let final_binary_path = if binary_path.exists() {
        binary_path
    } else {
        let alt_binary_name = binary_name.replace('-', "_");
        let alt = if cfg!(windows) {
            target_dir.join(format!("{alt_binary_name}.exe"))
        } else {
            target_dir.join(alt_binary_name)
        };
        if alt.exists() {
            alt
        } else {
            bail!(
                "Built hydrolysis binary not found at {}. Did you run build first?",
                binary_path.display()
            );
        }
    };

    #[cfg(target_os = "macos")]
    {
        if platform == TargetPlatform::MacOS {
            let app_name = project
                .manifest()
                .package
                .name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ')
                .collect::<String>();
            let app_name = if app_name.is_empty() {
                "WaterUIHydrolysis".to_string()
            } else {
                app_name
            };
            let dist_dir = backend_path.join("dist");
            fs::create_dir_all(&dist_dir).await?;
            let app_path = package_binary_as_app(
                &final_binary_path,
                project.bundle_identifier(),
                &app_name,
                Some(&backend_path.join("resources")),
                &dist_dir,
            )
            .await?;
            return Ok(Artifact::new(project.bundle_identifier(), app_path));
        }
    }

    Ok(Artifact::new(
        project.bundle_identifier(),
        final_binary_path,
    ))
}

/// Check if a platform is supported by the hydrolysis backend.
pub const fn is_hydrolysis_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::Linux | TargetPlatform::MacOS | TargetPlatform::Windows
    )
}

async fn copy_assets_and_fonts(project: &Project, backend_path: &Path) -> eyre::Result<()> {
    let resources_dir = backend_path.join("resources");
    fs::create_dir_all(&resources_dir).await?;
    assets::stage_project_assets_for_gtk(project, &resources_dir).await?;

    let font_declarations = assets::scan_fonts(project).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(project)?);
    if !resolved_fonts.is_empty() {
        let fonts_dest = resources_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
        info!(
            "Copied {} fonts to hydrolysis resources",
            resolved_fonts.len()
        );
    }
    Ok(())
}
