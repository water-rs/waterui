use crate::config::WaterConfig;
use crate::template;
use anyhow::{Context, Result};
use colored::Colorize;
use std::env;
use std::fs;
use std::path::PathBuf;

pub fn execute(name: &str, path: Option<PathBuf>) -> Result<()> {
    let project_path = if let Some(p) = path {
        p.join(name)
    } else {
        env::current_dir()?.join(name)
    };

    if project_path.exists() {
        anyhow::bail!("Directory '{}' already exists", project_path.display());
    }

    println!(
        "{} Creating new WaterUI project '{}'...",
        "✨".green(),
        name.cyan()
    );

    fs::create_dir_all(&project_path).context("Failed to create project directory")?;

    template::create_project_structure(&project_path, name)?;

    let authors = vec![
        env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "Unknown".to_string()),
    ];

    let mut config = WaterConfig::new(name.to_string(), authors);

    config.add_dependency("waterui".to_string(), "0.1.0".to_string());
    config.add_dependency("nami".to_string(), "0.1.0".to_string());

    let config_content = config.to_toml_string()?;
    fs::write(project_path.join("Water.toml"), config_content)
        .context("Failed to write Water.toml")?;

    println!("{} Project created successfully!", "✅".green());
    println!();
    println!("  {}", "Next steps:".bold());
    println!("    cd {}", name.cyan());
    println!("    cargo build");
    println!("    cargo test");

    println!();
    println!("  {}", "Project structure:".bold());
    println!("    {} - WaterUI configuration", "Water.toml".cyan());
    println!("    {} - Cargo configuration", "Cargo.toml".cyan());
    println!("    {} - Library entry point", "src/lib.rs".cyan());

    Ok(())
}
