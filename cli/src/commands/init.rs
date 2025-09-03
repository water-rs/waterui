use crate::cargo_config::CargoToml;
use crate::config::WaterConfig;
use anyhow::{Context, Result};
use colored::Colorize;
use std::env;
use std::fs;
use std::path::Path;

const LIB_RS_TEMPLATE: &str = include_str!("../../templates/lib.rs");
const GITIGNORE_TEMPLATE: &str = include_str!("../../templates/gitignore");

pub fn execute() -> Result<()> {
    let current_dir = env::current_dir()?;
    let project_name = current_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("waterui-project")
        .to_string();

    if Path::new("Water.toml").exists() {
        anyhow::bail!("Water.toml already exists in this directory");
    }

    if Path::new("Cargo.toml").exists() {
        println!("{} Cargo.toml already exists, skipping...", "⚠️ ".yellow());
    } else {
        let cargo_toml = CargoToml::new(project_name.clone());
        fs::write("Cargo.toml", cargo_toml.to_toml_string()?)
            .context("Failed to create Cargo.toml")?;
    }

    println!(
        "{} Initializing WaterUI project in current directory...",
        "✨".green()
    );

    if !Path::new("src").exists() {
        fs::create_dir("src").context("Failed to create src directory")?;
    }

    let lib_path = Path::new("src/lib.rs");
    if lib_path.exists() {
        println!("{} src/lib.rs already exists, skipping...", "⚠️ ".yellow());
    } else {
        fs::write(lib_path, LIB_RS_TEMPLATE).context("Failed to create src/lib.rs")?;
    }

    if !Path::new(".gitignore").exists() {
        fs::write(".gitignore", GITIGNORE_TEMPLATE).context("Failed to create .gitignore")?;
    }

    let authors = vec![
        env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "Unknown".to_string()),
    ];

    let mut config = WaterConfig::new(project_name, authors);

    config.add_dependency("waterui".to_string(), "0.1.0".to_string());
    config.add_dependency("nami".to_string(), "0.1.0".to_string());

    let config_content = config.to_toml_string()?;
    fs::write("Water.toml", config_content).context("Failed to write Water.toml")?;

    println!("{} WaterUI project initialized successfully!", "✅".green());
    println!();
    println!("  {}", "Next steps:".bold());
    println!("    cargo build");
    println!("    cargo test");

    println!();
    println!("  {}", "Configuration:".bold());
    println!("    {} - WaterUI configuration", "Water.toml".cyan());

    Ok(())
}
