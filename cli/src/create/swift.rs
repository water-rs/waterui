use std::{collections::HashMap, fs, path::Path, process::Command};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail};

use super::{SwiftDependency, WATERUI_GIT_URL, template};
use crate::util;

const DEV_SWIFT_PACKAGE_RELATIVE_PATH: &str = "../.waterui/swift/backends/swift";
const DEV_SWIFT_BRANCH: &str = "main";

pub fn create_xcode_project(
    project_dir: &Path,
    app_name: &str,
    app_display_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
    development_team: &str,
    swift_dependency: &SwiftDependency,
) -> Result<()> {
    let apple_root = project_dir.join("apple");
    let lib_name = crate_name.replace('-', "_");

    let mut context = HashMap::new();
    context.insert("APP_NAME", app_name.to_string());
    context.insert("APP_DISPLAY_NAME", app_display_name.to_string());
    context.insert("LIB_NAME", lib_name.to_string());
    context.insert("BUNDLE_IDENTIFIER", bundle_identifier.to_string());
    context.insert("DEVELOPMENT_TEAM", development_team.to_string());

    match swift_dependency {
        SwiftDependency::Remote { requirement } => {
            context.insert(
                "SWIFT_PACKAGE_REFERENCE_ENTRY",
                "\t\t\tD01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference \"WaterUI\" */,\n"
                    .to_string(),
            );
            context.insert(
                "SWIFT_PACKAGE_REFERENCE_SECTION",
                format!(
                    "/* Begin XCRemoteSwiftPackageReference section */
\t\tD01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference \"WaterUI\" */ = {{
\t\t\tisa = XCRemoteSwiftPackageReference;
\t\t\trepositoryURL = \"{url}\";
\t\t\trequirement = {{
{requirement}
\t\t\t}};
\t\t}};
/* End XCRemoteSwiftPackageReference section */
",
                    url = WATERUI_GIT_URL,
                    requirement = requirement
                ),
            );
        }
        SwiftDependency::Dev => {
            ensure_dev_swift_package(project_dir)?;
            context.insert(
                "SWIFT_PACKAGE_REFERENCE_ENTRY",
                format!(
                    "\t\t\tD01867782E6C82CA00802E96 /* XCLocalSwiftPackageReference \"{}\" */,\n",
                    DEV_SWIFT_PACKAGE_RELATIVE_PATH
                ),
            );
            context.insert(
                "SWIFT_PACKAGE_REFERENCE_SECTION",
                format!(
                    "/* Begin XCLocalSwiftPackageReference section */
\t\tD01867782E6C82CA00802E96 /* XCLocalSwiftPackageReference \"{path}\" */ = {{
\t\t\tisa = XCLocalSwiftPackageReference;
\t\t\trelativePath = {path};
\t\t}};
/* End XCLocalSwiftPackageReference section */
",
                    path = DEV_SWIFT_PACKAGE_RELATIVE_PATH
                ),
            );
        }
    }

    let templates = &template::TEMPLATES_DIR;
    let apple_template_dir = templates.get_dir("apple").expect("apple template directory should exist");

    template::process_template_directory(apple_template_dir, &apple_root, &context)?;

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

fn ensure_dev_swift_package(project_dir: &Path) -> Result<()> {
    let checkout_dir = project_dir.join(".waterui/swift");
    util::ensure_directory(
        checkout_dir
            .parent()
            .context("Swift checkout directory should have a parent")?,
    )?;

    if checkout_dir.exists() {
        let fetch_status = Command::new("git")
            .arg("-C")
            .arg(&checkout_dir)
            .args(["fetch", "--quiet", "origin", DEV_SWIFT_BRANCH])
            .status()
            .context("Failed to fetch WaterUI Swift dev package")?;
        if !fetch_status.success() {
            bail!("git fetch for Swift dev package failed with status {fetch_status}");
        }

        let reset_status = Command::new("git")
            .arg("-C")
            .arg(&checkout_dir)
            .args([
                "reset",
                "--quiet",
                "--hard",
                &format!("origin/{DEV_SWIFT_BRANCH}"),
            ])
            .status()
            .context("Failed to reset WaterUI Swift dev package to origin")?;
        if !reset_status.success() {
            bail!("git reset for Swift dev package failed with status {reset_status}");
        }
    } else {
        let clone_status = Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--branch",
                DEV_SWIFT_BRANCH,
                WATERUI_GIT_URL,
            ])
            .arg(&checkout_dir)
            .status()
            .context("Failed to clone WaterUI Swift dev package")?;
        if !clone_status.success() {
            bail!("git clone for Swift dev package failed with status {clone_status}");
        }
    }

    Ok(())
}
