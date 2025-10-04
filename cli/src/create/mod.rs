use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};

use crate::{
    config::{Config, Package, Swift},
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

    let crate_name = util::kebab_case(&display_name);
    let app_name = util::pascal_case(&display_name);

    let bundle_identifier = match args.bundle_identifier {
        Some(id) => id,
        None => Input::with_theme(&theme)
            .with_prompt("Bundle identifier")
            .default(format!("com.example.{crate_name}"))
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

    util::info(format!("Application: {display_name}"));
    util::info(format!("Crate name: {crate_name}"));
    util::info(format!("Xcode scheme: {app_name}"));
    util::info(format!("Bundle ID: {bundle_identifier}"));
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

    let templates = &["Android", "SwiftUI"];
    let template_idx = Select::with_theme(&theme)
        .with_prompt("Choose a project template")
        .items(templates)
        .default(0) // Android is the default
        .interact()?;

    prepare_directory(&project_dir)?;
    rust::create_rust_sources(&project_dir, &crate_name, &display_name)?;

    match templates[template_idx] {
        "Android" => {
            android::create_android_project(
                &project_dir,
                &app_name,
                &crate_name,
                &bundle_identifier,
            )?;
        }
        "SwiftUI" => {
            swift::create_xcode_project(&project_dir, &app_name, &crate_name, &bundle_identifier)?;
            let config = Config::new(
                Package {
                    name: crate_name.clone(),
                    display_name: display_name.clone(),
                    bundle_identifier,
                },
                Swift {
                    project_path: "apple".to_string(),
                    scheme: app_name.clone(),
                },
            );
            config.save(&project_dir)?;
        }
        _ => unreachable!(),
    }

    util::info("✅ Project created");
    util::info(format!(
        "Next steps:\n  cd {}\n  water run",
        project_dir.display()
    ));
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
