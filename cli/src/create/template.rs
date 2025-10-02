use std::{
    collections::HashMap,
    fs,
    path::Path,
};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::util;

pub fn process_template_directory(
    template_dir: &Path,
    output_dir: &Path,
    context: &HashMap<&str, String>,
) -> Result<()> {
    for entry in WalkDir::new(template_dir).min_depth(1) {
        let entry = entry?;
        let template_path = entry.path();

        let relative_path = template_path.strip_prefix(template_dir)?;
        let mut dest_path_str = relative_path.to_str().unwrap().to_string();

        for (key, value) in context {
            dest_path_str = dest_path_str.replace(key, value);
        }

        let dest_path = output_dir.join(&dest_path_str);

        if entry.file_type().is_dir() {
            util::ensure_directory(&dest_path)?;
        } else if let Some(ext) = template_path.extension() {
            if ext == "tpl" {
                process_template_file(template_path, &dest_path.with_extension(""), context)?;
            } else {
                if let Some(parent) = dest_path.parent() {
                    util::ensure_directory(parent)?;
                }
                fs::copy(template_path, &dest_path)
                    .with_context(|| format!("Failed to copy file: {}", template_path.display()))?;
            }
        }
    }

    Ok(())
}

pub fn process_template_file(
    template_path: &Path,
    output_path: &Path,
    context: &HashMap<&str, String>,
) -> Result<()> {
    let mut content = fs::read_to_string(template_path)
        .with_context(|| format!("Failed to read template file: {}", template_path.display()))?;

    for (key, value) in context {
        content = content.replace(&format!("__{}__", key), value);
    }

    if let Some(parent) = output_path.parent() {
        util::ensure_directory(parent)?;
    }

    fs::write(output_path, content)
        .with_context(|| format!("Failed to write file: {}", output_path.display()))?;

    Ok(())
}
