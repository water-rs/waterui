use anyhow::Result;
use std::{collections::HashMap, path::Path};

use super::{ProjectDependencies, template};

pub fn create_rust_sources(
    project_dir: &Path,
    crate_name: &str,
    author: &str,
    display_name: &str,
    deps: &ProjectDependencies,
) -> Result<()> {
    let mut context = HashMap::new();
    context.insert("CRATE_NAME", crate_name.to_string());
    context.insert("AUTHOR", author.to_string());
    context.insert("APP_DISPLAY_NAME", display_name.to_string());

    context.insert("WATERUI_DEPS", deps.rust_toml.clone());

    let templates = &template::TEMPLATES_DIR;

    template::process_template_file(
        templates.get_file(".gitignore.tpl").unwrap(),
        &project_dir.join(".gitignore"),
        &context,
    )?;

    template::process_template_file(
        templates.get_file("Cargo.toml.tpl").unwrap(),
        &project_dir.join("Cargo.toml"),
        &context,
    )?;

    template::process_template_file(
        templates.get_file("lib.rs.tpl").unwrap(),
        &project_dir.join("src/lib.rs"),
        &context,
    )?;

    Ok(())
}
