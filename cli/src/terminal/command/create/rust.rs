use color_eyre::eyre::{Context, Result};
use std::{collections::HashMap, fs, path::Path};

use super::{ProjectDependencies, template};
use crate::util;

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

    context.insert("WATERUI_DEPS", deps.rust_toml.clone());

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

    copy_ffi_header(project_dir)?;

    Ok(())
}

fn copy_ffi_header(project_dir: &Path) -> Result<()> {
    let source = util::workspace_root().join("ffi/waterui.h");
    if !source.exists() {
        return Ok(());
    }

    let destination_dir = project_dir.join("ffi");
    util::ensure_directory(&destination_dir)?;
    let destination = destination_dir.join("waterui.h");
    fs::copy(&source, &destination).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}
