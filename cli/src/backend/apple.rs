use core::fmt::Display;

use color_eyre::{Section, eyre};
use heck::{ToPascalCase, ToSnakeCase, ToUpperCamelCase};
use thiserror::Error;

use crate::{backend::Backend, doctor::ToolchainIssue, impl_display};

#[derive(Debug)]
pub struct Apple;

impl_display!(Apple, "apple");

#[derive(Debug, Clone, Error)]
pub enum AppleToolchainIssue {
    #[error("Xcode is not installed.")]
    XcodeNotInstalled,
    #[error("Xcode Command Line Tools are not installed.")]
    CommandLineToolsNotInstalled,
}

impl ToolchainIssue for AppleToolchainIssue {
    fn suggestion(&self) -> String {
        match self {
            Self::XcodeNotInstalled => "Install Xcode from the App Store.".to_string(),
            Self::CommandLineToolsNotInstalled => {
                "Install Xcode Command Line Tools by running `xcode-select --install`.".to_string()
            }
        }
    }
}

impl Backend for Apple {
    type ToolchainIssue = AppleToolchainIssue;

    fn init(&self, project: &crate::project::Project, dev: bool) -> eyre::Result<()> {
        let bundle_identifier = format!(
            "com.{}.{}",
            project.author().to_snake_case(),
            project.identifier()
        );
        init_backend(project, &bundle_identifier, dev)
    }

    fn is_existing(&self, project: &crate::project::Project) -> bool {
        todo!()
    }

    fn clean(&self, project: &crate::project::Project) -> eyre::Result<()> {
        todo!()
    }

    fn check_requirements(
        &self,
        project: &crate::project::Project,
    ) -> Result<(), Vec<Self::ToolchainIssue>> {
        todo!()
    }
}

fn init_backend(
    project: &crate::project::Project,
    bundle_identifier: &str,

    dev: bool,
) -> eyre::Result<()> {
    // Implementation for initializing Apple backend

    Ok(())
}

fn clean(project: &crate::project::Project) -> eyre::Result<()> {
    // run command, clean xcode build artifacts
    let ident = project.identifier().to_upper_camel_case();
    let status = std::process::Command::new("xcodebuild")
        .arg("-workspace")
        .arg(format!("apple/{}.xcworkspace", ident))
        .arg("-scheme")
        .arg(ident)
        .arg("clean")
        .current_dir(project.root())
        .status()?;

    if !status.success() {
        return Err(
            eyre::eyre!("Failed to clean Xcode project.").with_section(move || status.to_string())
        );
    }

    Ok(())
}

/*


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
    context.insert("LIB_NAME", lib_name.to_string());
    context.insert("BUNDLE_IDENTIFIER", bundle_identifier.to_string());
    context.insert("CRATE_NAME", crate_name.to_string());

    let SwiftDependency::Git { version, branch } = swift_dependency;

    let requirement = if let Some(version) = version {
        format!(
            "requirement = {{ \n\t\t\t\tkind = upToNextMajorVersion;\n\t\t\t\tminimumVersion = \"{version}\";\n\t\t\t\t}}"
        )
    } else {
        let branch = branch.as_deref().unwrap_or("main");
        format!(
            "requirement = {{\n\t\t\t\tkind = branch;\n\t\t\t\tbranch = \"{branch}\";\n\t\t\t}};"
        )
    };

    context.insert(
        "SWIFT_PACKAGE_REFERENCE_ENTRY",
        r#"D01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference "waterui-swift" */
,"#
            .to_string(),
    );

    let repo_url = std::env::var("WATERUI_SWIFT_BACKEND_URL")
        .unwrap_or_else(|_| SWIFT_BACKEND_GIT_URL.to_string());

    context.insert(
        "SWIFT_PACKAGE_REFERENCE_SECTION",
        format!(
r#" /* Begin XCRemoteSwiftPackageReference section */
D01867782E6C82CA00802E96 /* XCRemoteSwiftPackageReference "waterui-swift" */
 = {{
            isa = XCRemoteSwiftPackageReference;
            repositoryURL = "{}";
            {}
        }};
/* End XCRemoteSwiftPackageReference section */
"#,
            repo_url, requirement
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


*/
