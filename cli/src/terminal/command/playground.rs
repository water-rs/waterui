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
//! - Users cannot customize backend settings (scheme, local_path, etc.)
//! - `waterui_path` and `dev_dependencies` still work for local WaterUI development

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};
use heck::ToUpperCamelCase;

use super::create::{
    self, BackendChoice, SwiftDependency, resolve_dependencies_with_path, validate_waterui_path,
};
use crate::util;
use waterui_cli::permission::{Permission, ResolvedPermission};
use waterui_cli::project::{Android, Config, Swift, Web, playground_cache_dir};

/// Ensure platform backend exists for a playground project.
/// Creates the backend in the playground cache directory if it doesn't exist.
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
            if !web_dir.exists() {
                util::ensure_directory(&web_dir)?;
                create::web::create_web_assets(&cache_dir, display_name)?;
            }
            config.backends.web = Some(Web {
                project_path: cache_dir.join("web").display().to_string(),
                version: None,
                dev: use_dev,
            });
        }
        BackendChoice::Android => {
            let android_dir = cache_dir.join("android");
            if !android_dir.exists() {
                create::android::create_android_project_with_permissions(
                    &cache_dir,
                    &app_name,
                    crate_name,
                    &config.package.bundle_identifier,
                    use_dev,
                    deps.local_waterui_path.as_ref(),
                    &resolved_permissions,
                )?;
            }
            config.backends.android = Some(Android {
                project_path: cache_dir.join("android").display().to_string(),
                version: None,
                dev: use_dev,
            });
        }
        BackendChoice::Apple => {
            let apple_dir = cache_dir.join("apple");
            if !apple_dir.exists() {
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
                project_path: cache_dir.join("apple").display().to_string(),
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
