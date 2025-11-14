use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Result, bail};
use dialoguer::{Confirm, Input, MultiSelect, theme::ColorfulTheme};
use heck::{ToKebabCase, ToUpperCamelCase};
use tracing::{info, warn};

use crate::util;
use serde::Serialize;
use waterui_cli::{
    WATERUI_SWIFT_BACKEND_VERSION, WATERUI_VERSION, output,
    project::{Android, Config, Package, Swift, Web},
};

pub mod android;
pub mod rust;
pub mod swift;
pub mod template;
pub mod web;

const SWIFT_BACKEND_GIT_URL: &str = "https://github.com/water-rs/apple-backend.git";
#[allow(dead_code)]
const SWIFT_TAG_PREFIX: &str = "apple-backend-v";

#[derive(Args, Debug, Default)]
pub struct CreateArgs {
    /// Application display name
    #[arg(long)]
    pub name: Option<String>,

    /// Directory to create the project in
    #[arg(long)]
    pub directory: Option<PathBuf>,

    /// Bundle identifier used for Apple platforms
    #[arg(long)]
    pub bundle_identifier: Option<String>,

    /// Apple Development Team ID
    #[arg(long)]
    pub team_id: Option<String>,

    /// Use the development version of `WaterUI` from GitHub
    #[arg(long)]
    pub dev: bool,

    /// Accept defaults without confirmation
    #[arg(short, long)]
    pub yes: bool,

    /// Backends to include (android, web, swiftui). Can be provided multiple times or as a comma-separated list.
    #[arg(long = "backend", value_enum, value_delimiter = ',', num_args = 1..)]
    pub backends: Vec<BackendChoice>,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum BackendChoice {
    #[clap(name = "web")]
    Web,
    #[clap(name = "swiftui")]
    Swiftui,
    #[clap(name = "android")]
    Android,
}

impl BackendChoice {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Web => "Web",
            Self::Swiftui => "SwiftUI",
            Self::Android => "Android",
        }
    }
}

