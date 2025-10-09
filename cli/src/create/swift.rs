use std::{collections::HashMap, fs, path::Path};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result};
use include_dir::{Dir, DirEntry, include_dir};

use crate::util;

use super::template;

static SWIFT_BACKEND_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../backends/swift");

pub fn create_xcode_project(
    project_dir: &Path,
    app_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
    development_team: &str,
) -> Result<()> {
    let apple_root = project_dir.join("apple");
    let lib_name = crate_name.replace('-', "_");

    let mut context = HashMap::new();
    context.insert("APP_NAME", app_name.to_string());
    context.insert("LIB_NAME", lib_name.to_string());
    context.insert("BUNDLE_IDENTIFIER", bundle_identifier.to_string());
    context.insert("DEVELOPMENT_TEAM", development_team.to_string());

    let templates = &template::TEMPLATES_DIR;
    let apple_template_dir = templates.get_dir("apple").unwrap();

    template::process_template_directory(apple_template_dir, &apple_root, &context)?;

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
    util::ensure_directory(destination)?;
    extract_dir(&SWIFT_BACKEND_DIR, destination)
}

fn extract_dir(dir: &Dir, to: &Path) -> Result<()> {
    for entry in dir.entries() {
        let path = to.join(entry.path());
        match entry {
            DirEntry::File(file) => {
                if should_skip(file.path()) {
                    continue;
                }
                fs::write(path, file.contents())?;
            }
            DirEntry::Dir(dir) => {
                if should_skip(dir.path()) {
                    continue;
                }
                fs::create_dir_all(&path)?;
                extract_dir(dir, &path)?;
            }
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
