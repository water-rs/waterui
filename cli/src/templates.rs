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
const WATERUI_HYDROLYSIS_VERSION: &str = "0.1";

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
    pub static GTK4: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/gtk4");
    pub static HYDROLYSIS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/hydrolysis");
    pub static PREVIEW: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/preview");
    pub static INSPECTOR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/inspector");
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
    /// iOS permissions to include in Info.plist (e.g., "microphone", "camera")
    pub ios_permissions: Vec<(String, String)>,
    /// Whether to build as an accessory (headless) app on macOS.
    pub accessory: bool,
    /// Preview runtime fingerprint inserted into preview support app templates.
    pub preview_runtime_fingerprint: Option<String>,
    /// Package type of the project being scaffolded.
    pub package_type: crate::project::PackageType,
}

impl TemplateContext {
    /// Build a template context for a new root project scaffold.
    #[must_use]
    pub fn for_create_options(
        options: &crate::project::CreateOptions,
        crate_name: impl Into<String>,
    ) -> Self {
        let waterui_path = options.waterui_path.clone();
        Self {
            app_display_name: options.name.clone(),
            app_name: options.name.replace(' ', ""),
            crate_name: crate_name.into(),
            bundle_identifier: options.bundle_identifier.clone(),
            author: options.author.clone(),
            android_backend_path: waterui_path
                .as_ref()
                .map(|path| path.join("backends/android")),
            use_remote_dev_backend: waterui_path.is_none(),
            waterui_path,
            backend_project_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: false,
            preview_runtime_fingerprint: None,
            package_type: options.package_type,
        }
    }

    /// Build a context from an existing project manifest for backend scaffolding.
    #[must_use]
    pub fn for_project_manifest(
        manifest: &crate::project::Manifest,
        crate_name: impl Into<String>,
        app_name: impl Into<String>,
    ) -> Self {
        Self {
            app_display_name: manifest.package.name.clone(),
            app_name: app_name.into(),
            crate_name: crate_name.into(),
            bundle_identifier: manifest.package.bundle_identifier.clone(),
            author: String::new(),
            android_backend_path: None,
            use_remote_dev_backend: manifest.waterui_path.is_none(),
            waterui_path: manifest.waterui_path.as_ref().map(PathBuf::from),
            backend_project_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: manifest.package.accessory,
            preview_runtime_fingerprint: None,
            package_type: manifest.package.package_type,
        }
    }

    /// Build a context for support applications that always run as playground projects.
    #[must_use]
    pub fn for_support_playground(
        app_display_name: impl Into<String>,
        app_name: impl Into<String>,
        crate_name: impl Into<String>,
        bundle_identifier: impl Into<String>,
        waterui_path: PathBuf,
        accessory: bool,
        preview_runtime_fingerprint: Option<String>,
    ) -> Self {
        let android_backend_path = Some(waterui_path.join("backends/android"));
        Self {
            app_display_name: app_display_name.into(),
            app_name: app_name.into(),
            crate_name: crate_name.into(),
            bundle_identifier: bundle_identifier.into(),
            author: String::new(),
            android_backend_path,
            use_remote_dev_backend: false,
            waterui_path: Some(waterui_path),
            backend_project_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory,
            preview_runtime_fingerprint,
            package_type: crate::project::PackageType::Playground,
        }
    }

    /// Set backend project path for template rendering.
    #[must_use]
    pub fn with_backend_project_path(mut self, path: PathBuf) -> Self {
        self.backend_project_path = Some(path);
        self
    }

    /// Set optional Android backend path for template rendering.
    #[must_use]
    pub fn with_android_backend_path(mut self, path: Option<PathBuf>) -> Self {
        self.android_backend_path = path;
        self
    }

    /// Set Android permissions for template rendering.
    #[must_use]
    pub fn with_android_permissions(mut self, permissions: Vec<String>) -> Self {
        self.android_permissions = permissions;
        self
    }

    /// Set iOS permissions for template rendering.
    #[must_use]
    pub fn with_ios_permissions(mut self, permissions: Vec<(String, String)>) -> Self {
        self.ios_permissions = permissions;
        self
    }

