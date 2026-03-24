//! Type-safe template scaffolding for `WaterUI` project backends.
//!
//! Uses `include_dir` to embed templates at compile time and provides
//! a type-safe substitution API for generating Apple and Android backend projects.

use std::{
    io,
    path::{Path, PathBuf},
};

use askama::Template;

const WATERUI_VERSION: &str = "0.2";
const WATERUI_FFI_VERSION: &str = "0.2";
const WATERUI_HYDROLYSIS_VERSION: &str = "0.1";

use include_dir::{Dir, include_dir};
use smol::fs;

use crate::project_types::{BundleIdentifier, CrateName, RustIdent};

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
    pub static FFI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/ffi");
    pub static GTK4: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/gtk4");
    pub static HYDROLYSIS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/hydrolysis");
    pub static PREVIEW: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/preview");
    pub static INSPECTOR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/inspector");
    pub static ROOT: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates");
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidPermissionTemplateEntry {
    pub name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosPermissionTemplateEntry {
    pub plist_key: &'static str,
    pub description: String,
}

impl IosPermissionTemplateEntry {
    #[must_use]
    pub fn escaped_description(&self) -> String {
        self.description.replace('"', "\\\"")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontRegistrationTemplateEntry {
    pub family_name: String,
    pub file_name: String,
}

/// Context for rendering templates with type-safe substitutions.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// The application display name (e.g., "My App")
    pub app_display_name: String,
    /// The application name for file/folder naming (e.g., "`MyApp`")
    pub app_name: String,
    /// The Rust crate name (e.g., "`my_app`")
    pub crate_name: CrateName,
    /// The bundle identifier (e.g., "com.example.myapp")
    pub bundle_identifier: BundleIdentifier,
    /// The author name
    pub author: String,
    /// Path to the Android backend (relative or absolute)
    pub android_backend_path: Option<PathBuf>,
    /// Whether to use remote dev backend (`JitPack`) instead of local
    pub use_remote_dev_backend: bool,
    /// Path to local `WaterUI` repository (for dev mode)
    pub waterui_path: Option<PathBuf>,
    /// Path to the backend project being scaffolded.
    ///
    /// This may be relative to the project root or an absolute cache path.
    pub backend_project_path: Option<PathBuf>,
    /// Absolute path to the user project root when scaffolding generated backend projects.
    pub project_root_path: Option<PathBuf>,
    /// Android permissions to include in the manifest (e.g., "internet", "camera")
    pub android_permissions: Vec<AndroidPermissionTemplateEntry>,
    /// iOS permissions to include in Info.plist (e.g., "microphone", "camera")
    pub ios_permissions: Vec<IosPermissionTemplateEntry>,
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
        crate_name: CrateName,
    ) -> Self {
        let waterui_path = options.waterui_path.clone();
        Self {
            app_display_name: options.name.clone(),
            app_name: options.name.replace(' ', ""),
            crate_name,
            bundle_identifier: options.bundle_identifier.clone(),
            author: options.author.clone(),
            android_backend_path: waterui_path
                .as_ref()
                .map(|path| path.join("backends/android")),
            use_remote_dev_backend: waterui_path.is_none(),
            waterui_path,
            backend_project_path: None,
            project_root_path: None,
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
        crate_name: CrateName,
        app_name: impl Into<String>,
    ) -> Self {
        Self {
            app_display_name: manifest.package.name.clone(),
            app_name: app_name.into(),
            crate_name,
            bundle_identifier: manifest.package.bundle_identifier.clone(),
            author: String::new(),
            android_backend_path: None,
            use_remote_dev_backend: manifest.waterui_path.is_none(),
            waterui_path: manifest.waterui_path.as_ref().map(PathBuf::from),
            backend_project_path: None,
            project_root_path: None,
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
        crate_name: CrateName,
        bundle_identifier: BundleIdentifier,
        waterui_path: PathBuf,
        accessory: bool,
        preview_runtime_fingerprint: Option<String>,
    ) -> Self {
        let android_backend_path = Some(waterui_path.join("backends/android"));
        Self {
            app_display_name: app_display_name.into(),
            app_name: app_name.into(),
            crate_name,
            bundle_identifier,
            author: String::new(),
            android_backend_path,
            use_remote_dev_backend: false,
            waterui_path: Some(waterui_path),
            backend_project_path: None,
            project_root_path: None,
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

    /// Set absolute project root path for template rendering.
    #[must_use]
    pub fn with_project_root_path(mut self, path: PathBuf) -> Self {
        self.project_root_path = Some(path);
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
    pub fn with_android_permissions(mut self, permissions: Vec<AndroidPermissionTemplateEntry>) -> Self {
        self.android_permissions = permissions;
        self
    }

    /// Set iOS permissions for template rendering.
    #[must_use]
    pub fn with_ios_permissions(mut self, permissions: Vec<IosPermissionTemplateEntry>) -> Self {
        self.ios_permissions = permissions;
        self
    }

    #[must_use]
    pub fn crate_name_ident(&self) -> RustIdent {
        self.crate_name.rust_ident()
    }

    #[must_use]
    pub fn android_package_name(&self) -> String {
        self.bundle_identifier
            .android_package_name()
            .unwrap_or_else(|error| panic!("{error}"))
            .to_string()
    }

    #[must_use]
    pub fn android_backend_path(&self) -> String {
        if self.use_remote_dev_backend {
            return String::new();
        }

        self.compute_android_backend_path().unwrap_or_else(|| {
            panic!(
                "TemplateContext missing local Android backend path: \
use_remote_dev_backend=false requires waterui_path or android_backend_path"
            )
        })
    }

    #[must_use]
    pub fn is_playground(&self) -> bool {
        self.package_type == crate::project::PackageType::Playground
    }

    #[must_use]
    pub const fn macos_lsuielement(&self) -> &'static str {
        if self.accessory { "YES" } else { "NO" }
    }

    #[must_use]
    pub fn preview_runtime_fingerprint(&self) -> &str {
        self.preview_runtime_fingerprint
            .as_deref()
            .unwrap_or_default()
    }

    /// Transform a path by replacing "`AppName`" with the actual app name.
    #[must_use]
    pub fn transform_path(&self, path: &Path) -> PathBuf {
        let path_str = path.to_string_lossy();
        PathBuf::from(path_str.replace("AppName", &self.app_name))
    }

    /// Compute the relative path from the backend project to a `WaterUI` backend.
    ///
    /// This accounts for the project being in a generated backend subdirectory.
    fn compute_relative_backend_path(&self, backend_subdir: &str) -> Option<String> {
        let waterui_path = self.waterui_path.as_ref()?;

        // If `waterui_path` is absolute, use it directly. This avoids producing invalid
        // paths like `../../../..//Users/...` in generated config files.
        if waterui_path.is_absolute() {
            let absolute_backend_path = waterui_path.join("backends").join(backend_subdir);
            return Some(normalize_path_for_config(&absolute_backend_path));
        }

        if let Some(backend_project_path) = self
            .backend_project_path
            .as_ref()
            .filter(|path| path.is_absolute())
        {
            let project_root = self.project_root_path.as_ref().unwrap_or_else(|| {
                panic!(
                    "TemplateContext missing project_root_path for absolute backend project {}",
                    backend_project_path.display()
                )
            });
            let absolute_backend_path = project_root
                .join(waterui_path)
                .join("backends")
                .join(backend_subdir);
            let relative_path = pathdiff::diff_paths(&absolute_backend_path, backend_project_path)
                .unwrap_or_else(|| {
                    panic!(
                        "Failed to compute backend dependency path from {} to {}",
                        backend_project_path.display(),
                        absolute_backend_path.display()
                    )
                });
            return Some(normalize_path_for_config(&relative_path));
        }

        // Count how many levels deep the project is from the project root.
        // Default is 1 level (e.g., "android"), generated playground backends may be deeper.
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
    /// For a backend at `managed_backends/apple/`, returns `../..` (go up 2 levels).
    fn project_root_relative_path(&self) -> String {
        if let Some(backend_project_path) = self
            .backend_project_path
            .as_ref()
            .filter(|path| path.is_absolute())
        {
            let project_root = self.project_root_path.as_ref().unwrap_or_else(|| {
                panic!(
                    "TemplateContext missing project_root_path for absolute backend project {}",
                    backend_project_path.display()
                )
            });
            let relative_path = pathdiff::diff_paths(project_root, backend_project_path)
                .unwrap_or_else(|| {
                    panic!(
                        "Failed to compute project root path from {} to {}",
                        backend_project_path.display(),
                        project_root.display()
                    )
                });
            return normalize_path_for_config(&relative_path);
        }

        let depth = self
            .backend_project_path
            .as_ref()
            .map_or(1, |p| p.components().count());

        (0..depth).map(|_| "..").collect::<Vec<_>>().join("/")
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateNamespace {
    Apple,
    Android,
    Ffi,
    Gtk4,
    Hydrolysis,
    Inspector,
    Preview,
    Root,
}

impl TemplateNamespace {
    const fn scaffold_template_prefix(self) -> &'static str {
        match self {
            Self::Apple => "src/templates/apple",
            Self::Android => "src/templates/android",
            Self::Ffi => "src/templates/ffi",
            Self::Gtk4 => "src/templates/gtk4",
            Self::Hydrolysis => "src/templates/hydrolysis",
            Self::Inspector => "src/templates/inspector",
            Self::Preview => "src/templates/preview",
            Self::Root => "src/templates",
        }
    }
}

fn scaffold_template_dispatch_path(namespace: TemplateNamespace, relative_path: &Path) -> String {
    let relative_path = normalize_path_for_config(relative_path);
    if relative_path.starts_with("src/templates/") {
        return relative_path;
    }
    format!(
        "{}/{relative_path}",
        namespace.scaffold_template_prefix()
    )
}

macro_rules! define_scaffold_templates {
    ($($name:ident => ($namespace:ident, $path:literal)),* $(,)?) => {
        $(
            #[derive(Template)]
            #[template(path = $path, escape = "none")]
            struct $name<'a> {
                ctx: &'a TemplateContext,
            }
        )*

        fn render_scaffold_template(
            namespace: TemplateNamespace,
            relative_path: &Path,
            content: &str,
            ctx: &TemplateContext,
        ) -> io::Result<String> {
            let display_path = relative_path.to_string_lossy();
            let dispatch_path = scaffold_template_dispatch_path(namespace, relative_path);
            match dispatch_path.as_str() {
                "src/templates/apple/AppName/WaterUIFonts.swift.tpl" => {
                    let empty_font_entries: &[FontRegistrationTemplateEntry] = &[];
                    ScaffoldAppleFontTemplate {
                        font_entries: empty_font_entries,
                    }
                    .render()
                    .map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Failed to render template {display_path}: {error}"),
                        )
                    })
                }
                $(
                    $path => $name { ctx }
                        .render()
                        .map_err(|error| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("Failed to render template {display_path}: {error}"),
                            )
                        }),
                )*
                _ => Ok(content.to_string()),
            }
        }
    };
}

#[derive(Template)]
#[template(path = "src/templates/apple/AppName/WaterUIFonts.swift.tpl", escape = "none")]
struct ScaffoldAppleFontTemplate<'a> {
    font_entries: &'a [FontRegistrationTemplateEntry],
}

