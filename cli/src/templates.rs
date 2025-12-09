//! Type-safe template scaffolding for `WaterUI` project backends.
//!
//! Uses `include_dir` to embed templates at compile time and provides
//! a type-safe substitution API for generating Apple and Android backend projects.

use std::{
    io,
    path::{Path, PathBuf},
};

const WATERUI_VERSION: &str = "0.1.0";
const WATERUI_FFI_VERSION: &str = "0.1.0";

use include_dir::{Dir, include_dir};
use smol::fs;

/// Normalize a path to use forward slashes for config files (Cargo.toml, Xcode projects, etc.)
/// This is necessary because Windows uses backslashes but these config files expect forward slashes.
fn normalize_path_for_config(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Embedded template directories.
mod embedded {
    use super::{Dir, include_dir};

    pub static APPLE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/apple");
    pub static ANDROID: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/android");
    pub static ROOT: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates");
}

/// Context for rendering templates with type-safe substitutions.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// The application display name (e.g., "My App")
    pub app_display_name: String,
    /// The application name for file/folder naming (e.g., "`MyApp`")
    pub app_name: String,
    /// The Rust crate name (e.g., "`my_app`")
    pub crate_name: String,
    /// The bundle identifier (e.g., "com.example.myapp")
    pub bundle_identifier: String,
    /// The author name
    pub author: String,
    /// Path to the Android backend (relative or absolute)
    pub android_backend_path: Option<PathBuf>,
    /// Whether to use remote dev backend (`JitPack`) instead of local
    pub use_remote_dev_backend: bool,
    /// Path to local `WaterUI` repository (for dev mode)
    pub waterui_path: Option<PathBuf>,
    /// Relative path from project root to where the Xcode/Android project is located.
    /// Used to compute correct relative paths. Defaults to "apple" for standard projects.
    /// For playground projects, this would be ".water/apple".
    pub backend_project_path: Option<PathBuf>,
}

impl TemplateContext {
    /// Render a template string by replacing all placeholders.
    #[must_use]
    pub fn render(&self, template: &str) -> String {
        template
            .replace("__APP_DISPLAY_NAME__", &self.app_display_name)
            .replace("__APP_NAME__", &self.app_name)
            .replace("__CRATE_NAME__", &self.crate_name)
            .replace("__BUNDLE_IDENTIFIER__", &self.bundle_identifier)
            .replace("__AUTHOR__", &self.author)
            .replace(
                "__ANDROID_BACKEND_PATH__",
                &self
                    .android_backend_path
                    .as_ref()
                    .map_or(String::new(), |p| normalize_path_for_config(p)),
            )
            .replace(
                "__USE_REMOTE_DEV_BACKEND__",
                if self.use_remote_dev_backend {
                    "true"
                } else {
                    "false"
                },
            )
            .replace(
                "__SWIFT_PACKAGE_REFERENCE_ENTRY__",
                &self.swift_package_reference_entry(),
            )
            .replace(
                "__SWIFT_PACKAGE_REFERENCE_SECTION__",
                &self.swift_package_reference_section(),
            )
            .replace("__IOS_PERMISSION_KEYS__", "")
            .replace("__ANDROID_PERMISSIONS__", "")
    }

    /// Transform a path by replacing "`AppName`" with the actual app name.
    #[must_use]
    pub fn transform_path(&self, path: &Path) -> PathBuf {
        let path_str = path.to_string_lossy();
        PathBuf::from(path_str.replace("AppName", &self.app_name))
    }

    /// Compute the relative path from the Xcode project to the WaterUI Swift backend.
    ///
    /// This accounts for the Xcode project being in a subdirectory (e.g., `.water/apple`).
    fn compute_backend_path(&self) -> Option<String> {
        let waterui_path = self.waterui_path.as_ref()?;

        // Count how many levels deep the Xcode project is from the project root
        // Default is "apple" (1 level), playground uses ".water/apple" (2 levels)
        let project_depth = self
            .backend_project_path
            .as_ref()
            .map_or(1, |p| p.components().count());

        // Build the relative path: go up `project_depth` levels, then to waterui_path/backends/apple
        let mut backend_path = String::new();
        for _ in 0..project_depth {
            backend_path.push_str("../");
        }
        backend_path.push_str(&normalize_path_for_config(waterui_path));
        backend_path.push_str("/backends/apple");

        Some(backend_path)
    }

