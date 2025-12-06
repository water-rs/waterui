//! Playground runtime support.
//!
//! This module handles the creation and management of temporary platform backends
//! for playground projects. Playground projects don't have permanent backend directories;
//! instead, backends are created on-demand in a cache directory.
//!
//! ## Key differences from regular app projects:
//!
//! - No `[backends.*]` sections in Water.toml - backends are auto-generated
//! - Backend projects are created in `.water/playground/` cache directory
//! - Users cannot customize backend settings (scheme, `local_path`, etc.)
//! - `waterui_path` and `dev_dependencies` still work for local `WaterUI` development
//!
//! ## Version tracking
//!
//! Each platform backend stores a `.cli-version` file containing the CLI version
//! that generated it. When the CLI version changes, the backend is regenerated
//! (preserving build caches like Gradle's `build/` or Xcode's `DerivedData`).

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};
use heck::ToUpperCamelCase;
use tracing::info;

use super::create::{
    self, BackendChoice, SwiftDependency, resolve_dependencies_with_path, validate_waterui_path,
};
use crate::util;
use waterui_cli::permission::{Permission, ResolvedPermission};
use waterui_cli::project::{Android, Config, Swift, Web, playground_cache_dir};

/// Filename for storing the CLI version that generated a playground backend.
const CLI_VERSION_FILE: &str = ".cli-version";

/// Ensure platform backend exists for a playground project.
///
/// The backend is regenerated when:
/// - It doesn't exist yet
/// - The CLI version has changed (ensures users get latest templates/fixes)
/// - Dev mode is enabled (`waterui_path` is set) - always regenerate for local development
///
/// Build caches (Gradle's `build/`, Xcode's `DerivedData`) are preserved during regeneration.
///
/// Returns the modified config with the backend configuration pointing to the cache.
pub fn ensure_playground_backend(
    project_dir: &Path,
    config: &Config,
    crate_name: &str,
    backend: BackendChoice,
) -> Result<Config> {
    let cache_dir = playground_cache_dir(project_dir);
    util::ensure_directory(&cache_dir)?;

    let mut config = config.clone();

    // For playground, dev mode is determined by waterui_path presence
    let use_dev = config.waterui_path.is_some();

    // Resolve dependencies based on waterui_path
    let validated_waterui_path = if let Some(ref path_str) = config.waterui_path {
        Some(validate_waterui_path(&PathBuf::from(path_str))?)
    } else {
        None
    };
    let deps = resolve_dependencies_with_path(validated_waterui_path.as_ref())?;

    let display_name = &config.package.name;
    let app_name = {
        let generated = display_name.to_upper_camel_case();
        if generated.is_empty() {
            "PlaygroundApp".to_string()
        } else {
            generated
        }
    };

    // Resolve permissions from config
    let resolved_permissions = resolve_permissions(&config);

    match backend {
        BackendChoice::Web => {
            let web_dir = cache_dir.join("web");
            if should_regenerate_backend(&web_dir, use_dev) {
                regenerate_backend_dir(&web_dir)?;
                util::ensure_directory(&web_dir)?;
                create::web::create_web_assets(&cache_dir, display_name)?;
                write_cli_version(&web_dir)?;
            }
            config.backends.web = Some(Web {
                project_path: web_dir.display().to_string(),
                version: None,
                dev: use_dev,
            });
        }
        BackendChoice::Android => {
            let android_dir = cache_dir.join("android");
            if should_regenerate_backend(&android_dir, use_dev) {
                regenerate_backend_dir(&android_dir)?;
                create::android::create_android_project_with_permissions(
                    &cache_dir,
                    &app_name,
                    crate_name,
                    &config.package.bundle_identifier,
                    use_dev,
                    deps.local_waterui_path.as_ref(),
                    &resolved_permissions,
                )?;
                write_cli_version(&android_dir)?;
            }
            config.backends.android = Some(Android {
                project_path: android_dir.display().to_string(),
                version: None,
                dev: use_dev,
            });
        }
        BackendChoice::Apple => {
            let apple_dir = cache_dir.join("apple");
            if should_regenerate_backend(&apple_dir, use_dev) {
                regenerate_backend_dir(&apple_dir)?;
                util::ensure_directory(&apple_dir)?;
                create::swift::create_xcode_project_with_permissions(
                    &cache_dir,
                    &app_name,
                    display_name,
                    crate_name,
                    &config.package.bundle_identifier,
                    &deps.swift,
                    &resolved_permissions,
                )?;
                write_cli_version(&apple_dir)?;
            }
            let (version, branch, revision, local_path) = match &deps.swift {
                SwiftDependency::Git {
                    version,
                    branch,
                    revision,
                } => (version.clone(), branch.clone(), revision.clone(), None),
                SwiftDependency::Local { path } => {
                    (None, None, None, Some(path.display().to_string()))
                }
            };
            config.backends.swift = Some(Swift {
                project_path: apple_dir.display().to_string(),
                scheme: crate_name.to_string(),
                project_file: Some(format!("{app_name}.xcodeproj")),
                version,
                branch,
                revision,
                local_path,
                dev: use_dev,
            });
        }
    }

    Ok(config)
}