define_scaffold_templates! {
    AppleProjectTemplate => (Apple, "src/templates/apple/AppName.xcodeproj/project.pbxproj.tpl"),
    AppleAppTemplate => (Apple, "src/templates/apple/AppName/AppNameApp.swift.tpl"),
    AppleBuildScriptTemplate => (Apple, "src/templates/apple/build-rust.sh.tpl"),
    AndroidGradleAppTemplate => (Android, "src/templates/android/app/build.gradle.kts.tpl"),
    AndroidManifestTemplate => (Android, "src/templates/android/app/src/main/AndroidManifest.xml.tpl"),
    AndroidMainActivityTemplate => (Android, "src/templates/android/app/src/main/java/MainActivity.kt.tpl"),
    AndroidStringsTemplate => (Android, "src/templates/android/app/src/main/res/values/strings.xml.tpl"),
    AndroidSettingsTemplate => (Android, "src/templates/android/settings.gradle.kts.tpl"),
    FfiLibTemplate => (Ffi, "src/templates/ffi/src/lib.rs.tpl"),
    Gtk4MainTemplate => (Gtk4, "src/templates/gtk4/src/main.rs.tpl"),
    HydrolysisLibTemplate => (Hydrolysis, "src/templates/hydrolysis/src/lib.rs.tpl"),
    HydrolysisMainTemplate => (Hydrolysis, "src/templates/hydrolysis/src/main.rs.tpl"),
    HydrolysisPreviewRuntimeTemplate => (Hydrolysis, "src/templates/hydrolysis/src/preview_runtime.rs.tpl"),
    HydrolysisWebIndexTemplate => (Hydrolysis, "src/templates/hydrolysis/web/index.html.tpl"),
    PreviewLibTemplate => (Preview, "src/templates/preview/src/lib.rs.tpl"),
}