    /// Generate the XCode package reference entry line for the project file.
    fn swift_package_reference_entry(&self) -> String {
        const PACKAGE_ID: &str = "D01867782E6C82CA00802E96";
        const INDENT: &str = "\t\t\t\t";

        match self.compute_backend_path() {
            Some(backend_path) => {
                format!(
                    "{INDENT}{PACKAGE_ID} /* XCLocalSwiftPackageReference \"{backend_path}\" */,"
                )
            }
            None => {
                format!(
                    "{INDENT}{PACKAGE_ID} /* XCRemoteSwiftPackageReference \"apple-backend\" */,"
                )
            }
        }
    }

    /// Generate the XCode package reference section for the project file.
    fn swift_package_reference_section(&self) -> String {
        const PACKAGE_ID: &str = "D01867782E6C82CA00802E96";
        const REPO_URL: &str = "https://github.com/user/waterui-apple.git";
        const MIN_VERSION: &str = "1.0.0";

        match self.compute_backend_path() {
            Some(backend_path) => {
                format!(
                    "/* Begin XCLocalSwiftPackageReference section */\n\
                    \t\t{PACKAGE_ID} /* XCLocalSwiftPackageReference \"{backend_path}\" */ = {{\n\
                    \t\t\tisa = XCLocalSwiftPackageReference;\n\
                    \t\t\trelativePath = \"{backend_path}\";\n\
                    \t\t}};\n\
                    /* End XCLocalSwiftPackageReference section */"
                )
            }
            None => {
                format!(
                    "/* Begin XCRemoteSwiftPackageReference section */\n\
                    \t\t{PACKAGE_ID} /* XCRemoteSwiftPackageReference \"apple-backend\" */ = {{\n\
                    \t\t\tisa = XCRemoteSwiftPackageReference;\n\
                    \t\t\trepositoryURL = \"{REPO_URL}\";\n\
                    \t\t\trequirement = {{\n\
                    \t\t\t\tkind = upToNextMajorVersion;\n\
                    \t\t\t\tminimumVersion = {MIN_VERSION};\n\
                    \t\t\t}};\n\
                    \t\t}};\n\
                    /* End XCRemoteSwiftPackageReference section */"
                )
            }
        }
    }
}

/// Scaffold a directory from embedded templates (non-recursive, uses stack).
async fn scaffold_dir(
    embedded_dir: &Dir<'_>,
    base_dir: &Path,
    ctx: &TemplateContext,
) -> io::Result<()> {
    // Use a stack to avoid async recursion (which requires boxing)
    let mut dirs_to_process = vec![embedded_dir];

    while let Some(current_dir) = dirs_to_process.pop() {
        // Process all files in this directory
        for file in current_dir.files() {
            let relative_path = file.path();

            // Determine if this is a template file and compute destination path
            let is_template = relative_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "tpl");

            let dest_path = if is_template {
                // Remove .tpl extension and transform path
                let without_tpl = relative_path.with_extension("");
                ctx.transform_path(&without_tpl)
            } else {
                // Binary file - just transform the path
                ctx.transform_path(relative_path)
            };

            let full_dest = base_dir.join(&dest_path);

            // Create parent directories
            if let Some(parent) = full_dest.parent() {
                fs::create_dir_all(parent).await?;
            }

            // Write file content
            if is_template {
                // Template file - render content
                let content = file
                    .contents_utf8()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-8"))?;
                let rendered = ctx.render(content);
                fs::write(&full_dest, rendered).await?;
            } else {
                // Binary file - copy as-is
                fs::write(&full_dest, file.contents()).await?;
            }
        }

        // Add subdirectories to the stack
        for subdir in current_dir.dirs() {
            dirs_to_process.push(subdir);
        }
    }

    Ok(())
}

/// Apple backend templates.
pub mod apple {
    use super::{Path, TemplateContext, embedded, fs, io, scaffold_dir};

    /// Write all Apple templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(&embedded::APPLE, base_dir, ctx).await?;

        // Make build-rust.sh executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script_path = base_dir.join("build-rust.sh");
            if script_path.exists() {
                let mut perms = fs::metadata(&script_path).await?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&script_path, perms).await?;
            }
        }

        Ok(())
    }
}

/// Android backend templates.
pub mod android {
    use super::{Path, TemplateContext, embedded, fs, io, scaffold_dir};

    /// Write all Android templates to the given directory.
    ///
    /// # Errors
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(&embedded::ANDROID, base_dir, ctx).await?;

