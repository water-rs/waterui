use std::path::PathBuf;

use clap::Args;
use color_eyre::eyre::{Result, bail};
use heck::ToUpperCamelCase;
use tracing::info;

use super::create::{self, BackendChoice, resolve_dependencies};
use serde::Serialize;
use waterui_cli::project::{Android, Config, Swift, Web};

#[derive(Args, Debug, Clone)]
pub struct AddBackendArgs {
    /// Backend to add to the project
    pub backend: BackendChoice,

    /// Project directory (defaults to current working directory)
    pub project: Option<PathBuf>,

    /// Use the development version of `WaterUI` from GitHub
    pub dev: bool,
}

/// Add an additional backend implementation to an existing `WaterUI` project.
///
/// # Errors
/// Returns an error if the project configuration cannot be read or written, or if template
/// generation fails.
///
/// # Panics
/// Panics when the current working directory cannot be determined, which indicates an
/// unexpected environment issue.
#[allow(clippy::needless_pass_by_value)]
pub fn run(args: AddBackendArgs) -> Result<AddBackendReport> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let mut config = Config::load(&project_dir)?;

    let deps = resolve_dependencies(args.dev)?;

    match args.backend {
        BackendChoice::Web => {
            if config.backends.web.is_some() {
                bail!("Web backend already exists in this project.");
            }
            info!("Adding Web backend...");
            create::web::create_web_assets(&project_dir, &config.package.display_name)?;
            config.backends.web = Some(Web {
                project_path: "web".to_string(),
            });
            if !config.hot_reload.watch.iter().any(|path| path == "web") {
                config.hot_reload.watch.push("web".to_string());
            }
            info!("Web backend added successfully.");
        }
        BackendChoice::Android => {
            if config.backends.android.is_some() {
                bail!("Android backend already exists in this project.");
            }
            info!("Adding Android backend...");
            let app_name = {
                let generated = config.package.display_name.to_upper_camel_case();
                if generated.is_empty() {
                    "WaterUIApp".to_string()
                } else {
                    generated
                }
            };
            create::android::create_android_project(
                &project_dir,
                &app_name,
                &config.package.name,
                &config.package.bundle_identifier,
                args.dev,
            )?;
            config.backends.android = Some(Android {
                project_path: "android".to_string(),
            });
            info!("Android backend added successfully.");
        }
        BackendChoice::Swiftui => {
            if config.backends.swift.is_some() {
                bail!("SwiftUI backend already exists in this project.");
            }
            info!("Adding SwiftUI backend...");
            let app_name = {
                let generated = config.package.display_name.to_upper_camel_case();
                if generated.is_empty() {
                    "WaterUIApp".to_string()
                } else {
                    generated
                }
            };
            create::swift::create_xcode_project(
                &project_dir,
                &app_name,
                &config.package.display_name,
                &config.package.name,
                &config.package.bundle_identifier,
                &deps.swift,
            )?;
            config.backends.swift = Some(Swift {
                project_path: "apple".to_string(),
                scheme: config.package.name.clone(),
                project_file: Some(format!("{app_name}.xcodeproj")),
            });
            info!("SwiftUI backend added successfully.");
        }
    }

    config.save(&project_dir)?;
    info!("Updated Water.toml.");

    let report = AddBackendReport {
        project_dir: project_dir.display().to_string(),
        backend: args.backend.label().to_string(),
        using_dev_dependencies: args.dev,
        config_path: Config::path(&project_dir).display().to_string(),
    };

    Ok(report)
}

#[derive(Debug, Serialize)]
pub struct AddBackendReport {
    pub project_dir: String,
    pub backend: String,
    pub using_dev_dependencies: bool,
    pub config_path: String,
}
