use std::{collections::HashMap, fs, path::Path};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use color_eyre::eyre::Result;

use super::{SwiftDependency, swift_backend_repo_url, template};
use waterui_cli::permission::ResolvedPermission;

/// Generate the Swift/Xcode portion of a `WaterUI` project.
///
/// # Errors
/// Returns an error if file writes fail or file permissions cannot be updated.
///
/// # Panics
/// Panics if bundled templates are missing or contain invalid data.
#[allow(clippy::too_many_lines)]
pub fn create_xcode_project(
    project_dir: &Path,
    app_name: &str,
    app_display_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
    swift_dependency: &SwiftDependency,
) -> Result<()> {
    create_xcode_project_with_permissions(
        project_dir,
        app_name,
        app_display_name,
        crate_name,
        bundle_identifier,
        swift_dependency,
        &[],
    )
}

/// Generate the Swift/Xcode portion of a `WaterUI` project with custom permissions.
///
/// # Errors
/// Returns an error if file writes fail or file permissions cannot be updated.
#[allow(clippy::too_many_lines)]
pub fn create_xcode_project_with_permissions(
    project_dir: &Path,
    app_name: &str,
    app_display_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
    swift_dependency: &SwiftDependency,
    permissions: &[ResolvedPermission],
) -> Result<()> {
    let apple_root = project_dir.join("apple");

    let mut context = HashMap::new();
    context.insert("APP_NAME", app_name.to_string());
    context.insert("APP_DISPLAY_NAME", app_display_name.to_string());
    context.insert("BUNDLE_IDENTIFIER", bundle_identifier.to_string());
    context.insert("CRATE_NAME", crate_name.to_string());

    let (package_reference_entry, package_reference_section) = match swift_dependency {
        SwiftDependency::Local { path } => {
            // Local package - use XCLocalSwiftPackageReference
            let local_path = pathdiff::diff_paths(path, &apple_root)
                .unwrap_or_else(|| path.clone());
            (
                r#"D01867782E6C82CA00802E96 /* XCLocalSwiftPackageReference "waterui-swift" */,"#.to_string(),
                format!(
                    r#"/* Begin XCLocalSwiftPackageReference section */
        D01867782E6C82CA00802E96 /* XCLocalSwiftPackageReference "waterui-swift" */ = {{
            isa = XCLocalSwiftPackageReference;
            relativePath = "{}";
        }};
/* End XCLocalSwiftPackageReference section */"#,
                    local_path.display()
                ),
            )
        }
        SwiftDependency::Git {
            version,
            branch,
            revision,
        } => {
            let requirement = if let Some(version) = version.as_ref() {
                format!(
                    "requirement = {{ \n\t\t\t\tkind = upToNextMajorVersion;\n\t\t\t\tminimumVersion = \"{version}\";\n\t\t\t\t}}"
                )
            } else if let Some(revision) = revision.as_ref() {
                format!(
                    "requirement = {{\n\t\t\t\tkind = revision;\n\t\t\t\trevision = \"{revision}\";\n\t\t\t}};"
                )
            } else {
                let branch = branch.as_deref().unwrap_or("main");
                format!(
                    "requirement = {{\n\t\t\t\tkind = branch;\n\t\t\t\tbranch = \"{branch}\";\n\t\t\t}};"
                )
            };

            let repo_url = swift_backend_repo_url();
            (
                r#"D01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference "waterui-swift" */,"#.to_string(),
                format!(
                    r#"/* Begin XCRemoteSwiftPackageReference section */
        D01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference "waterui-swift" */ = {{
            isa = XCRemoteSwiftPackageReference;
            repositoryURL = "{repo_url}";
            {requirement}
        }};
/* End XCRemoteSwiftPackageReference section */"#
                ),
            )
        }
    };

    context.insert("SWIFT_PACKAGE_REFERENCE_ENTRY", package_reference_entry);
    context.insert("SWIFT_PACKAGE_REFERENCE_SECTION", package_reference_section);

    // Generate iOS permission build settings (INFOPLIST_KEY_* entries)
    let ios_permission_keys = generate_ios_permission_keys(permissions);
    context.insert("IOS_PERMISSION_KEYS", ios_permission_keys);

    let templates = &template::TEMPLATES_DIR;
    let apple_template_dir = templates
        .get_dir("apple")
        .expect("apple template directory should exist");

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

/// Generate iOS permission build settings for Xcode's auto-generated Info.plist.
///
/// This generates `INFOPLIST_KEY_*` build settings that Xcode uses when
/// `GENERATE_INFOPLIST_FILE = YES` is set.
fn generate_ios_permission_keys(permissions: &[ResolvedPermission]) -> String {
    use std::collections::HashSet;

    let mut entries = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for perm in permissions {
        for (key, description) in perm.ios_plist_entries() {
            if seen.insert(key.clone()) {
                // Convert Info.plist key to INFOPLIST_KEY_* format
                // e.g., NSCameraUsageDescription -> INFOPLIST_KEY_NSCameraUsageDescription
                let escaped_description = description.replace('"', "\\\"");
                entries.push(format!(
                    "\t\t\t\tINFOPLIST_KEY_{} = \"{}\";",
                    key, escaped_description
                ));
            }
        }
    }

    entries.join("\n")
}
