//! Type-safe template scaffolding for `WaterUI` project backends.
//!
//! Uses `include_dir` to embed templates at compile time and provides
//! a type-safe substitution API for generating Apple and Android backend projects.

use std::{
    io,
    path::{Path, PathBuf},
};

const WATERUI_VERSION: &str = "0.2";
const WATERUI_FFI_VERSION: &str = "0.2";

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
    /// Android permissions to include in the manifest (e.g., "internet", "camera")
    pub android_permissions: Vec<String>,
}

impl TemplateContext {
    /// Render a template string by replacing all placeholders.
    #[must_use]
    pub fn render(&self, template: &str) -> String {
        // Android namespace must be a valid Java package name (no hyphens)
        let android_namespace = self.bundle_identifier.replace('-', "_");

        template
            .replace("__APP_DISPLAY_NAME__", &self.app_display_name)
            .replace("__APP_NAME__", &self.app_name)
            .replace("__CRATE_NAME__", &self.crate_name)
            .replace("__ANDROID_NAMESPACE__", &android_namespace)
            .replace("__BUNDLE_IDENTIFIER__", &self.bundle_identifier)
            .replace("__AUTHOR__", &self.author)
            .replace(
                "__ANDROID_BACKEND_PATH__",
                &self.compute_android_backend_path().unwrap_or_default(),
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
            .replace("__ANDROID_PERMISSIONS__", &self.android_permissions_xml())
            .replace(
                "__PROJECT_ROOT_RELATIVE_PATH__",
                &self.project_root_relative_path(),
            )
    }

    /// Transform a path by replacing "`AppName`" with the actual app name.
    #[must_use]
    pub fn transform_path(&self, path: &Path) -> PathBuf {
        let path_str = path.to_string_lossy();
        PathBuf::from(path_str.replace("AppName", &self.app_name))
    }

    /// Compute the relative path from the backend project to a `WaterUI` backend.
    ///
    /// This accounts for the project being in a subdirectory (e.g., `.water/android`).
    fn compute_relative_backend_path(&self, backend_subdir: &str) -> Option<String> {
        let waterui_path = self.waterui_path.as_ref()?;

        // If `waterui_path` is absolute, use it directly. This avoids producing invalid
        // paths like `../../../..//Users/...` in generated config files.
        if waterui_path.is_absolute() {
            let absolute_backend_path = waterui_path.join("backends").join(backend_subdir);
            return Some(normalize_path_for_config(&absolute_backend_path));
        }

        // Count how many levels deep the project is from the project root
        // Default is 1 level (e.g., "android"), playground uses 2 levels (e.g., ".water/android")
        let project_depth = self
            .backend_project_path
            .as_ref()
            .map_or(1, |p| p.components().count());

        // Build the relative path: go up `project_depth` levels, then to waterui_path/backends/<backend>.
        // Use `PathBuf` joins to avoid accidental `//` sequences and to keep behavior consistent
        // across platforms.
        let mut backend_path = PathBuf::new();
        for _ in 0..project_depth {
            backend_path.push("..");
        }
        backend_path.push(waterui_path);
        backend_path.push("backends");
        backend_path.push(backend_subdir);

        Some(normalize_path_for_config(&backend_path))
    }

    /// Compute the relative path from the Xcode project to the `WaterUI` Swift backend.
    fn compute_apple_backend_path(&self) -> Option<String> {
        self.compute_relative_backend_path("apple")
    }

    /// Compute the relative path from the Android project to the `WaterUI` Android backend.
    fn compute_android_backend_path(&self) -> Option<String> {
        self.compute_relative_backend_path("android")
    }

    /// Compute the relative path from the backend project directory to the project root.
    ///
    /// For a backend at `apple/`, returns `..` (go up 1 level).
    /// For a backend at `.water/apple/`, returns `../..` (go up 2 levels).
    fn project_root_relative_path(&self) -> String {
        let depth = self
            .backend_project_path
            .as_ref()
            .map_or(1, |p| p.components().count());

        (0..depth).map(|_| "..").collect::<Vec<_>>().join("/")
    }

    /// Generate Android permission XML entries for the manifest.
    fn android_permissions_xml(&self) -> String {
        if self.android_permissions.is_empty() {
            return String::new();
        }

        self.android_permissions
            .iter()
            .map(|perm| {
                let android_perm = match perm.to_lowercase().as_str() {
                    "internet" => "android.permission.INTERNET",
                    "camera" => "android.permission.CAMERA",
                    "microphone" => "android.permission.RECORD_AUDIO",
                    "location" => "android.permission.ACCESS_FINE_LOCATION",
                    "coarse_location" => "android.permission.ACCESS_COARSE_LOCATION",
                    "storage" => "android.permission.READ_EXTERNAL_STORAGE",
                    "write_storage" => "android.permission.WRITE_EXTERNAL_STORAGE",
                    "bluetooth" => "android.permission.BLUETOOTH",
                    "bluetooth_admin" => "android.permission.BLUETOOTH_ADMIN",
                    "vibrate" => "android.permission.VIBRATE",
                    "wake_lock" => "android.permission.WAKE_LOCK",
                    // Allow raw Android permission names
                    other => return format!("    <uses-permission android:name=\"{other}\" />"),
                };
                format!("    <uses-permission android:name=\"{android_perm}\" />")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Generate the `XCode` package reference entry line for the project file.
    fn swift_package_reference_entry(&self) -> String {
        const PACKAGE_ID: &str = "D01867782E6C82CA00802E96";
        const INDENT: &str = "\t\t\t\t";

        self.compute_apple_backend_path().map_or_else(
            || {
                format!(
                    "{INDENT}{PACKAGE_ID} /* XCRemoteSwiftPackageReference \"apple-backend\" */,"
                )
            },
            |backend_path| {
                format!(
                    "{INDENT}{PACKAGE_ID} /* XCLocalSwiftPackageReference \"{backend_path}\" */,"
                )
            },
        )
    }

    /// Generate the `XCode` package reference section for the project file.
    fn swift_package_reference_section(&self) -> String {
        const PACKAGE_ID: &str = "D01867782E6C82CA00802E96";
        const REPO_URL: &str = "https://github.com/user/waterui-apple.git";
        const MIN_VERSION: &str = "1.0.0";

        self.compute_apple_backend_path().map_or_else(
            || {
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
            },
            |backend_path| {
                format!(
                    "/* Begin XCLocalSwiftPackageReference section */\n\
                    \t\t{PACKAGE_ID} /* XCLocalSwiftPackageReference \"{backend_path}\" */ = {{\n\
                    \t\t\tisa = XCLocalSwiftPackageReference;\n\
                    \t\t\trelativePath = \"{backend_path}\";\n\
                    \t\t}};\n\
                    /* End XCLocalSwiftPackageReference section */"
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::TemplateContext;
    use std::path::PathBuf;

    fn ctx(
        waterui_path: Option<PathBuf>,
        backend_project_path: Option<PathBuf>,
    ) -> TemplateContext {
        TemplateContext {
            app_display_name: String::new(),
            app_name: String::new(),
            crate_name: String::new(),
            bundle_identifier: "com.example.test".to_string(),
            author: String::new(),
            android_backend_path: None,
            use_remote_dev_backend: waterui_path.is_none(),
            waterui_path,
            backend_project_path,
            android_permissions: Vec::new(),
        }
    }

    #[test]
    fn relative_waterui_path_produces_clean_relative_backend_path() {
        let ctx = ctx(
            Some(PathBuf::from("../..")),
            Some(PathBuf::from(".water/apple")),
        );

        let path = ctx
            .compute_relative_backend_path("apple")
            .expect("expected relative backend path");

        assert_eq!(path, "../../../../backends/apple");
        assert!(!path.contains("//"));
    }

    #[test]
    fn absolute_waterui_path_is_used_directly() {
        let abs = if cfg!(windows) {
            PathBuf::from(r"C:\waterui")
        } else {
            PathBuf::from("/waterui")
        };

        let ctx = ctx(Some(abs), Some(PathBuf::from("apple")));
        let path = ctx
            .compute_relative_backend_path("apple")
            .expect("expected backend path");

        let expected = if cfg!(windows) {
            "C:/waterui/backends/apple"
        } else {
            "/waterui/backends/apple"
        };
        assert_eq!(path, expected);
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
    use crate::android::toolchain::AndroidSdk;

    use super::{Path, TemplateContext, embedded, fs, io, normalize_path_for_config, scaffold_dir};

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

        // Generate local.properties with Android SDK path
        if let Some(sdk_path) = AndroidSdk::detect_path() {
            let local_props = base_dir.join("local.properties");
            let content = format!("sdk.dir={}\n", normalize_path_for_config(&sdk_path));
            fs::write(&local_props, content).await?;
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
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
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