        // Make gradlew executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let gradlew_path = base_dir.join("gradlew");
            if gradlew_path.exists() {
                let mut perms = fs::metadata(&gradlew_path).await?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&gradlew_path, perms).await?;
            }
        }

        // Create jniLibs directories
        for abi in ["arm64-v8a", "x86_64", "armeabi-v7a", "x86"] {
            let jni_dir = base_dir.join(format!("app/src/main/jniLibs/{abi}"));
            fs::create_dir_all(&jni_dir).await?;
        }

        Ok(())
    }
}

/// Root-level templates (Cargo.toml, lib.rs, .gitignore).
pub mod root {
    use crate::templates::{WATERUI_FFI_VERSION, WATERUI_VERSION};

    use super::{Path, TemplateContext, embedded, fs, io, normalize_path_for_config};

    /// Root template files (only .tpl files at the root level, excluding Cargo.toml).
    static ROOT_TEMPLATES: &[&str] = &["lib.rs.tpl", ".gitignore.tpl"];

    /// Write root templates to the given directory.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        // Generate Cargo.toml programmatically using toml_edit
        generate_cargo_toml(base_dir, ctx).await?;

        // Process remaining templates
        for template_name in ROOT_TEMPLATES {
            if let Some(file) = embedded::ROOT.get_file(template_name) {
                let dest_name = template_name.strip_suffix(".tpl").unwrap_or(template_name);
                let dest_path = if dest_name == "lib.rs" {
                    base_dir.join("src").join(dest_name)
                } else {
                    base_dir.join(dest_name)
                };

                // Create parent directories
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent).await?;
                }

                let content = file
                    .contents_utf8()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-8"))?;
                let rendered = ctx.render(content);
                fs::write(&dest_path, rendered).await?;
            }
        }
        Ok(())
    }

    /// Generate Cargo.toml programmatically using serde-compatible structs for type safety.
    async fn generate_cargo_toml(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        use serde::Serialize;
        use std::collections::BTreeMap;

        #[derive(Serialize)]
        struct CargoManifest {
            package: PackageSection,
            lib: LibSection,
            dependencies: BTreeMap<String, DependencyValue>,
            workspace: WorkspaceSection,
        }

        #[derive(Serialize)]
        struct PackageSection {
            name: String,
            version: String,
            edition: String,
            authors: Vec<String>,
        }

        #[derive(Serialize)]
        struct LibSection {
            #[serde(rename = "crate-type")]
            crate_type: Vec<String>,
        }

        #[derive(Serialize)]
        struct WorkspaceSection {}

        #[derive(Serialize)]
        #[serde(untagged)]
        enum DependencyValue {
            Simple(String),
            Detailed(DependencyDetail),
        }

        #[derive(Serialize)]
        struct DependencyDetail {
            path: String,
        }

        let mut dependencies = BTreeMap::new();

        if let Some(waterui_path) = &ctx.waterui_path {
            // Local path dependencies
            dependencies.insert(
                "waterui".to_string(),
                DependencyValue::Detailed(DependencyDetail {
                    path: normalize_path_for_config(waterui_path),
                }),
            );

            let ffi_path = waterui_path.join("ffi");
            dependencies.insert(
                "waterui-ffi".to_string(),
                DependencyValue::Detailed(DependencyDetail {
                    path: normalize_path_for_config(&ffi_path),
                }),
            );
        } else {
            // Registry dependencies
            dependencies.insert(
                "waterui".to_string(),
                DependencyValue::Simple(WATERUI_VERSION.to_string()),
            );
            dependencies.insert(
                "waterui-ffi".to_string(),
                DependencyValue::Simple(WATERUI_FFI_VERSION.to_string()),
            );
        }

        let manifest = CargoManifest {
            package: PackageSection {
                name: ctx.crate_name.clone(),
                version: "0.1.0".to_string(),
                edition: "2024".to_string(),
                authors: vec![ctx.author.clone()],
            },
            lib: LibSection {
                crate_type: vec![
                    "staticlib".to_string(),
                    "cdylib".to_string(),
                    "rlib".to_string(),
                ],
            },
            dependencies,
            workspace: WorkspaceSection {},
        };

        // Serialize to TOML
        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let cargo_path = base_dir.join("Cargo.toml");
        fs::write(&cargo_path, toml_string).await?;

        Ok(())
    }
}
