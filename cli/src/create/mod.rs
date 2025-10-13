use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, Result, bail};
use clap::{Args, ValueEnum};
use crates_index::Index;
use dialoguer::{Confirm, Input, MultiSelect, Select, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use semver::Version;
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

pub(crate) const WATERUI_GIT_URL: &str = "https://github.com/water-rs/waterui.git";
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
        info!("Xcode scheme: {}", app_name);
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
                    scheme: app_name.clone(),
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

#[derive(Clone, Debug)]
struct AppleIdentity {
    description: String,
    team_id: String,
}

pub struct ProjectDependencies {
    rust_toml: String,
    swift: SwiftDependency,
}

pub enum SwiftDependency {
    Remote { requirement: String },
    Dev,
}

fn fetch_team_ids() -> Result<Vec<AppleIdentity>> {
    if cfg!(not(target_os = "macos")) {
        return Ok(Vec::new());
    }

    let output = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output();

    let output = match output {
        Ok(output) => output,
        Err(_) => return Ok(Vec::new()),
    };

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut identities = Vec::new();

    for line in stdout.lines() {
        if let Some(identity) = parse_identity_line(line) {
            let duplicate = identities
                .iter()
                .any(|existing: &AppleIdentity| existing.team_id == identity.team_id);
            if !duplicate {
                identities.push(identity);
            }
        }
    }

    Ok(identities)
}

fn parse_identity_line(line: &str) -> Option<AppleIdentity> {
    let trimmed = line.trim();
    let start = trimmed.find('"')?;
    let end = trimmed.rfind('"')?;
    if end <= start {
        return None;
    }

    let label = &trimmed[start + 1..end];
    let open = label.rfind('(')?;
    let close = label.rfind(')')?;
    if close <= open {
        return None;
    }

    let team_id = label[open + 1..close].trim().to_string();
    if team_id.is_empty() {
        return None;
    }

    Some(AppleIdentity {
        description: label.trim().to_string(),
        team_id,
    })
}

fn resolve_dependencies(dev: bool) -> Result<ProjectDependencies> {
    if dev {
        let rust_toml = r#"waterui = { git = "https://github.com/water-rs/waterui" }
waterui-ffi = { git = "https://github.com/water-rs/waterui" }"#
            .to_string();
        return Ok(ProjectDependencies {
            rust_toml,
            swift: SwiftDependency::Dev,
        });
    }

    let mut index = Index::new_cargo_default().map_err(|err| human_dependency_error(err, None))?;
    index
        .update()
        .map_err(|err| human_dependency_error(err, Some("updating the crates.io index")))?;

    let waterui_version = latest_crates_io_version(&index, "waterui")
        .ok_or_else(|| missing_crate_error("waterui"))?;
    let waterui_ffi_version = latest_crates_io_version(&index, "waterui-ffi")
        .ok_or_else(|| missing_crate_error("waterui-ffi"))?;

    let rust_toml = format!(
        r#"waterui = "{}"
waterui-ffi = "{}""#,
        waterui_version, waterui_ffi_version
    );

    let swift_version = latest_swift_backend_version().map_err(|err| {
        human_dependency_error(
            err,
            Some("discovering Swift backend releases (tags prefixed with swift-backend-v*)"),
        )
    })?;

    Ok(ProjectDependencies {
        rust_toml,
        swift: SwiftDependency::Remote {
            requirement: format!(
                "\t\t\tkind = exactVersion;\n\t\t\tversion = \"{}\";",
                swift_version
            ),
        },
    })
}

fn latest_crates_io_version(index: &Index, name: &str) -> Option<Version> {
    let krate = index.crate_(name)?;
    krate
        .versions()
        .iter()
        .filter(|v| !v.is_yanked())
        .filter_map(|v| Version::parse(v.version()).ok())
        .max()
}

fn latest_swift_backend_version() -> Result<String> {
    let pattern = format!("refs/tags/{}*", SWIFT_TAG_PREFIX);

    let output = Command::new("git")
        .args(["ls-remote", "--tags", WATERUI_GIT_URL, pattern.as_str()])
        .output()
        .context("Failed to query Swift backend tags with git")?;

    if !output.status.success() {
        bail!("`git ls-remote` returned a non-zero status");
    }

    let mut best: Option<(Version, String)> = None;

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split_ascii_whitespace();
        let _hash = parts.next();
        let reference = match parts.next() {
            Some(r) => r,
            None => continue,
        };
        if reference.ends_with("^{}") {
            continue;
        }
        if let Some(tag) = reference.strip_prefix("refs/tags/") {
            if let Some(version_str) = tag.strip_prefix(SWIFT_TAG_PREFIX) {
                if let Ok(version) = Version::parse(version_str) {
                    if best.as_ref().is_none_or(|(best_v, _)| &version > best_v) {
                        best = Some((version, version_str.to_string()));
                    }
                }
            }
        }
    }

    best.map(|(_, version)| version)
        .ok_or_else(|| anyhow::anyhow!("No Swift backend release tags found"))
}

fn human_dependency_error<E: std::fmt::Display>(err: E, action: Option<&str>) -> anyhow::Error {
    let err_str = err.to_string();
    let mut message = String::new();
    if let Some(action) = action {
        message.push_str(&format!("Problem while {action}:\n"));
    }
    message.push_str(&err_str);

    if err_str.contains("refs/heads/master") {
        message.push_str(
            "\n\nHint: Your local crates.io index still points to `master`. Remove \
~/.cargo/registry/index/github.com-1ecc6299db9ec823 or update HEAD to `main`, then retry.",
        );
    }

    message.push_str(
        "\n\nHint: If you prefer to use the latest development version of WaterUI, rerun with `--dev`.",
    );

    anyhow::anyhow!(message)
}

fn missing_crate_error(name: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "No crates.io release of `{name}` found.\n\nHint: If you prefer to use the latest development version of WaterUI, rerun with `--dev`."
    )
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