#[cfg(test)]
mod tests {
    use super::{
        TemplateContext, TemplateNamespace, embedded, normalize_path_for_config,
        render_scaffold_template,
    };
    use crate::project_types::{BundleIdentifier, CrateName};
    use std::path::PathBuf;

    fn ctx(
        waterui_path: Option<PathBuf>,
        backend_project_path: Option<PathBuf>,
        project_root_path: Option<PathBuf>,
        package_type: crate::project::PackageType,
    ) -> TemplateContext {
        TemplateContext {
            app_display_name: String::new(),
            app_name: String::new(),
            crate_name: CrateName::try_from("waterui_test").expect("test crate name must be valid"),
            bundle_identifier: BundleIdentifier::try_from("com.example.test")
                .expect("test bundle identifier must be valid"),
            author: String::new(),
            android_backend_path: None,
            use_remote_dev_backend: waterui_path.is_none(),
            waterui_path,
            backend_project_path,
            project_root_path,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: false,
            preview_runtime_fingerprint: None,
            package_type,
        }
    }

    fn app_ctx() -> TemplateContext {
        ctx(None, None, None, crate::project::PackageType::App)
    }

    fn playground_ctx() -> TemplateContext {
        TemplateContext::for_support_playground(
            "WaterUIApp",
            "WaterUIApp",
            CrateName::try_from("waterui_app").expect("test crate name must be valid"),
            BundleIdentifier::try_from("dev.waterui.playground")
                .expect("test bundle identifier must be valid"),
            PathBuf::from("../.."),
            false,
            None,
        )
        .with_backend_project_path(PathBuf::from("managed_backends/apple"))
    }

