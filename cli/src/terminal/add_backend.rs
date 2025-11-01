use std::path::PathBuf;

use clap::Args;
use color_eyre::eyre::{Result, bail};
use tracing::info;

use crate::{
    config::{Android, Config, Swift, Web},
    create::{self, BackendChoice, resolve_dependencies},
    output, util,
};
use serde::Serialize;

#[derive(Args, Debug)]
pub struct AddBackendArgs {
    /// Backend to add to the project
    #[arg(value_enum)]
    pub backend: BackendChoice,

    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Use the development version of WaterUI from GitHub
    #[arg(long)]
    pub dev: bool,
}

pub fn run(args: AddBackendArgs) -> Result<()> {
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
            let app_name = util::pascal_case(&config.package.display_name);
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
            let app_name = util::pascal_case(&config.package.display_name);
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
                project_file: Some(format!("{}.xcodeproj", app_name)),
            });
            info!("SwiftUI backend added successfully.");
        }
    }

    config.save(&project_dir)?;
    info!("Updated Water.toml.");

    if output::global_output_format().is_json() {
        let report = AddBackendReport {
            project_dir: project_dir.display().to_string(),
            backend: args.backend.label().to_string(),
            using_dev_dependencies: args.dev,
            config_path: Config::path(&project_dir).display().to_string(),
        };
        output::emit_json(&report)?;
    }

    Ok(())
}

#[derive(Serialize)]
struct AddBackendReport {
    project_dir: String,
    backend: String,
    using_dev_dependencies: bool,
    config_path: String,
}
