use anyhow::Result;
use std::{collections::HashMap, path::Path};

use super::template;

pub fn create_rust_sources(
    project_dir: &Path,
    crate_name: &str,
    display_name: &str,
    author: &str,
    dev: bool,
) -> Result<()> {
    let mut context = HashMap::new();
    context.insert("CRATE_NAME", crate_name.to_string());
    context.insert("DISPLAY_NAME", display_name.to_string());
    context.insert("AUTHOR", author.to_string());

    let waterui_deps = if dev {
        r#"waterui = { git = "https://github.com/water-rs/waterui" }
waterui-ffi = { git = "https://github.com/water-rs/waterui" }"#
    } else {
        r#"waterui = "0.1"
waterui-ffi = "0.1""#
    };
    context.insert("WATERUI_DEPS", waterui_deps.to_string());

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