    #[test]
    fn relative_waterui_path_produces_clean_relative_backend_path() {
        let ctx = ctx(
            Some(PathBuf::from("../..")),
            Some(PathBuf::from("managed_backends/apple")),
            None,
            crate::project::PackageType::App,
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

        let ctx = ctx(
            Some(abs),
            Some(PathBuf::from("apple")),
            None,
            crate::project::PackageType::App,
        );
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

    #[test]
    fn absolute_backend_project_path_uses_real_project_root() {
        let project_root = if cfg!(windows) {
            PathBuf::from(r"C:\Users\lexo\demo")
        } else {
            PathBuf::from("/Users/lexo/demo")
        };
        let backend_project_path = if cfg!(windows) {
            PathBuf::from(
                r"C:\Users\lexo\.water\build_cache\drive-C\Users\lexo\demo\managed_backends\apple",
            )
        } else {
            PathBuf::from("/Users/lexo/.water/build_cache/Users/lexo/demo/managed_backends/apple")
        };

        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(backend_project_path.clone()),
            Some(project_root.clone()),
            crate::project::PackageType::Playground,
        );

        let path = ctx
            .compute_relative_backend_path("apple")
            .expect("expected backend path");
        let expected_backend_path = pathdiff::diff_paths(
            project_root.join("../waterui").join("backends/apple"),
            &backend_project_path,
        )
        .expect("backend diff path");
        assert_eq!(path, normalize_path_for_config(&expected_backend_path));

        let expected_project_root =
            pathdiff::diff_paths(&project_root, &backend_project_path).expect("project root diff");
        assert_eq!(
            ctx.project_root_relative_path(),
            normalize_path_for_config(&expected_project_root)
        );
    }

    #[test]
    fn android_manifest_enables_picture_in_picture_by_default() {
        let ctx = app_ctx();
        let template = embedded::ANDROID
            .get_file("app/src/main/AndroidManifest.xml.tpl")
            .expect("android manifest template must exist")
            .contents_utf8()
            .expect("android manifest template must be utf-8");

        let rendered = render_scaffold_template(
            TemplateNamespace::Android,
            std::path::Path::new("app/src/main/AndroidManifest.xml.tpl"),
            template,
            &ctx,
        )
        .expect("android manifest render");

        assert!(rendered.contains("android:resizeableActivity=\"true\""));
        assert!(rendered.contains("android:supportsPictureInPicture=\"true\""));
        assert!(rendered.contains(
            "android:configChanges=\"screenSize|smallestScreenSize|screenLayout|orientation\""
        ));
    }

    #[test]
    fn apple_project_enables_picture_in_picture_background_mode_by_default() {
        let ctx = app_ctx();
        let template = embedded::APPLE
            .get_file("AppName.xcodeproj/project.pbxproj.tpl")
            .expect("apple project template must exist")
            .contents_utf8()
            .expect("apple project template must be utf-8");

        let rendered = render_scaffold_template(
            TemplateNamespace::Apple,
            std::path::Path::new("AppName.xcodeproj/project.pbxproj.tpl"),
            template,
            &ctx,
        )
        .expect("apple project render");

        assert!(
            rendered.contains("\"INFOPLIST_KEY_UIBackgroundModes[sdk=iphoneos*][0]\" = audio;")
        );
        assert!(
            rendered
                .contains("\"INFOPLIST_KEY_UIBackgroundModes[sdk=iphonesimulator*][0]\" = audio;")
        );
    }

    #[test]
    fn android_main_activity_forwards_user_leave_hint_to_runtime_callback() {
        let ctx = app_ctx();
        let template = embedded::ANDROID
            .get_file("app/src/main/java/MainActivity.kt.tpl")
            .expect("android main activity template must exist")
            .contents_utf8()
            .expect("android main activity template must be utf-8");

        let rendered = render_scaffold_template(
            TemplateNamespace::Android,
            std::path::Path::new("app/src/main/java/MainActivity.kt.tpl"),
            template,
            &ctx,
        )
        .expect("android activity render");

        assert!(rendered.contains(
            "import dev.waterui.android.runtime.notifyVideoPictureInPictureUserLeaveHint"
        ));
        assert!(rendered.contains("override fun onUserLeaveHint()"));
        assert!(rendered.contains("notifyVideoPictureInPictureUserLeaveHint(this)"));
    }

    #[test]
    fn playground_android_manifest_enables_picture_in_picture_by_default() {
        let ctx = playground_ctx();
        let template = embedded::ANDROID
            .get_file("app/src/main/AndroidManifest.xml.tpl")
            .expect("android manifest template must exist")
            .contents_utf8()
            .expect("android manifest template must be utf-8");

        let rendered = render_scaffold_template(
            TemplateNamespace::Android,
            std::path::Path::new("app/src/main/AndroidManifest.xml.tpl"),
            template,
            &ctx,
        )
        .expect("playground android manifest render");

        assert!(rendered.contains("android:resizeableActivity=\"true\""));
        assert!(rendered.contains("android:supportsPictureInPicture=\"true\""));
    }

    #[test]
    fn playground_apple_project_enables_picture_in_picture_background_mode_by_default() {
        let ctx = playground_ctx();
        let template = embedded::APPLE
            .get_file("AppName.xcodeproj/project.pbxproj.tpl")
            .expect("apple project template must exist")
            .contents_utf8()
            .expect("apple project template must be utf-8");

        let rendered = render_scaffold_template(
            TemplateNamespace::Apple,
            std::path::Path::new("AppName.xcodeproj/project.pbxproj.tpl"),
            template,
            &ctx,
        )
        .expect("playground apple project render");

        assert!(
            rendered.contains("\"INFOPLIST_KEY_UIBackgroundModes[sdk=iphoneos*][0]\" = audio;")
        );
        assert!(
            rendered
                .contains("\"INFOPLIST_KEY_UIBackgroundModes[sdk=iphonesimulator*][0]\" = audio;")
        );
    }

    #[test]
    fn playground_apple_build_script_skips_direct_rust_build() {
        let ctx = playground_ctx();
        let template = embedded::APPLE
            .get_file("build-rust.sh.tpl")
            .expect("apple build script template must exist")
            .contents_utf8()
            .expect("apple build script template must be utf-8");

        let rendered = render_scaffold_template(
            TemplateNamespace::Apple,
            std::path::Path::new("build-rust.sh.tpl"),
            template,
            &ctx,
        )
        .expect("playground apple build script render");

        assert!(rendered.contains("playground support app is managed by water run/package"));
        assert!(rendered.contains("if [ \"true\" = \"true\" ]; then"));
    }
}

/// Scaffold a directory from embedded templates (non-recursive, uses stack).
async fn scaffold_dir(
    namespace: TemplateNamespace,
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
                let rendered = render_scaffold_template(namespace, relative_path, content, ctx)?;
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
    WorkspaceSubdir(&'a str),
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
            NativeBackendDependencyPathKind::WorkspaceSubdir(subdir) => waterui_path.join(subdir),
            NativeBackendDependencyPathKind::BackendsSubdir(subdir) => {
                waterui_path.join("backends").join(subdir)
            }
        };
        return normalize_path_for_config(&absolute_path);
    }

