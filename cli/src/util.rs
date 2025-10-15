use color_eyre::eyre::{Result, bail};
use console::style;
use core::fmt::Display;
use heck::{AsKebabCase, AsPascalCase};
use serde_json::json;
use std::path::{Path, PathBuf};
use which::which;

use crate::output;

pub fn print_error(error: impl Display, hint: Option<&str>) {
    if output::global_output_format().is_json() {
        let mut value = json!({
            "reason": "error",
            "message": error.to_string(),
        });
        if let Some(hint) = hint {
            value["hint"] = json!(hint);
        }
        println!("{}", value);
        return;
    }

    let icon = style("✖").red();
    eprintln!("{} {}", icon, style("Error").red().bold());
    eprintln!("  {}", style(error.to_string()).red());
    if let Some(hint) = hint {
        eprintln!(
            "  {} {}",
            style("Hint:").yellow().bold(),
            style(hint).yellow()
        );
    }
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI manifest to have parent")
        .to_path_buf()
}

pub fn kebab_case(name: &str) -> String {
    let s = AsKebabCase(name).to_string();
    if s.is_empty() {
        "waterui-app".to_string()
    } else {
        s
    }
}

pub fn pascal_case(name: &str) -> String {
    let s = AsPascalCase(name).to_string();
    if s.is_empty() {
        "WaterUIApp".to_string()
    } else {
        s
    }
}

pub fn ensure_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

pub fn require_tool(tool: &str, hint: &str) -> Result<()> {
    if which(tool).is_ok() {
        Ok(())
    } else {
        bail!("{tool} not found. {hint}")
    }
}
