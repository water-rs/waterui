use std::{collections::HashMap, fs, path::Path};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use color_eyre::eyre::Result;

use super::{SwiftDependency, swift_backend_repo_url, template};

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
    let apple_root = project_dir.join("apple");
    let lib_name = crate_name.replace('-', "_");

    let mut context = HashMap::new();
    context.insert("APP_NAME", app_name.to_string());
    context.insert("APP_DISPLAY_NAME", app_display_name.to_string());
    context.insert("LIB_NAME", lib_name);
    context.insert("BUNDLE_IDENTIFIER", bundle_identifier.to_string());
    context.insert("CRATE_NAME", crate_name.to_string());

    let SwiftDependency::Git {
        version,
        branch,
        revision,
    } = swift_dependency;

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

    context.insert(
        "SWIFT_PACKAGE_REFERENCE_ENTRY",
        r#"D01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference "waterui-swift" */,"#
            .to_string(),
    );

    let repo_url = swift_backend_repo_url();

    context.insert(
        "SWIFT_PACKAGE_REFERENCE_SECTION",
        format!(
            r#"/* Begin XCRemoteSwiftPackageReference section */
        D01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference "waterui-swift" */ = {{
            isa = XCRemoteSwiftPackageReference;
            repositoryURL = "{repo_url}";
            {requirement}
        }};
/* End XCRemoteSwiftPackageReference section */"#
        ),
    );

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