    let project_relative_root = PathBuf::from(ctx.project_root_relative_path());
    let relative_path = match path_kind {
        NativeBackendDependencyPathKind::WateruiRoot => project_relative_root.join(waterui_path),
        NativeBackendDependencyPathKind::WorkspaceSubdir(subdir) => {
            project_relative_root.join(waterui_path).join(subdir)
        }
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
        ctx.crate_name.to_string(),
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
    use super::{Path, TemplateContext, TemplateNamespace, embedded, fs, io, scaffold_dir};

    /// Write all Apple templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(TemplateNamespace::Apple, &embedded::APPLE, base_dir, ctx).await?;

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

    use super::{
        Path, TemplateContext, TemplateNamespace, embedded, fs, io, normalize_path_for_config,
        scaffold_dir,
    };

    /// Write all Android templates to the given directory.
    ///
    /// # Errors
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(TemplateNamespace::Android, &embedded::ANDROID, base_dir, ctx).await?;

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
        TemplateNamespace, embedded, io, scaffold_dir, write_native_backend_bin_cargo_toml,
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
        scaffold_dir(TemplateNamespace::Gtk4, &embedded::GTK4, base_dir, ctx).await
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
        TemplateNamespace, WATERUI_HYDROLYSIS_VERSION, WATERUI_VERSION,
        compute_native_backend_dependency_path, embedded, fs, io, scaffold_dir,
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
        scaffold_dir(TemplateNamespace::Hydrolysis, &embedded::HYDROLYSIS, base_dir, ctx).await
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        use serde::Serialize;
        use std::collections::BTreeMap;

        #[derive(Serialize)]
        struct CargoManifest {
            package: PackageSection,
            lib: LibSection,
            features: BTreeMap<String, Vec<String>>,
            dependencies: BTreeMap<String, DependencyValue>,
            target: BTreeMap<String, TargetSection>,
            workspace: WorkspaceSection,
        }

        #[derive(Serialize)]
        struct PackageSection {
            name: String,
            version: String,
            edition: String,
        }

        #[derive(Serialize)]
        struct LibSection {
            #[serde(rename = "crate-type")]
            crate_type: Vec<String>,
        }

        #[derive(Serialize)]
        struct TargetSection {
            dependencies: BTreeMap<String, DependencyValue>,
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
            #[serde(skip_serializing_if = "Option::is_none")]
            version: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            path: Option<String>,
            #[serde(rename = "default-features")]
            default_features: bool,
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            features: Vec<String>,
        }

        fn dependency_from_spec(
            ctx: &TemplateContext,
            spec: NativeBackendDependencySpec<'_>,
        ) -> DependencyValue {
            let features = spec
                .features
                .iter()
                .map(|item| item.to_string())
                .collect::<Vec<_>>();

            if let Some(waterui_path) = &ctx.waterui_path
                && let Some(path_kind) = spec.path_kind
            {
                return DependencyValue::Detailed(DependencyDetail {
                    version: None,
                    path: Some(compute_native_backend_dependency_path(
                        ctx,
                        waterui_path,
                        path_kind,
                    )),
                    default_features: false,
                    features,
                });
            }

            DependencyValue::Detailed(DependencyDetail {
                version: Some(spec.version.to_string()),
                path: None,
                default_features: false,
                features,
            })
        }

        let mut dependencies = BTreeMap::new();
        dependencies.insert(
            ctx.crate_name.to_string(),
            DependencyValue::Detailed(DependencyDetail {
                version: None,
                path: Some(ctx.project_root_relative_path()),
                default_features: false,
                features: Vec::new(),
            }),
        );
        dependencies.insert(
            "waterui".to_string(),
            match dependency_from_spec(
                ctx,
                NativeBackendDependencySpec::new(
                    "waterui",
                    WATERUI_VERSION,
                    &[],
                    Some(NativeBackendDependencyPathKind::WateruiRoot),
                ),
            ) {
                DependencyValue::Simple(version) => DependencyValue::Detailed(DependencyDetail {
                    version: Some(version),
                    path: None,
                    default_features: false,
                    features: Vec::new(),
                }),
                DependencyValue::Detailed(mut detail) => {
                    detail.default_features = false;
                    DependencyValue::Detailed(detail)
                }
            },
        );

        let mut native_dependencies = BTreeMap::new();
        native_dependencies.insert(
            "hydrolysis".to_string(),
            dependency_from_spec(
                ctx,
                NativeBackendDependencySpec::new(
                    "hydrolysis",
                    WATERUI_HYDROLYSIS_VERSION,
                    &["winit"],
                    Some(NativeBackendDependencyPathKind::BackendsSubdir(
                        "hydrolysis",
                    )),
                ),
            ),
        );
        native_dependencies.insert(
            "pollster".to_string(),
            DependencyValue::Simple("0.4".to_string()),
        );
        native_dependencies.insert(
            "waterui-preview".to_string(),
            dependency_from_spec(
                ctx,
                NativeBackendDependencySpec::new(
                    "waterui-preview",
                    WATERUI_VERSION,
                    &[],
                    Some(NativeBackendDependencyPathKind::WorkspaceSubdir(
                        "components/preview",
                    )),
                ),
            ),
        );

        let mut wasm_dependencies = BTreeMap::new();
        wasm_dependencies.insert(
            "hydrolysis".to_string(),
            dependency_from_spec(
                ctx,
                NativeBackendDependencySpec::new(
                    "hydrolysis",
                    WATERUI_HYDROLYSIS_VERSION,
                    &["web"],
                    Some(NativeBackendDependencyPathKind::BackendsSubdir(
                        "hydrolysis",
                    )),
                ),
            ),
        );
        wasm_dependencies.insert(
            "wasm-bindgen".to_string(),
            DependencyValue::Simple("0.2".to_string()),
        );

        let mut target = BTreeMap::new();
        target.insert(
            "cfg(not(target_arch = \"wasm32\"))".to_string(),
            TargetSection {
                dependencies: native_dependencies,
            },
        );
        target.insert(
            "cfg(target_arch = \"wasm32\")".to_string(),
            TargetSection {
                dependencies: wasm_dependencies,
            },
        );

        let manifest = CargoManifest {
            package: PackageSection {
                name: package_name.to_string(),
                version: "0.1.0".to_string(),
                edition: "2024".to_string(),
            },
            lib: LibSection {
                crate_type: vec!["cdylib".to_string(), "rlib".to_string()],
            },
            features: BTreeMap::from([("waterui-preview-mode".to_string(), Vec::new())]),
            dependencies,
            target,
            workspace: WorkspaceSection {},
        };

        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        fs::write(base_dir.join("Cargo.toml"), toml_string).await
    }
}