/// Check if a backend directory needs to be regenerated.
///
/// Returns true if:
/// - The directory doesn't exist
/// - Dev mode is enabled (always regenerate for local development)
/// - The CLI version has changed
fn should_regenerate_backend(backend_dir: &Path, dev_mode: bool) -> bool {
    if !backend_dir.exists() {
        return true;
    }

    // In dev mode, always regenerate to pick up local WaterUI changes
    if dev_mode {
        info!("Dev mode enabled, regenerating playground backend");
        return true;
    }

    // Check if CLI version matches
    let version_file = backend_dir.join(CLI_VERSION_FILE);
    if let Ok(stored_version) = std::fs::read_to_string(&version_file) {
        let current_version = waterui_cli::WATERUI_VERSION;
        if stored_version.trim() == current_version {
            false
        } else {
            info!(
                "CLI version changed ({} -> {}), regenerating playground backend",
                stored_version.trim(),
                current_version
            );
            true
        }
    } else {
        // No version file means old backend, regenerate
        info!("No CLI version found, regenerating playground backend");
        true
    }
}

/// Remove backend directory contents while preserving build caches.
fn regenerate_backend_dir(backend_dir: &Path) -> Result<()> {
    if !backend_dir.exists() {
        return Ok(());
    }

    // Directories to preserve (build caches)
    let preserve = ["build", ".gradle", "DerivedData", ".build"];

    for entry in std::fs::read_dir(backend_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if preserve.iter().any(|p| *p == name_str) {
            continue;
        }

        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        } else {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }

    Ok(())
}

/// Write the current CLI version to the backend directory.
fn write_cli_version(backend_dir: &Path) -> Result<()> {
    let version_file = backend_dir.join(CLI_VERSION_FILE);
    std::fs::write(&version_file, waterui_cli::WATERUI_VERSION)
        .with_context(|| format!("failed to write CLI version to {}", version_file.display()))?;
    Ok(())
}

/// Get available backends for a playground based on platform detection.
#[allow(dead_code)]
pub fn available_playground_backends() -> Vec<BackendChoice> {
    let mut backends = vec![BackendChoice::Web];

    #[cfg(target_os = "macos")]
    backends.push(BackendChoice::Apple);

    // Android is available on all platforms if Android SDK is configured
    backends.push(BackendChoice::Android);

    backends
}

/// Clean the playground cache directory.
#[allow(dead_code)]
pub fn clean_playground_cache(project_dir: &Path) -> Result<()> {
    let cache_dir = playground_cache_dir(project_dir);
    if cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir)
            .with_context(|| format!("failed to remove playground cache at {}", cache_dir.display()))?;
    }
    Ok(())
}

/// Resolve permissions from project config into platform-ready format.
fn resolve_permissions(config: &Config) -> Vec<ResolvedPermission> {
    config
        .permissions
        .all_enabled()
        .into_iter()
        .filter_map(|name| {
            let permission: Permission = name.parse().ok()?;
            let description = config.permissions.get_description(&name);
            Some(ResolvedPermission {
                permission,
                description,
            })
        })
        .collect()
}
