use std::{collections::HashMap, fs, path::Path};

use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};

use crate::util;

pub static TEMPLATES_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates");

pub fn process_template_directory(
    template_dir: &Dir,
    output_dir: &Path,
    context: &HashMap<&str, String>,
) -> Result<()> {
    for entry in template_dir.entries() {
        let relative_path = entry
            .path()
            .strip_prefix(template_dir.path())
            .unwrap_or(entry.path());

        let mut dest_path_str = relative_path.to_str().unwrap().to_string();

        if let Some(app_name) = context.get("APP_NAME") {
            dest_path_str = dest_path_str.replace("AppName", app_name);
        }
        if let Some(lib_name) = context.get("LIB_NAME") {
            dest_path_str = dest_path_str.replace("__LIB_NAME__", lib_name);
        }
        if let Some(bundle_id) = context.get("BUNDLE_IDENTIFIER") {
            dest_path_str = dest_path_str.replace("__BUNDLE_IDENTIFIER__", bundle_id);
        }

        let dest_path = output_dir.join(&dest_path_str);

        match entry {
            include_dir::DirEntry::Dir(dir) => {
                util::ensure_directory(&dest_path)?;
                process_template_directory(dir, &dest_path, context)?;
            }
            include_dir::DirEntry::File(file) => {
                if let Some(ext) = file.path().extension() {
                    if ext == "tpl" {
                        process_template_file(file, &dest_path.with_extension(""), context)?;
                    } else {
                        if let Some(parent) = dest_path.parent() {
                            util::ensure_directory(parent)?;
                        }
                        fs::write(&dest_path, file.contents()).with_context(|| {
                            format!("Failed to copy file: {}", file.path().display())
                        })?;
                    }
                } else {
                    if let Some(parent) = dest_path.parent() {
                        util::ensure_directory(parent)?;
                    }
                    fs::write(&dest_path, file.contents()).with_context(|| {
                        format!("Failed to copy file: {}", file.path().display())
                    })?;
                }
            }
        }
    }

    Ok(())
}

pub fn process_template_file(
    template_file: &include_dir::File,
    output_path: &Path,
    context: &HashMap<&str, String>,
) -> Result<()> {
    let mut content = template_file.contents_utf8().unwrap().to_string();

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