/// Native FFI companion crate templates.
pub mod ffi {
    use cargo_toml::{Dependency, DependencyDetail, Manifest, Package, Product, Workspace};

    use super::{
        NativeBackendDependencyPathKind, Path, TemplateContext, WATERUI_FFI_VERSION,
        TemplateNamespace, WATERUI_VERSION, compute_native_backend_dependency_path, embedded, fs, io,
        normalize_path_for_config, scaffold_dir,
    };

    /// Write all FFI companion templates to the given directory.
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
        scaffold_dir(TemplateNamespace::Ffi, &embedded::FFI, base_dir, ctx).await
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let mut manifest = Manifest::<()>::default();
        manifest.package = Some(Package::new(package_name.to_string(), "0.1.0".to_string()));
        if let Some(ref mut package) = manifest.package {
            package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
        }

        manifest.lib = Some(Product {
            crate_type: vec![
                "staticlib".to_string(),
                "cdylib".to_string(),
                "rlib".to_string(),
            ],
            ..Default::default()
        });

        manifest.dependencies.insert(
            ctx.crate_name.to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(ctx.project_root_relative_path()),
                ..Default::default()
            })),
        );

        let waterui_dependency = if let Some(waterui_path) = &ctx.waterui_path {
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(compute_native_backend_dependency_path(
                    ctx,
                    waterui_path,
                    NativeBackendDependencyPathKind::WateruiRoot,
                )),
                default_features: false,
                ..Default::default()
            }))
        } else {
            Dependency::Detailed(Box::new(DependencyDetail {
                version: Some(WATERUI_VERSION.to_string()),
                default_features: false,
                ..Default::default()
            }))
        };
        manifest
            .dependencies
            .insert("waterui".to_string(), waterui_dependency);

        let ffi_dependency = if let Some(waterui_path) = &ctx.waterui_path {
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(normalize_path_for_config(&waterui_path.join("ffi"))),
                ..Default::default()
            }))
        } else {
            Dependency::Detailed(Box::new(DependencyDetail {
                version: Some(WATERUI_FFI_VERSION.to_string()),
                ..Default::default()
            }))
        };
        manifest
            .dependencies
            .insert("waterui-ffi".to_string(), ffi_dependency);

        manifest.workspace = Some(Workspace::default());

        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        fs::write(base_dir.join("Cargo.toml"), toml_string).await?;
        Ok(())
    }
}

