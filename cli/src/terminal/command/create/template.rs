use std::{collections::HashMap, fs, hash::BuildHasher, path::Path};

use color_eyre::eyre::{Context, Result};
use include_dir::{Dir, include_dir};

use crate::util;

pub static TEMPLATES_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates");

/// Recursively process every entry in `template_dir`, rendering templated files into
/// `output_dir` using the provided context map.
///
/// # Errors
/// Returns an error if reading a template or writing the rendered output fails.
///
/// # Panics
/// Panics if any template path cannot be represented as UTF-8; this indicates a bug in the
/// bundled templates.
pub fn process_template_directory<S: BuildHasher>(
    template_dir: &Dir,
    output_dir: &Path,
    context: &HashMap<&str, String, S>,
) -> Result<()> {
    for entry in template_dir.entries() {
        let relative_path = entry
            .path()
            .strip_prefix(template_dir.path())
            .unwrap_or_else(|_| entry.path());

        let mut dest_path_str = relative_path
            .to_str()
            .expect("path should be valid UTF-8")
            .to_string();

        if let Some(app_name) = context.get("APP_NAME") {
            dest_path_str = dest_path_str.replace("AppName", app_name);
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

/// Render a single template file into `output_path` using the provided context.
///
/// # Errors
/// Returns an error if the file cannot be written to disk.
///
/// # Panics
/// Panics if the bundled template contents are not valid UTF-8.
pub fn process_template_file<S: BuildHasher>(
    template_file: &include_dir::File,
    output_path: &Path,
    context: &HashMap<&str, String, S>,
) -> Result<()> {
    let mut rendered = template_file
        .contents_utf8()
        .expect("template file should be valid UTF-8")
        .to_string();

    for (key, value) in context {
        rendered = rendered.replace(&format!("__{key}__"), value);
    }

    if let Some(parent) = output_path.parent() {
        util::ensure_directory(parent)?;
    }

    fs::write(output_path, rendered)
        .with_context(|| format!("Failed to write file: {}", output_path.display()))?;

    Ok(())
}
