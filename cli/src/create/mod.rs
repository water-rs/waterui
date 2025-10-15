use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Result, bail};
use dialoguer::{Confirm, Input, MultiSelect, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{info, warn};

use crate::{
    config::{Android, Config, Package, Swift, Web},
    util,
};

pub mod android;
pub mod rust;
pub mod swift;
pub mod template;
pub mod web;

pub(crate) const SWIFT_BACKEND_GIT_URL: &str = "https://github.com/water-rs/swift-backend.git";
const SWIFT_TAG_PREFIX: &str = "swift-backend-v";

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

    /// Use the development version of WaterUI from GitHub
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
    fn label(&self) -> &'static str {
        match self {
            BackendChoice::Web => "Web",
            BackendChoice::Swiftui => "SwiftUI",
            BackendChoice::Android => "Android",
        }
    }
}

pub fn run(args: CreateArgs) -> Result<()> {
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

    let crate_name = util::kebab_case(&display_name);
    let app_name = util::pascal_case(&display_name);

    let default_bundle_identifier = format!("com.waterui.{}", crate_name);
    let bundle_identifier = if let Some(id) = args.bundle_identifier {
        id
    } else if args.yes {
        default_bundle_identifier.clone()
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
            .map(BackendChoice::label)
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
            return Ok(());
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
        .map(BackendChoice::label)
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
            return Ok(());
        }
    }

    // Create progress indicator
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message("Creating project structure...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    prepare_directory(&project_dir)?;

    spinner.set_message("Generating Rust sources...");
    rust::create_rust_sources(&project_dir, &crate_name, &author, &display_name, &deps)?;

    let mut config = Config::new(Package {
        name: crate_name.clone(),
        display_name: display_name.clone(),
        bundle_identifier: bundle_identifier.clone(),
    });

    let mut web_enabled = false;
    let total_backends = selected_backends.len();
    for (idx, backend) in selected_backends.iter().enumerate() {
        let progress = format!("[{}/{}]", idx + 1, total_backends);
        match backend {
            BackendChoice::Web => {
                spinner.set_message(format!("{} Creating Web backend...", progress));
                web::create_web_assets(&project_dir, &display_name)?;
                spinner.set_message(format!("{} Web backend created ✓", progress));
                config.backends.web = Some(Web {
                    project_path: "web".to_string(),
                });
                web_enabled = true;
            }
            BackendChoice::Android => {
                spinner.set_message(format!("{} Creating Android backend...", progress));
                android::create_android_project(
                    &project_dir,
                    &app_name,
                    &crate_name,
                    &bundle_identifier,
                )?;
                spinner.set_message(format!("{} Android backend created ✓", progress));
                config.backends.android = Some(Android {
                    project_path: "android".to_string(),
                });
            }
            BackendChoice::Swiftui => {
                spinner.set_message(format!("{} Creating SwiftUI backend...", progress));
                swift::create_xcode_project(
                    &project_dir,
                    &app_name,
                    &display_name,
                    &crate_name,
                    &bundle_identifier,
                    &deps.swift,
                )?;
                spinner.set_message(format!("{} SwiftUI backend created ✓", progress));
                config.backends.swift = Some(Swift {
                    project_path: "apple".to_string(),
                    scheme: crate_name.clone(),
                    project_file: Some(format!("{}.xcodeproj", app_name)),
                });
            }
        }
    }

    if web_enabled && !config.hot_reload.watch.iter().any(|path| path == "web") {
        config.hot_reload.watch.push("web".to_string());
    }

    spinner.set_message("Saving configuration...");
    config.save(&project_dir)?;

    spinner.finish_with_message("Project created successfully!");
    spinner.finish_and_clear();
    info!("✅ Project created");
    let current_dir = std::env::current_dir()?;
    let display_path = project_dir
        .strip_prefix(current_dir)
        .unwrap_or(&project_dir);
    info!("Next steps:\n  cd {}\n  water run", display_path.display());

    // if which::which("git").is_ok() {
    //     std::process::Command::new("git")
    //         .arg("init")
    //         .current_dir(&project_dir)
    //         .output()?;
    //     info!("✅ Git repository initialized");
    // }

    Ok(())
}

pub struct ProjectDependencies {
    rust_toml: String,
    swift: SwiftDependency,
}

pub enum SwiftDependency {
    Git { version: Option<String> },
}

#[allow(clippy::const_is_empty)]
fn resolve_dependencies(dev: bool) -> Result<ProjectDependencies> {
    if dev {
        let rust_toml = r#"waterui = { git = "https://github.com/water-rs/waterui" }
waterui-ffi = { git = "https://github.com/water-rs/waterui" }"#
            .to_string();
        return Ok(ProjectDependencies {
            rust_toml,
            swift: SwiftDependency::Git { version: None },
        });
    }

    let waterui_version = crate::WATERUI_VERSION;
    if waterui_version.is_empty() {
        bail!("WATERUI_VERSION is not set. This should be set at build time.");
    }

    let rust_toml = format!(
        r#"waterui = "{}"
waterui-ffi = "{}""#,
        waterui_version, waterui_version
    );

    let swift_backend_version = crate::WATERUI_SWIFT_BACKEND_VERSION;
    let swift_version = if swift_backend_version.is_empty() {
        warn!(
            "WATERUI_SWIFT_BACKEND_VERSION is not set. This can happen if no tags are found. Defaulting to main branch for Swift backend."
        );
        None
    } else {
        Some(swift_backend_version.to_string())
    };

    Ok(ProjectDependencies {
        rust_toml,
        swift: SwiftDependency::Git {
            version: swift_version,
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