/// Root-level templates (Cargo.toml, lib.rs, .gitignore).
pub mod root {
    use crate::templates::WATERUI_VERSION;

    use super::{
        Path, TemplateContext, TemplateNamespace, embedded, fs, io, normalize_path_for_config,
        render_scaffold_template,
    };

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
                let rendered = render_scaffold_template(
                    TemplateNamespace::Root,
                    Path::new(template_name),
                    content,
                    ctx,
                )?;
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
            dependencies: BTreeMap<String, DependencyDetail>,
            #[serde(skip_serializing_if = "BTreeMap::is_empty")]
            target: BTreeMap<String, TargetSection>,
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
        struct TargetSection {
            dependencies: BTreeMap<String, DependencyDetail>,
        }

        #[derive(Serialize, Clone)]
        struct DependencyDetail {
            #[serde(skip_serializing_if = "Option::is_none")]
            path: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            version: Option<String>,
            #[serde(rename = "default-features", skip_serializing_if = "Option::is_none")]
            default_features: Option<bool>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            features: Vec<String>,
        }

        let mut dependencies = BTreeMap::new();
        let mut target = BTreeMap::new();

        fn path_dependency(path: &Path) -> DependencyDetail {
            DependencyDetail {
                path: Some(normalize_path_for_config(path)),
                version: None,
                default_features: None,
                features: Vec::new(),
            }
        }

