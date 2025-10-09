use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;
use dialoguer::{Confirm, Input, MultiSelect, theme::ColorfulTheme};

use crate::{
    config::{Android, Config, Package, Swift},
    util,
};

pub mod android;
pub mod rust;
pub mod swift;
pub mod template;

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
}

pub fn run(args: CreateArgs) -> Result<()> {
    let theme = ColorfulTheme::default();

    let display_name = match args.name {
        Some(name) => name,
        None => Input::with_theme(&theme)
            .with_prompt("Application name")
            .default("Water Demo".to_string())
            .interact_text()?,
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

    let development_team = match args.team_id {
        Some(id) => id,
        None => {
            if args.yes {
                "".to_string()
            } else {
                Input::with_theme(&theme)
                    .with_prompt("Apple Development Team ID (optional, for automatic signing)")
                    .interact_text()?
            }
        }
    };

    let crate_name = util::kebab_case(&display_name);
    let app_name = util::pascal_case(&display_name);

    let bundle_identifier = match args.bundle_identifier {
        Some(id) => id,
        None => Input::with_theme(&theme)
            .with_prompt("Bundle identifier")
            .default(format!("com.waterui.{crate_name}"))
            .interact_text()?,
    };

    let project_dir = match args.directory {
        Some(dir) => dir,
        None => {
            let default = std::env::current_dir()?.join(&crate_name);
            Input::with_theme(&theme)
                .with_prompt("Project directory")
                .default(default.display().to_string())
                .interact_text()
                .map(PathBuf::from)?
        }
    };

    let backends = &["SwiftUI", "Android"];
    let defaults = vec![true; backends.len()];
    let selected_indices = if args.yes {
        (0..backends.len()).collect()
    } else {
        MultiSelect::with_theme(&theme)
            .with_prompt("Choose project backends (space to select, enter to confirm)")
            .items(backends)
            .defaults(&defaults)
            .interact()?
    };

    if selected_indices.is_empty() {
        util::warn("No backends selected, aborting.");
        return Ok(());
    }

    let selected_backends: Vec<&str> = selected_indices.iter().map(|&i| backends[i]).collect();

    util::info(format!("Application: {display_name}"));
    util::info(format!("Author: {author}"));
    util::info(format!("Crate name: {crate_name}"));
    if selected_backends.contains(&"SwiftUI") {
        util::info(format!("Xcode scheme: {app_name}"));
    }
    util::info(format!("Bundle ID: {bundle_identifier}"));
    util::info(format!("Backends: {}", selected_backends.join(", ")));
    util::info(format!("Location: {}", project_dir.display()));

    if !args.yes {
        let proceed = Confirm::with_theme(&theme)
            .with_prompt("Create project with these settings?")
            .default(true)
            .interact()?;
        if !proceed {
            util::warn("Cancelled");
            return Ok(());
        }
    }

    prepare_directory(&project_dir)?;
    rust::create_rust_sources(&project_dir, &crate_name, &display_name, &author, args.dev)?;

    let mut config = Config::new(Package {
        name: crate_name.clone(),
        display_name: display_name.clone(),
        bundle_identifier: bundle_identifier.clone(),
    });

    for backend in selected_backends {
        match backend {
            "Android" => {
                android::create_android_project(
                    &project_dir,
                    &app_name,
                    &crate_name,
                    &bundle_identifier,
                )?;
                config.backends.android = Some(Android {
                    project_path: "android".to_string(),
                });
            }
            "SwiftUI" => {
                swift::create_xcode_project(
                    &project_dir,
                    &app_name,
                    &crate_name,
                    &bundle_identifier,
                    &development_team,
                )?;
                config.backends.swift = Some(Swift {
                    project_path: "apple".to_string(),
                    scheme: app_name.clone(),
                });
            }
            _ => unreachable!(),
        }
    }

    config.save(&project_dir)?;

    util::info("✅ Project created");
    let current_dir = std::env::current_dir()?;
    let display_path = project_dir
        .strip_prefix(current_dir)
        .unwrap_or(&project_dir);
    util::info(format!(
        "Next steps:\n  cd {}\n  water run",
        display_path.display()
    ));

    // if which::which("git").is_ok() {
    //     std::process::Command::new("git")
    //         .arg("init")
    //         .current_dir(&project_dir)
    //         .output()?;
    //     util::info("✅ Git repository initialized");
    // }

    Ok(())
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
