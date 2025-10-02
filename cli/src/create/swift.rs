use std::{collections::HashMap, fs, path::Path};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

use crate::util;

use super::template;

pub fn create_xcode_project(
    project_dir: &Path,
    app_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
) -> Result<()> {
    let apple_root = project_dir.join("apple");
    let lib_name = crate_name.replace('-', "_");

    let mut context = HashMap::new();
    context.insert("APP_NAME", app_name.to_string());
    context.insert("LIB_NAME", lib_name.to_string());
    context.insert("BUNDLE_IDENTIFIER", bundle_identifier.to_string());

    let template_dir = util::workspace_root().join("cli/src/templates/apple");

    template::process_template_directory(&template_dir, &apple_root, &context)?;

    copy_swift_backend(&apple_root.join("WaterUI"))?;

    let build_script_path = apple_root.join("build-rust.sh");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&build_script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&build_script_path, perms)?;
    }

    let xcconfig = apple_root.join("rust_build_info.xcconfig");
    fs::write(xcconfig, "RUST_LIBRARY_PATH=\n")?;

    Ok(())
}

fn copy_swift_backend(destination: &Path) -> Result<()> {
    let source_root = util::workspace_root().join("backends/swift");
    if !source_root.exists() {
        bail!(
            "Swift backend sources not found at {}. Ensure you are running the CLI from the WaterUI workspace.",
            source_root.display()
        );
    }

    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    util::ensure_directory(destination)?;

    for entry in WalkDir::new(&source_root).into_iter() {
        let entry = entry?;
        let relative = match entry.path().strip_prefix(&source_root) {
            Ok(rel) if rel.as_os_str().is_empty() => continue,
            Ok(rel) => rel,
            Err(_) => continue,
        };

        if should_skip(relative) {
            continue;
        }

        let target_path = destination.join(relative);
        if entry.file_type().is_dir() {
            util::ensure_directory(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                util::ensure_directory(parent)?;
            }
            fs::copy(entry.path(), &target_path)
                .with_context(|| format!("failed to copy {}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| match component {
        std::path::Component::Normal(name) => {
            matches!(
                name.to_str(),
                Some(".git") | Some(".build") | Some(".swiftpm") | Some(".DS_Store")
            )
        }
        _ => false,
    })
}