/// Interactive entry point for `water create`.
///
/// # Errors
/// Returns an error if user input cannot be read, dependencies cannot be resolved, or
/// template files fail to write.
///
/// # Panics
/// Panics if required embedded templates are missing; this indicates a build-time bug.
#[allow(clippy::too_many_lines)]
pub fn run(args: CreateArgs) -> Result<CreateReport> {
    let is_json = output::global_output_format().is_json();
    if is_json && !args.yes {
        bail!(
            "JSON output requires --yes to avoid interactive prompts. Re-run with --yes or provide --backend, --name, and related flags."
        );
    }

    let theme = ColorfulTheme::default();

    let deps = resolve_dependencies(args.dev)?;

    let display_name = if let Some(name) = args.name.clone() {
        name
    } else if args.yes {
        "Water Demo".to_string()
    } else {
        Input::with_theme(&theme)
            .with_prompt("Application name")
            .default("Water Demo".to_string())
            .interact_text()?
    };

    let default_author = std::process::Command::new("git")
        .arg("config")
        .arg("user.name")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let author = if args.yes {
        default_author
    } else {
        Input::with_theme(&theme)
            .with_prompt("Author")
            .default(default_author)
            .interact_text()?
    };

    let crate_name = {
        let generated = display_name.to_kebab_case();
        if generated.is_empty() {
            "waterui-app".to_string()
        } else {
            generated
        }
    };
    let app_name = {
        let generated = display_name.to_upper_camel_case();
        if generated.is_empty() {
            "WaterUIApp".to_string()
        } else {
            generated
        }
    };

    let default_bundle_identifier = format!("com.waterui.{crate_name}");
    let bundle_identifier = if let Some(id) = args.bundle_identifier {
        id
    } else if args.yes {
        default_bundle_identifier
    } else {
        Input::with_theme(&theme)
            .with_prompt("Bundle identifier")
            .default(default_bundle_identifier)
            .interact_text()?
    };

    let project_dir = if let Some(dir) = args.directory {
        dir
    } else {
        let default = std::env::current_dir()?.join(&crate_name);
        if args.yes {
            default
        } else {
            Input::with_theme(&theme)
                .with_prompt("Project directory")
                .default(default.display().to_string())
                .interact_text()
                .map(PathBuf::from)?
        }
    };

    let selected_backends: Vec<BackendChoice> = if args.backends.is_empty() {
        let available_backends = [
            BackendChoice::Web,
            BackendChoice::Swiftui,
            BackendChoice::Android,
        ];
        let defaults = vec![true; available_backends.len()];
        let labels: Vec<String> = available_backends
            .iter()
            .map(|choice| choice.label())
            .map(str::to_string)
            .collect();
        let selected_indices = if args.yes {
            (0..available_backends.len()).collect()
        } else {
            MultiSelect::with_theme(&theme)
                .with_prompt("Choose project backends (space to select, enter to confirm)")
                .items(&labels)
                .defaults(&defaults)
                .interact()?
        };

        if selected_indices.is_empty() {
            warn!("No backends selected, aborting.");
            return Ok(build_report(
                CreateStatus::Cancelled,
                &project_dir,
                &crate_name,
                &display_name,
                &bundle_identifier,
                &[],
                args.dev,
            ));
        }

        selected_indices
            .iter()
            .map(|&index| available_backends[index])
            .collect()
    } else {
        args.backends.clone()
    };

    info!("Application: {}", display_name);
    info!("Author: {}", author);
    info!("Crate name: {}", crate_name);
    if selected_backends.contains(&BackendChoice::Swiftui) {
        info!("Xcode scheme: {}", crate_name);
    }
    info!("Bundle ID: {}", bundle_identifier);
    let backend_list = selected_backends
        .iter()
        .map(|choice| choice.label())
        .collect::<Vec<_>>()
        .join(", ");
    info!("Backends: {}", backend_list);
    info!("Location: {}", project_dir.display());

    if !args.yes {
        let proceed = Confirm::with_theme(&theme)
            .with_prompt("Create project with these settings?")
            .default(true)
            .interact()?;
        if !proceed {
            warn!("Cancelled");
            return Ok(build_report(
                CreateStatus::Cancelled,
                &project_dir,
                &crate_name,
                &display_name,
                &bundle_identifier,
                &selected_backends,
                args.dev,
            ));
        }
    }

    prepare_directory(&project_dir)?;

    rust::create_rust_sources(&project_dir, &crate_name, &author, &display_name, &deps)?;

    let mut config = Config::new(Package {
        name: crate_name.clone(),
        display_name: display_name.clone(),
        bundle_identifier: bundle_identifier.clone(),
        author,
    });

    let mut web_enabled = false;
    for backend in &selected_backends {
        match backend {
            BackendChoice::Web => {
                web::create_web_assets(&project_dir, &display_name)?;
                config.backends.web = Some(Web {
                    project_path: "web".to_string(),
                });
                web_enabled = true;
            }
            BackendChoice::Android => {
                android::create_android_project(
                    &project_dir,
                    &app_name,
                    &crate_name,
                    &bundle_identifier,
                    args.dev,
                )?;
                config.backends.android = Some(Android {
                    project_path: "android".to_string(),
                });
            }
            BackendChoice::Swiftui => {
                swift::create_xcode_project(
                    &project_dir,
                    &app_name,
                    &display_name,
                    &crate_name,
                    &bundle_identifier,
                    &deps.swift,
                )?;
                config.backends.swift = Some(Swift {
                    project_path: "apple".to_string(),
                    scheme: crate_name.clone(),
                    project_file: Some(format!("{app_name}.xcodeproj")),
                });
            }
        }
    }

    if web_enabled && !config.hot_reload.watch.iter().any(|path| path == "web") {
        config.hot_reload.watch.push("web".to_string());
    }

    config.save(&project_dir)?;
    info!("✅ Project created");
    if !is_json {
        let current_dir = std::env::current_dir()?;
        let display_path = project_dir
            .strip_prefix(current_dir)
            .unwrap_or(&project_dir);
        info!("Next steps:\n  cd {}\n  water run", display_path.display());
    }

    let report = build_report(
        CreateStatus::Created,
        &project_dir,
        &crate_name,
        &display_name,
        &bundle_identifier,
        &selected_backends,
        args.dev,
    );

    // if which::which("git").is_ok() {
    //     std::process::Command::new("git")
    //         .arg("init")
    //         .current_dir(&project_dir)
    //         .output()?;
    //     info!("✅ Git repository initialized");
    // }

    Ok(report)
}