    /// Render a template string by replacing all placeholders.
    #[must_use]
    pub fn render(&self, template: &str) -> String {
        // Android namespace must be a valid Java package name (no hyphens)
        let android_namespace = self.bundle_identifier.replace('-', "_");

        // Rust identifier form of crate name (hyphens -> underscores)
        let crate_name_ident = self.crate_name.replace('-', "_");
        let android_backend_path = if self.use_remote_dev_backend {
            String::new()
        } else {
            self.compute_android_backend_path().unwrap_or_else(|| {
                panic!(
                    "TemplateContext missing local Android backend path: \
use_remote_dev_backend=false requires waterui_path or android_backend_path"
                )
            })
        };

        template
            .replace("__APP_DISPLAY_NAME__", &self.app_display_name)
            .replace("__APP_NAME__", &self.app_name)
            .replace("__CRATE_NAME_IDENT__", &crate_name_ident)
            .replace("__CRATE_NAME__", &self.crate_name)
            .replace("__ANDROID_NAMESPACE__", &android_namespace)
            .replace("__BUNDLE_IDENTIFIER__", &self.bundle_identifier)
            .replace("__AUTHOR__", &self.author)
            .replace("__ANDROID_BACKEND_PATH__", &android_backend_path)
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
            .replace("__IOS_PERMISSION_KEYS__", &self.ios_permissions_plist())
            .replace("__ANDROID_PERMISSIONS__", &self.android_permissions_xml())
            .replace(
                "__PROJECT_ROOT_RELATIVE_PATH__",
                &self.project_root_relative_path(),
            )
            .replace(
                "__IS_ACCESSORY__",
                if self.accessory { "true" } else { "false" },
            )
            .replace(
                "__MACOS_LSUIELEMENT__",
                if self.accessory { "YES" } else { "NO" },
            )
            .replace(
                "__PREVIEW_RUNTIME_FINGERPRINT__",
                self.preview_runtime_fingerprint
                    .as_deref()
                    .unwrap_or_default(),
            )
            .replace(
                "__FFI_EXPORT__",
                if self.package_type == crate::project::PackageType::App {
                    "waterui_ffi::export!();"
                } else {
                    ""
                },
            )
            // Font entries are populated during packaging, not creation - use empty default
            .replace("__FONT_ENTRIES__", "")
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
        self.android_backend_path
            .as_ref()
            .map(|path| normalize_path_for_config(path))
            .or_else(|| self.compute_relative_backend_path("android"))
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

    /// Generate iOS Info.plist permission keys for Xcode build settings.
    fn ios_permissions_plist(&self) -> String {
        if self.ios_permissions.is_empty() {
            return String::new();
        }

        self.ios_permissions
            .iter()
            .filter_map(|(perm, desc)| {
                let plist_key = match perm.to_lowercase().as_str() {
                    "microphone" => "INFOPLIST_KEY_NSMicrophoneUsageDescription",
                    "camera" => "INFOPLIST_KEY_NSCameraUsageDescription",
                    "location" => "INFOPLIST_KEY_NSLocationWhenInUseUsageDescription",
                    "photo_library" => "INFOPLIST_KEY_NSPhotoLibraryUsageDescription",
                    "contacts" => "INFOPLIST_KEY_NSContactsUsageDescription",
                    "calendars" => "INFOPLIST_KEY_NSCalendarsUsageDescription",
                    "bluetooth" => "INFOPLIST_KEY_NSBluetoothAlwaysUsageDescription",
                    _ => return None, // Unknown permission, skip
                };
                // Escape double quotes in description
                let escaped_desc = desc.replace('"', "\\\"");
                Some(format!(
                    "                                {plist_key} = \"{escaped_desc}\";"
                ))
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        const REPO_URL: &str = "https://github.com/water-rs/apple-backend.git";
        const MIN_VERSION: &str = "0.2.0";

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
            ios_permissions: Vec::new(),
            accessory: false,
            preview_runtime_fingerprint: None,
            package_type: crate::project::PackageType::App,
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

#[derive(serde::Serialize)]
struct SupportCargoManifest {
    package: SupportPackageSection,
    lib: SupportLibSection,
    dependencies: std::collections::BTreeMap<String, SupportDependencyValue>,
    workspace: SupportWorkspaceSection,
}

#[derive(serde::Serialize)]
struct SupportPackageSection {
    name: String,
    version: String,
    edition: String,
}

#[derive(serde::Serialize)]
struct SupportLibSection {
    #[serde(rename = "crate-type")]
    crate_type: Vec<String>,
}

#[derive(serde::Serialize)]
struct SupportWorkspaceSection {}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum SupportDependencyValue {
    Simple(String),
    Detailed(SupportDependencyDetail),
}

#[derive(serde::Serialize)]
struct SupportDependencyDetail {
    path: String,
}

async fn write_support_cargo_toml(
    base_dir: &Path,
    crate_name: &str,
    dependencies: std::collections::BTreeMap<String, SupportDependencyValue>,
) -> io::Result<()> {
    let manifest = SupportCargoManifest {
        package: SupportPackageSection {
            name: crate_name.to_string(),
            version: "0.1.0".to_string(),
            edition: "2024".to_string(),
        },
        lib: SupportLibSection {
            crate_type: vec![
                "staticlib".to_string(),
                "cdylib".to_string(),
                "rlib".to_string(),
            ],
        },
        dependencies,
        workspace: SupportWorkspaceSection {},
    };

    let toml_string = toml::to_string_pretty(&manifest)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::create_dir_all(base_dir).await?;
    fs::write(base_dir.join("Cargo.toml"), toml_string).await?;
    Ok(())
}

#[derive(Clone, Copy)]
enum NativeBackendDependencyPathKind<'a> {
    WateruiRoot,
    BackendsSubdir(&'a str),
}

#[derive(Clone, Copy)]
struct NativeBackendDependencySpec<'a> {
    crate_name: &'a str,
    version: &'a str,
    features: &'a [&'a str],
    path_kind: Option<NativeBackendDependencyPathKind<'a>>,
}

impl<'a> NativeBackendDependencySpec<'a> {
    const fn new(
        crate_name: &'a str,
        version: &'a str,
        features: &'a [&'a str],
        path_kind: Option<NativeBackendDependencyPathKind<'a>>,
    ) -> Self {
        Self {
            crate_name,
            version,
            features,
            path_kind,
        }
    }
}

fn compute_native_backend_dependency_path(
    ctx: &TemplateContext,
    waterui_path: &Path,
    path_kind: NativeBackendDependencyPathKind<'_>,
) -> String {
    if waterui_path.is_absolute() {
        let absolute_path = match path_kind {
            NativeBackendDependencyPathKind::WateruiRoot => waterui_path.to_path_buf(),
            NativeBackendDependencyPathKind::BackendsSubdir(subdir) => {
                waterui_path.join("backends").join(subdir)
            }
        };
        return normalize_path_for_config(&absolute_path);
    }

    let project_relative_root = PathBuf::from(ctx.project_root_relative_path());
    let relative_path = match path_kind {
        NativeBackendDependencyPathKind::WateruiRoot => project_relative_root.join(waterui_path),
        NativeBackendDependencyPathKind::BackendsSubdir(subdir) => project_relative_root
            .join(waterui_path)
            .join("backends")
            .join(subdir),
    };
    normalize_path_for_config(&relative_path)
}

async fn write_native_backend_bin_cargo_toml(
    base_dir: &Path,
    ctx: &TemplateContext,
    package_name: &str,
    dependencies: &[NativeBackendDependencySpec<'_>],
) -> io::Result<()> {
    use cargo_toml::{Dependency, DependencyDetail, Manifest, Package, Workspace};

    let mut manifest = Manifest::<()>::default();
    manifest.package = Some(Package::new(package_name.to_string(), "0.1.0".to_string()));
    if let Some(ref mut package) = manifest.package {
        package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
    }

    manifest.dependencies.insert(
        ctx.crate_name.clone(),
        Dependency::Detailed(Box::new(DependencyDetail {
            path: Some(ctx.project_root_relative_path()),
            ..Default::default()
        })),
    );

    for dependency in dependencies {
        let features = dependency
            .features
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>();

        if let Some(waterui_path) = &ctx.waterui_path
            && let Some(path_kind) = dependency.path_kind
        {
            let dependency_path =
                compute_native_backend_dependency_path(ctx, waterui_path, path_kind);
            manifest.dependencies.insert(
                dependency.crate_name.to_string(),
                Dependency::Detailed(Box::new(DependencyDetail {
                    path: Some(dependency_path),
                    features,
                    ..Default::default()
                })),
            );
            continue;
        }

        manifest.dependencies.insert(
            dependency.crate_name.to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                version: Some(dependency.version.to_string()),
                features,
                ..Default::default()
            })),
        );
    }