        fn version_dependency(version: &str) -> DependencyDetail {
            DependencyDetail {
                path: None,
                version: Some(version.to_string()),
                default_features: None,
                features: Vec::new(),
            }
        }

        let mut waterui_dependency = if let Some(waterui_path) = &ctx.waterui_path {
            path_dependency(waterui_path)
        } else {
            version_dependency(WATERUI_VERSION)
        };
        waterui_dependency.default_features = Some(false);
        dependencies.insert("waterui".to_string(), waterui_dependency.clone());

        let mut native_dependencies = BTreeMap::new();
        let mut native_waterui_dependency = waterui_dependency;
        native_waterui_dependency.features = vec![
            "assets".to_string(),
            "media".to_string(),
            "webview".to_string(),
            "flow-markdown".to_string(),
        ];
        native_dependencies.insert("waterui".to_string(), native_waterui_dependency);

        if !native_dependencies.is_empty() {
            target.insert(
                "cfg(not(target_arch = \"wasm32\"))".to_string(),
                TargetSection {
                    dependencies: native_dependencies,
                },
            );
        }

        let manifest = CargoManifest {
            package: PackageSection {
                name: ctx.crate_name.to_string(),
                version: "0.1.0".to_string(),
                edition: "2024".to_string(),
                authors: vec![ctx.author.clone()],
            },
            lib: LibSection {
                crate_type: vec!["lib".to_string()],
            },
            dependencies,
            target,
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
        Path, TemplateContext, TemplateNamespace, dependency_path, dependency_version, embedded,
        io, scaffold_dir, write_support_cargo_toml,
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
        scaffold_dir(TemplateNamespace::Preview, &embedded::PREVIEW, base_dir, ctx).await
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
        write_support_cargo_toml(base_dir, ctx.crate_name.as_str(), dependencies).await
    }
}

/// Inspector app templates.
pub mod inspector {
    use super::{
        Path, TemplateContext, TemplateNamespace, dependency_path, dependency_version, embedded,
        io, scaffold_dir, write_support_cargo_toml,
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
        scaffold_dir(TemplateNamespace::Inspector, &embedded::INSPECTOR, base_dir, ctx).await
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

        write_support_cargo_toml(base_dir, ctx.crate_name.as_str(), dependencies).await
    }
}