#[derive(Debug, Serialize)]
pub struct CreateReport {
    pub status: CreateStatus,
    pub project_dir: String,
    pub crate_name: String,
    pub display_name: String,
    pub bundle_identifier: String,
    pub backends: Vec<String>,
    pub using_dev_dependencies: bool,
    pub config_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateStatus {
    Created,
    Cancelled,
}

fn build_report(
    status: CreateStatus,
    project_dir: &Path,
    crate_name: &str,
    display_name: &str,
    bundle_identifier: &str,
    backends: &[BackendChoice],
    using_dev_dependencies: bool,
) -> CreateReport {
    CreateReport {
        status,
        project_dir: project_dir.display().to_string(),
        crate_name: crate_name.to_string(),
        display_name: display_name.to_string(),
        bundle_identifier: bundle_identifier.to_string(),
        backends: backends
            .iter()
            .map(|backend| backend.label().to_string())
            .collect(),
        using_dev_dependencies,
        config_path: Config::path(project_dir).display().to_string(),
    }
}

#[derive(Debug)]
pub struct ProjectDependencies {
    rust_toml: String,
    pub swift: SwiftDependency,
}

#[derive(Debug)]
pub enum SwiftDependency {
    Git {
        version: Option<String>,
        branch: Option<String>,
    },
}

#[allow(clippy::const_is_empty)]
/// Resolve the template dependencies used when rendering new projects.
///
/// # Errors
/// Returns an error if the crates index cannot be queried.
pub fn resolve_dependencies(dev: bool) -> Result<ProjectDependencies> {
    if dev {
        let rust_toml =
            r#"waterui = { git = "https://github.com/water-rs/waterui", branch = "dev" }
waterui-ffi = { git = "https://github.com/water-rs/waterui", branch = "dev" }"#
                .to_string();
        return Ok(ProjectDependencies {
            rust_toml,
            swift: SwiftDependency::Git {
                version: None,
                branch: Some("dev".to_string()),
            },
        });
    }

    let waterui_version = WATERUI_VERSION;
    if waterui_version.is_empty() {
        bail!("WATERUI_VERSION is not set. This should be set at build time.");
    }

    let rust_toml = format!(
        r#"waterui = "{waterui_version}"
waterui-ffi = "{waterui_version}""#
    );

    let swift_backend_version = WATERUI_SWIFT_BACKEND_VERSION;
    if swift_backend_version.is_empty() {
        bail!("WATERUI_SWIFT_BACKEND_VERSION is not set. This should be set at build time.");
    }

    Ok(ProjectDependencies {
        rust_toml,
        swift: SwiftDependency::Git {
            version: Some(swift_backend_version.to_string()),
            branch: None,
        },
    })
}

fn prepare_directory(project_dir: &Path) -> Result<()> {
    if project_dir.exists() {
        if project_dir.is_file() {
            bail!("{} already exists and is a file", project_dir.display());
        }
        if project_dir.read_dir()?.next().is_some() {
            bail!("{} already exists and is not empty", project_dir.display());
        }
    }

    util::ensure_directory(project_dir)?;
    util::ensure_directory(&project_dir.join("src"))?;
    util::ensure_directory(&project_dir.join("apple"))?;
    Ok(())
}