    manifest.workspace = Some(Workspace::default());

    let toml_string = toml::to_string_pretty(&manifest)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::create_dir_all(base_dir).await?;
    fs::write(base_dir.join("Cargo.toml"), toml_string).await?;
    Ok(())
}

fn dependency_path(path: &Path) -> SupportDependencyValue {
    SupportDependencyValue::Detailed(SupportDependencyDetail {
        path: normalize_path_for_config(path),
    })
}

fn dependency_version(version: &str) -> SupportDependencyValue {
    SupportDependencyValue::Simple(version.to_string())
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

/// GTK4 backend templates.
pub mod gtk4 {
    use super::{
        NativeBackendDependencyPathKind, NativeBackendDependencySpec, Path, TemplateContext,
        embedded, io, scaffold_dir, write_native_backend_bin_cargo_toml,
    };

    const WATERUI_GTK_VERSION: &str = "0.1";

    /// Write all GTK4 templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        // Generate Cargo.toml programmatically
        generate_cargo_toml(base_dir, ctx, package_name).await?;

        // Scaffold remaining template files (main.rs, etc.)
        scaffold_dir(&embedded::GTK4, base_dir, ctx).await
    }

    /// Generate GTK4 Cargo.toml programmatically using cargo_toml crate.
    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let dependencies = [NativeBackendDependencySpec::new(
            "waterui-gtk",
            WATERUI_GTK_VERSION,
            &[],
            Some(NativeBackendDependencyPathKind::BackendsSubdir("gtk")),
        )];
        write_native_backend_bin_cargo_toml(base_dir, ctx, package_name, &dependencies).await
    }
}

