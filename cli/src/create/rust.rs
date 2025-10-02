use anyhow::Result;
use std::{collections::HashMap, path::Path};

use crate::util;

use super::template;

pub fn create_rust_sources(project_dir: &Path, crate_name: &str, display_name: &str) -> Result<()> {
    let mut context = HashMap::new();
    context.insert("CRATE_NAME", crate_name.to_string());
    context.insert("DISPLAY_NAME", display_name.to_string());

    let template_root = util::workspace_root().join("cli/src/templates");

    template::process_template_file(
        &template_root.join(".gitignore.tpl"),
        &project_dir.join(".gitignore"),
        &context,
    )?;

    template::process_template_file(
        &template_root.join("Cargo.toml.tpl"),
        &project_dir.join("Cargo.toml"),
        &context,
    )?;

    template::process_template_file(
        &template_root.join("lib.rs.tpl"),
        &project_dir.join("src/lib.rs"),
        &context,
    )?;

    Ok(())
}
