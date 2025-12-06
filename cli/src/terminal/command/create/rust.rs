use color_eyre::eyre::Result;
use std::{collections::HashMap, path::Path};

use super::{ProjectDependencies, template};

/// Create the Rust crate scaffolding for a `WaterUI` project.
///
/// # Errors
/// Returns an error if any of the generated files cannot be written to disk.
///
/// # Panics
/// Panics if required embedded templates are missing or contain invalid UTF-8.
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

    context.insert("WATERUI_DEPS", deps.rust_toml_for_project(project_dir));

    let templates = &template::TEMPLATES_DIR;

    template::process_template_file(
        templates
            .get_file(".gitignore.tpl")
            .expect(".gitignore.tpl should exist"),
        &project_dir.join(".gitignore"),
        &context,
    )?;

    template::process_template_file(
        templates
            .get_file("Cargo.toml.tpl")
            .expect("Cargo.toml.tpl should exist"),
        &project_dir.join("Cargo.toml"),
        &context,
    )?;

    template::process_template_file(
        templates
            .get_file("lib.rs.tpl")
            .expect("lib.rs.tpl should exist"),
        &project_dir.join("src/lib.rs"),
        &context,
    )?;

    template::process_template_file(
        templates
            .get_file("build.rs.tpl")
            .expect("build.rs.tpl should exist"),
        &project_dir.join("build.rs"),
        &context,
    )?;

    Ok(())
}