/// Hydrolysis backend templates.
pub mod hydrolysis {
    use super::{
        NativeBackendDependencyPathKind, NativeBackendDependencySpec, Path, TemplateContext,
        WATERUI_HYDROLYSIS_VERSION, WATERUI_VERSION, embedded, io, scaffold_dir,
        write_native_backend_bin_cargo_toml,
    };

    /// Write all hydrolysis templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        generate_cargo_toml(base_dir, ctx, package_name).await?;
        scaffold_dir(&embedded::HYDROLYSIS, base_dir, ctx).await
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let dependencies = [
            NativeBackendDependencySpec::new(
                "hydrolysis",
                WATERUI_HYDROLYSIS_VERSION,
                &["winit"],
                Some(NativeBackendDependencyPathKind::BackendsSubdir(
                    "hydrolysis",
                )),
            ),
            NativeBackendDependencySpec::new(
                "waterui",
                WATERUI_VERSION,
                &[],
                Some(NativeBackendDependencyPathKind::WateruiRoot),
            ),
        ];
        write_native_backend_bin_cargo_toml(base_dir, ctx, package_name, &dependencies).await
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
            dependencies.insert(
                "waterui".to_string(),
                DependencyValue::Detailed(DependencyDetail {
                    path: normalize_path_for_config(waterui_path),
                }),
            );
            if ctx.package_type == crate::project::PackageType::App {
                let ffi_path = waterui_path.join("ffi");
                dependencies.insert(
                    "waterui-ffi".to_string(),
                    DependencyValue::Detailed(DependencyDetail {
                        path: normalize_path_for_config(&ffi_path),
                    }),
                );
            }
        } else {
            dependencies.insert(
                "waterui".to_string(),
                DependencyValue::Simple(WATERUI_VERSION.to_string()),
            );
            if ctx.package_type == crate::project::PackageType::App {
                dependencies.insert(
                    "waterui-ffi".to_string(),
                    DependencyValue::Simple(WATERUI_FFI_VERSION.to_string()),
                );
            }
        }

        let manifest = CargoManifest {
            package: PackageSection {
                name: ctx.crate_name.clone(),
                version: "0.1.0".to_string(),
                edition: "2024".to_string(),
                authors: vec![ctx.author.clone()],
            },
            lib: LibSection {
                crate_type: if ctx.package_type == crate::project::PackageType::App {
                    vec![
                        "staticlib".to_string(),
                        "cdylib".to_string(),
                        "rlib".to_string(),
                    ]
                } else {
                    vec!["lib".to_string()]
                },
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

/// Preview app templates.
pub mod preview {
    use crate::templates::{WATERUI_FFI_VERSION, WATERUI_VERSION};

    use super::{
        Path, TemplateContext, dependency_path, dependency_version, embedded, io, scaffold_dir,
        write_support_cargo_toml,
    };

    const WATERUI_PREVIEW_VERSION: &str = "0.1";

    /// Hash of embedded preview template files.
    #[must_use]
    pub fn template_fingerprint() -> String {
        use sha2::Digest as _;

        let mut hasher = sha2::Sha256::new();
        let mut dirs_to_process = vec![&embedded::PREVIEW];
        while let Some(current_dir) = dirs_to_process.pop() {
            for file in current_dir.files() {
                hasher.update(file.path().to_string_lossy().as_bytes());
                hasher.update(file.contents());
            }
            for subdir in current_dir.dirs() {
                dirs_to_process.push(subdir);
            }
        }
        format!("{:x}", hasher.finalize())
    }

    /// Write preview app templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        // Generate Cargo.toml programmatically
        generate_cargo_toml(base_dir, ctx).await?;

        // Scaffold remaining template files (lib.rs)
        scaffold_dir(&embedded::PREVIEW, base_dir, ctx).await
    }

    /// Generate preview app Cargo.toml programmatically.
    async fn generate_cargo_toml(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        use std::collections::BTreeMap;

        let mut dependencies = BTreeMap::new();

        if let Some(waterui_path) = &ctx.waterui_path {
            // Local path dependencies
            dependencies.insert("waterui".to_string(), dependency_path(waterui_path));

            let ffi_path = waterui_path.join("ffi");
            dependencies.insert("waterui-ffi".to_string(), dependency_path(&ffi_path));

            let preview_path = waterui_path.join("components").join("preview");
            dependencies.insert(
                "waterui-preview".to_string(),
                dependency_path(&preview_path),
            );
        } else {
            // Registry dependencies
            dependencies.insert("waterui".to_string(), dependency_version(WATERUI_VERSION));
            dependencies.insert(
                "waterui-ffi".to_string(),
                dependency_version(WATERUI_FFI_VERSION),
            );
            dependencies.insert(
                "waterui-preview".to_string(),
                dependency_version(WATERUI_PREVIEW_VERSION),
            );
        }
        write_support_cargo_toml(base_dir, &ctx.crate_name, dependencies).await
    }
}

/// Inspector app templates.
pub mod inspector {
    use super::{
        Path, TemplateContext, dependency_path, dependency_version, embedded, io, scaffold_dir,
        write_support_cargo_toml,
    };

    /// Hash of embedded inspector template files.
    #[must_use]
    pub fn template_fingerprint() -> String {
        use sha2::Digest as _;

        let mut hasher = sha2::Sha256::new();
        let mut dirs_to_process = vec![&embedded::INSPECTOR];
        while let Some(current_dir) = dirs_to_process.pop() {
            for file in current_dir.files() {
                hasher.update(file.path().to_string_lossy().as_bytes());
                hasher.update(file.contents());
            }
            for subdir in current_dir.dirs() {
                dirs_to_process.push(subdir);
            }
        }
        format!("{:x}", hasher.finalize())
    }

    /// Write inspector app templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        generate_cargo_toml(base_dir, ctx).await?;
        scaffold_dir(&embedded::INSPECTOR, base_dir, ctx).await
    }

    async fn generate_cargo_toml(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        use std::collections::BTreeMap;

        let waterui_path = ctx.waterui_path.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Inspector support app requires a local waterui_path",
            )
        })?;

        let inspector_protocol_path = waterui_path.join("components/inspector-protocol");
        if !inspector_protocol_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "Inspector support app requires {} (missing components/inspector-protocol)",
                    waterui_path.display()
                ),
            ));
        }

        let mut dependencies = BTreeMap::new();
        dependencies.insert("waterui".to_string(), dependency_path(waterui_path));
        dependencies.insert(
            "waterui-ffi".to_string(),
            dependency_path(&waterui_path.join("ffi")),
        );
        dependencies.insert(
            "waterui-inspector-protocol".to_string(),
            dependency_path(&inspector_protocol_path),
        );
        dependencies.insert("smol".to_string(), dependency_version("2.0.2"));
        dependencies.insert("futures-lite".to_string(), dependency_version("2.6"));

        write_support_cargo_toml(base_dir, &ctx.crate_name, dependencies).await
    }
}
