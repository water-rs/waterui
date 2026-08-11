//! Type-safe template scaffolding for `WaterUI` project backends.
//!
//! Uses `include_dir` to embed templates at compile time and provides
//! a type-safe substitution API for generating Apple and Android backend projects.

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::build_info::{
    ANDROID_BACKEND, APPLE_BACKEND, DEW_VERSION, GTK_BACKEND_VERSION, HYDROLYSIS_VERSION,
    PREVIEW_VERSION, WATERUI_FFI_VERSION, WATERUI_VERSION,
};
use askama::Template;

use include_dir::{Dir, include_dir};
use smol::fs;

use crate::project::WebViewBackend;
use crate::project_types::{BundleIdentifier, CrateName, RustIdent};

/// Normalize a path to use forward slashes for config files (Cargo.toml, Xcode projects, etc.)
/// This is necessary because Windows uses backslashes but these config files expect forward slashes.
fn normalize_path_for_config(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn cargo_semver(version: &str) -> cargo_toml::SemVer {
    version
        .parse()
        .unwrap_or_else(|error| panic!("Invalid Cargo semantic version `{version}`: {error}"))
}

fn cargo_version_req(version: &str) -> cargo_toml::VersionReq {
    version
        .parse()
        .unwrap_or_else(|error| panic!("Invalid Cargo version requirement `{version}`: {error}"))
}

/// Embedded template directories.
/// A stable digest of every scaffold template baked into this CLI.
///
/// The generated host crates are a product of these templates, so anything that
/// caches a generated crate has to treat a template edit the same way it treats
/// a runtime source edit. Without this, upgrading the CLI leaves previously
/// generated crates in place, and they fail to compile against the API they
/// were meant to be regenerated for.
pub(crate) fn scaffold_template_digest() -> String {
    use sha2::Digest as _;

    fn hash_dir(hasher: &mut sha2::Sha256, dir: &Dir<'_>) {
        // `include_dir` yields entries in a stable order, but sort anyway so the
        // digest cannot depend on directory-walk order.
        let mut files: Vec<_> = dir.files().collect();
        files.sort_by_key(|file| file.path());
        for file in files {
            hasher.update(file.path().to_string_lossy().as_bytes());
            hasher.update(file.contents());
        }
        let mut dirs: Vec<_> = dir.dirs().collect();
        dirs.sort_by_key(|entry| entry.path());
        for entry in dirs {
            hash_dir(hasher, entry);
        }
    }

    let mut hasher = sha2::Sha256::new();
    for dir in [
        &embedded::ROOT,
        &embedded::HYDROLYSIS,
        &embedded::PREVIEW,
        &embedded::PREVIEW_FFI,
        &embedded::INSPECTOR,
        &embedded::FFI,
    ] {
        hash_dir(&mut hasher, dir);
    }
    let digest = hasher.finalize();
    digest.iter().take(8).fold(String::new(), |mut out, byte| {
        use core::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

mod embedded {
    use super::{Dir, include_dir};

    pub static APPLE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/apple");
    pub static ANDROID: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/android");
    pub static FFI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/ffi");
    pub static GTK4: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/gtk4");
    pub static HYDROLYSIS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/hydrolysis");
    pub static ESP32: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/esp32");
    pub static PREVIEW: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/preview");
    pub static PREVIEW_FFI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/preview_ffi");
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

/// ESP32 harness parameters substituted into the generated firmware crate.
///
/// `chip` is the single source of truth; the firmware fields are derived from
/// it via [`crate::esp32::chip::Esp32Chip::firmware_params`] when the entry is
/// constructed, so the harness templates never special-case a chip by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Esp32TemplateEntry {
    /// Target chip (e.g. "esp32s3"); selects the target triple and every
    /// chip-specific firmware parameter below.
    pub chip: String,
    /// Panel width in pixels.
    pub panel_width: u32,
    /// Panel height in pixels.
    pub panel_height: u32,
    /// Maximum rows per rasterization band (bounds scratch memory).
    pub band_height: u32,
    /// Route the firmware console to UART0 (`true`) or USB-Serial-JTAG.
    pub console_uart_default: bool,
    /// Flash size in megabytes (`CONFIG_ESPTOOLPY_FLASHSIZE_*MB`).
    pub flash_size_mb: u32,
    /// Main-task stack size in bytes (`CONFIG_ESP_MAIN_TASK_STACK_SIZE`).
    pub main_task_stack_bytes: u32,
    /// Offset of the app (`factory`) partition.
    pub app_partition_offset: String,
    /// Size of the app (`factory`) partition.
    pub app_partition_size: String,
    /// Cargo codegen `opt-level` for the firmware profiles.
    pub opt_level: String,
}

impl Esp32TemplateEntry {
    /// Builds a harness entry for `chip` with the given panel geometry,
    /// deriving every chip-specific firmware parameter from the chip's
    /// architecture.
    #[must_use]
    pub fn new(
        chip: crate::esp32::chip::Esp32Chip,
        panel_width: u32,
        panel_height: u32,
        band_height: u32,
    ) -> Self {
        let params = chip.firmware_params();
        Self {
            chip: chip.id().to_string(),
            panel_width,
            panel_height,
            band_height,
            console_uart_default: params.console_uart_default,
            flash_size_mb: params.flash_size_mb,
            main_task_stack_bytes: params.main_task_stack_bytes,
            app_partition_offset: params.app_partition_offset.to_string(),
            app_partition_size: params.app_partition_size.to_string(),
            opt_level: params.opt_level.to_string(),
        }
    }

    /// The Rust target triple for the configured chip (e.g.
    /// `riscv32imc-esp-espidf`), used by the `.cargo/config.toml` template and
    /// by regeneration checks.
    ///
    /// # Panics
    ///
    /// Panics when `chip` is not a supported ESP32 chip; the entry is only ever
    /// constructed from an already-validated [`crate::esp32::chip::Esp32Chip`].
    #[must_use]
    pub fn resolved_target_triple(&self) -> &'static str {
        self.chip
            .parse::<crate::esp32::chip::Esp32Chip>()
            .unwrap_or_else(|error| panic!("Esp32TemplateEntry holds an invalid chip: {error}"))
            .target_triple()
    }
}

impl Default for Esp32TemplateEntry {
    fn default() -> Self {
        Self::new(crate::esp32::chip::Esp32Chip::Esp32S3, 410, 502, 16)
    }
}

/// Browser features rendered into a generated backend manifest.
#[derive(Debug, Clone, Copy)]
pub struct BrowserTemplateContext {
    /// Standard `WebView` engine selected by `Water.toml`.
    pub webview_backend: WebViewBackend,
    /// Whether the packaged application links the standard `WebView` component.
    pub webview_enabled: bool,
    /// Whether the packaged application links the independent Chromium component.
    pub chromium_enabled: bool,
}

impl BrowserTemplateContext {
    const fn new(webview_backend: WebViewBackend) -> Self {
        Self {
            webview_backend,
            webview_enabled: false,
            chromium_enabled: false,
        }
    }
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
    /// The bundle identifier (e.g., "dev.waterui.myapp")
    pub bundle_identifier: BundleIdentifier,
    /// The author name
    pub author: String,
    /// Path to the Android backend (relative or absolute)
    pub android_backend_path: Option<PathBuf>,
    /// Whether to use remote dev backend (`JitPack`) instead of local
    pub use_remote_dev_backend: bool,
    /// Path to local `WaterUI` repository (for dev mode)
    pub waterui_path: Option<PathBuf>,
    /// Browser engine and component selections for generated backend manifests.
    pub browser: BrowserTemplateContext,
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
    /// Exact `WaterUI` feature set linked into a preview support runtime.
    pub preview_runtime_features: Vec<String>,
    /// User crate whose dependency graph defines the preview runtime ABI.
    pub preview_app_dependency: Option<(CrateName, PathBuf)>,
    /// Package type of the project being scaffolded.
    pub package_type: crate::project::PackageType,
    /// ESP32 harness parameters used by the esp32 templates.
    pub esp32: Esp32TemplateEntry,
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
            browser: BrowserTemplateContext::new(WebViewBackend::Default),
            backend_project_path: None,
            project_root_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: false,
            preview_runtime_fingerprint: None,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            package_type: options.package_type,
            esp32: Esp32TemplateEntry::default(),
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
            browser: BrowserTemplateContext::new(manifest.webview_backend),
            backend_project_path: None,
            project_root_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: manifest.package.accessory,
            preview_runtime_fingerprint: None,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            package_type: manifest.package.package_type,
            esp32: Esp32TemplateEntry::default(),
        }
    }

    /// Build a context for support applications that always run as playground projects.
    #[must_use]
    pub fn for_support_playground(
        app_display_name: impl Into<String>,
        app_name: impl Into<String>,
        crate_name: CrateName,
        bundle_identifier: BundleIdentifier,
        waterui_path: Option<PathBuf>,
        accessory: bool,
        preview_runtime_fingerprint: Option<String>,
    ) -> Self {
        let android_backend_path = waterui_path
            .as_ref()
            .map(|waterui_path| waterui_path.join("backends/android"));
        Self {
            app_display_name: app_display_name.into(),
            app_name: app_name.into(),
            crate_name,
            bundle_identifier,
            author: String::new(),
            android_backend_path,
            use_remote_dev_backend: waterui_path.is_none(),
            waterui_path,
            browser: BrowserTemplateContext::new(WebViewBackend::Default),
            backend_project_path: None,
            project_root_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory,
            preview_runtime_fingerprint,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            package_type: crate::project::PackageType::Playground,
            esp32: Esp32TemplateEntry::default(),
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

    /// Set whether the application runtime graph links `waterui-webview`.
    #[must_use]
    pub const fn with_webview_enabled(mut self, enabled: bool) -> Self {
        self.browser.webview_enabled = enabled;
        self
    }

    /// Set whether the application runtime graph links `waterui-chromium`.
    #[must_use]
    pub const fn with_chromium_enabled(mut self, enabled: bool) -> Self {
        self.browser.chromium_enabled = enabled;
        self
    }

    fn webview_backend_feature(&self) -> Option<&'static str> {
        self.browser
            .webview_enabled
            .then_some(self.browser.webview_backend.cargo_feature())
    }

    const fn chromium_enabled(&self) -> bool {
        self.browser.chromium_enabled
    }

    const fn cef_webview_enabled(&self) -> bool {
        self.browser.webview_enabled && matches!(self.browser.webview_backend, WebViewBackend::Cef)
    }

    const fn cef_runtime_enabled(&self) -> bool {
        self.chromium_enabled() || self.cef_webview_enabled()
    }

    /// Set the exact `WaterUI` feature set used by a preview support runtime.
    #[must_use]
    pub fn with_preview_runtime_features(mut self, features: Vec<String>) -> Self {
        self.preview_runtime_features = features;
        self
    }

    /// Set the user crate used to reproduce the preview module's dependency graph.
    #[must_use]
    pub fn with_preview_app_dependency(mut self, crate_name: CrateName, path: PathBuf) -> Self {
        self.preview_app_dependency = Some((crate_name, path));
        self
    }

    /// Set Android permissions for template rendering.
    #[must_use]
    pub fn with_android_permissions(
        mut self,
        permissions: Vec<AndroidPermissionTemplateEntry>,
    ) -> Self {
        self.android_permissions = permissions;
        self
    }

    /// Set iOS permissions for template rendering.
    #[must_use]
    pub fn with_ios_permissions(mut self, permissions: Vec<IosPermissionTemplateEntry>) -> Self {
        self.ios_permissions = permissions;
        self
    }

    /// Set ESP32 harness parameters for template rendering.
    #[must_use]
    pub fn with_esp32(mut self, esp32: Esp32TemplateEntry) -> Self {
        self.esp32 = esp32;
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
    #[expect(
        clippy::unused_self,
        reason = "Askama invokes template context values through instance methods"
    )]
    pub const fn android_min_api_level(&self) -> u32 {
        crate::android::ANDROID_MIN_API_LEVEL
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
    #[allow(clippy::unused_self)]
    pub fn android_remote_backend_dependency(&self) -> String {
        jitpack_dependency_coordinate(ANDROID_BACKEND.repository_url, ANDROID_BACKEND.commit)
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
        let repository_name = github_repository_name(APPLE_BACKEND.repository_url);

        self.compute_apple_backend_path().map_or_else(
            || {
                format!(
                    "{INDENT}{PACKAGE_ID} /* XCRemoteSwiftPackageReference \"{repository_name}\" */,"
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
        let repository_name = github_repository_name(APPLE_BACKEND.repository_url);

        self.compute_apple_backend_path().map_or_else(
            || {
                format!(
                    "/* Begin XCRemoteSwiftPackageReference section */\n\
                    \t\t{PACKAGE_ID} /* XCRemoteSwiftPackageReference \"{repository_name}\" */ = {{\n\
                    \t\t\tisa = XCRemoteSwiftPackageReference;\n\
                    \t\t\trepositoryURL = \"{}\";\n\
                    \t\t\trequirement = {{\n\
                    \t\t\t\tkind = revision;\n\
                    \t\t\t\trevision = \"{}\";\n\
                    \t\t\t}};\n\
                    \t\t}};\n\
                    /* End XCRemoteSwiftPackageReference section */",
                    APPLE_BACKEND.repository_url,
                    APPLE_BACKEND.commit,
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
    Esp32,
    Inspector,
    Preview,
    PreviewFfi,
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
            Self::Esp32 => "src/templates/esp32",
            Self::Inspector => "src/templates/inspector",
            Self::Preview => "src/templates/preview",
            Self::PreviewFfi => "src/templates/preview_ffi",
            Self::Root => "src/templates",
        }
    }
}

fn scaffold_template_dispatch_path(namespace: TemplateNamespace, relative_path: &Path) -> String {
    let relative_path = normalize_path_for_config(relative_path);
    if relative_path.starts_with("src/templates/") {
        return relative_path;
    }
    format!("{}/{relative_path}", namespace.scaffold_template_prefix())
}

fn github_repository_owner_and_name(repository_url: &str) -> (&str, &str) {
    let path = repository_url
        .strip_prefix("https://github.com/")
        .or_else(|| repository_url.strip_prefix("git@github.com:"))
        .unwrap_or_else(|| panic!("unsupported GitHub repository URL: {repository_url}"));
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = path.split('/');
    let owner = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| panic!("missing GitHub owner in repository URL: {repository_url}"));
    let repo = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| {
            panic!("missing GitHub repository name in repository URL: {repository_url}")
        });
    assert!(
        segments.next().is_none(),
        "unsupported GitHub repository URL path: {repository_url}"
    );
    (owner, repo)
}

fn github_repository_name(repository_url: &str) -> &str {
    let (_, repo) = github_repository_owner_and_name(repository_url);
    repo
}

fn jitpack_dependency_coordinate(repository_url: &str, commit: &str) -> String {
    let (owner, repo) = github_repository_owner_and_name(repository_url);
    format!("com.github.{owner}:{repo}:{commit}")
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
                "src/templates/esp32/Cargo.toml.tpl" => Esp32CargoTomlTemplate::from_ctx(ctx)
                    .render()
                    .map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Failed to render template {display_path}: {error}"),
                        )
                    }),
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
#[template(
    path = "src/templates/apple/AppName/WaterUIFonts.swift.tpl",
    escape = "none"
)]
struct ScaffoldAppleFontTemplate<'a> {
    font_entries: &'a [FontRegistrationTemplateEntry],
}

/// Generated `Cargo.toml` for the ESP32 firmware harness crate.
///
/// Rendered through an askama template (instead of a serialized manifest)
/// so the generated file can carry the Xtensa miscompilation profile note.
#[derive(Template)]
#[template(path = "src/templates/esp32/Cargo.toml.tpl", escape = "none")]
struct Esp32CargoTomlTemplate {
    package_name: String,
    app_crate_name: String,
    app_crate_path: String,
    dew_path: Option<String>,
    core_path: Option<String>,
    dew_version: &'static str,
    waterui_version: &'static str,
    opt_level: String,
}

impl Esp32CargoTomlTemplate {
    fn from_ctx(ctx: &TemplateContext) -> Self {
        let dew_path = ctx.waterui_path.as_ref().map(|waterui_path| {
            compute_native_backend_dependency_path(
                ctx,
                waterui_path,
                NativeBackendDependencyPathKind::BackendsSubdir("dew"),
            )
        });
        let core_path = ctx.waterui_path.as_ref().map(|waterui_path| {
            compute_native_backend_dependency_path(
                ctx,
                waterui_path,
                NativeBackendDependencyPathKind::WorkspaceSubdir("core"),
            )
        });

        Self {
            package_name: ctx.crate_name.with_suffix("esp32").to_string(),
            app_crate_name: ctx.crate_name.to_string(),
            app_crate_path: ctx.project_root_relative_path(),
            dew_path,
            core_path,
            dew_version: DEW_VERSION,
            waterui_version: WATERUI_VERSION,
            opt_level: ctx.esp32.opt_level.clone(),
        }
    }
}

define_scaffold_templates! {
    AppleProjectTemplate => (Apple, "src/templates/apple/AppName.xcodeproj/project.pbxproj.tpl"),
    AppleAppTemplate => (Apple, "src/templates/apple/AppName/AppNameApp.swift.tpl"),
    AppleBuildScriptTemplate => (Apple, "src/templates/apple/build-rust.sh.tpl"),
    AndroidGradleAppTemplate => (Android, "src/templates/android/app/build.gradle.kts.tpl"),
    AndroidManifestTemplate => (Android, "src/templates/android/app/src/main/AndroidManifest.xml.tpl"),
    AndroidMainActivityTemplate => (Android, "src/templates/android/app/src/main/java/MainActivity.kt.tpl"),
    AndroidApplicationTemplate => (Android, "src/templates/android/app/src/main/java/WaterUiApplication.kt.tpl"),
    AndroidStringsTemplate => (Android, "src/templates/android/app/src/main/res/values/strings.xml.tpl"),
    AndroidSettingsTemplate => (Android, "src/templates/android/settings.gradle.kts.tpl"),
    FfiLibTemplate => (Ffi, "src/templates/ffi/src/lib.rs.tpl"),
    Gtk4MainTemplate => (Gtk4, "src/templates/gtk4/src/main.rs.tpl"),
    HydrolysisLibTemplate => (Hydrolysis, "src/templates/hydrolysis/src/lib.rs.tpl"),
    HydrolysisMainTemplate => (Hydrolysis, "src/templates/hydrolysis/src/main.rs.tpl"),
    HydrolysisPreviewRuntimeTemplate => (Hydrolysis, "src/templates/hydrolysis/src/preview_runtime.rs.tpl"),
    HydrolysisPreviewTestRuntimeTemplate => (Hydrolysis, "src/templates/hydrolysis/src/preview_test_runtime.rs.tpl"),
    HydrolysisWebIndexTemplate => (Hydrolysis, "src/templates/hydrolysis/web/index.html.tpl"),
    Esp32MainTemplate => (Esp32, "src/templates/esp32/src/main.rs.tpl"),
    Esp32CargoConfigTemplate => (Esp32, "src/templates/esp32/.cargo/config.toml.tpl"),
    Esp32SdkconfigTemplate => (Esp32, "src/templates/esp32/sdkconfig.defaults.tpl"),
    Esp32PartitionsTemplate => (Esp32, "src/templates/esp32/partitions.csv.tpl"),
    PreviewLibTemplate => (Preview, "src/templates/preview/src/lib.rs.tpl"),
    PreviewFfiLibTemplate => (PreviewFfi, "src/templates/preview_ffi/src/lib.rs.tpl"),
}

#[cfg(test)]
mod tests {
    use super::{
        ANDROID_BACKEND, APPLE_BACKEND, BrowserTemplateContext, Esp32TemplateEntry,
        GTK_BACKEND_VERSION, PREVIEW_VERSION, TemplateContext, TemplateNamespace, embedded,
        jitpack_dependency_coordinate, normalize_path_for_config, preview_ffi,
        render_scaffold_template,
    };
    use crate::project::WebViewBackend;
    use crate::project_types::{BundleIdentifier, CrateName};
    use std::path::PathBuf;
    use tempfile::tempdir;

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
            browser: BrowserTemplateContext::new(WebViewBackend::Default),
            backend_project_path,
            project_root_path,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: false,
            preview_runtime_fingerprint: None,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            package_type,
            esp32: Esp32TemplateEntry::default(),
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
            Some(PathBuf::from("../..")),
            false,
            None,
        )
        .with_backend_project_path(PathBuf::from("managed_backends/apple"))
    }

    fn render_esp32(relative: &str, ctx: &TemplateContext) -> String {
        let template = embedded::ESP32
            .get_file(relative)
            .unwrap_or_else(|| panic!("esp32 template {relative} must exist"))
            .contents_utf8()
            .expect("esp32 template must be utf-8");
        render_scaffold_template(
            TemplateNamespace::Esp32,
            std::path::Path::new(relative),
            template,
            ctx,
        )
        .unwrap_or_else(|error| panic!("esp32 template {relative} render: {error}"))
    }

    #[test]
    fn esp32_templates_are_chip_architecture_aware() {
        use crate::esp32::chip::Esp32Chip;

        let mut s3 = app_ctx();
        s3.esp32 = Esp32TemplateEntry::new(Esp32Chip::Esp32S3, 410, 502, 16);
        let mut c3 = app_ctx();
        c3.esp32 = Esp32TemplateEntry::new(Esp32Chip::Esp32C3, 200, 240, 16);

        // .cargo/config.toml: Xtensa per-chip triple vs RISC-V architecture triple.
        let s3_cargo = render_esp32(".cargo/config.toml.tpl", &s3);
        assert!(s3_cargo.contains("target = \"xtensa-esp32s3-espidf\""));
        assert!(s3_cargo.contains("MCU = \"esp32s3\""));
        let c3_cargo = render_esp32(".cargo/config.toml.tpl", &c3);
        assert!(c3_cargo.contains("target = \"riscv32imc-esp-espidf\""));
        assert!(c3_cargo.contains("MCU = \"esp32c3\""));

        // sdkconfig: USB-Serial-JTAG + 8 MB + bigger stack on S3; UART0 + 4 MB on C3.
        let s3_sdk = render_esp32("sdkconfig.defaults.tpl", &s3);
        assert!(s3_sdk.contains("CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y"));
        assert!(s3_sdk.contains("CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y"));
        assert!(s3_sdk.contains("CONFIG_ESP_MAIN_TASK_STACK_SIZE=163840"));
        let c3_sdk = render_esp32("sdkconfig.defaults.tpl", &c3);
        assert!(c3_sdk.contains("CONFIG_ESP_CONSOLE_UART_DEFAULT=y"));
        assert!(c3_sdk.contains("CONFIG_ESPTOOLPY_FLASHSIZE_4MB=y"));
        assert!(c3_sdk.contains("CONFIG_ESP_MAIN_TASK_STACK_SIZE=49152"));

        // partitions: 6 MB app on S3, 3 MB on C3.
        assert!(render_esp32("partitions.csv.tpl", &s3).contains("0x10000, 0x600000,"));
        assert!(render_esp32("partitions.csv.tpl", &c3).contains("0x10000, 0x300000,"));

        // Cargo.toml profile: size-opt on Xtensa, full-opt on RISC-V; both enable
        // the dew progress widget.
        let s3_manifest = render_esp32("Cargo.toml.tpl", &s3);
        assert!(s3_manifest.contains("opt-level = \"s\""));
        assert!(s3_manifest.contains("features = [\"espidf\", \"progress\"]"));
        let c3_manifest = render_esp32("Cargo.toml.tpl", &c3);
        assert!(c3_manifest.contains("opt-level = \"2\""));
        assert!(c3_manifest.contains("features = [\"espidf\", \"progress\"]"));

        // main.rs panel geometry follows the entry.
        assert!(render_esp32("src/main.rs.tpl", &c3).contains("PanelConfig::new(200, 240, 16)"));
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
        assert!(rendered.contains("libwaterui_app.a in Frameworks"));
        assert!(!rendered.contains("-lwaterui_app"));
        assert!(rendered.contains(APPLE_BACKEND.repository_url));
        assert!(rendered.contains(APPLE_BACKEND.commit));
        assert!(rendered.contains("kind = revision;"));
    }

    #[test]
    fn apple_chromium_template_links_and_initializes_cef_before_appkit() {
        let ctx = app_ctx().with_chromium_enabled(true);
        let project_template = embedded::APPLE
            .get_file("AppName.xcodeproj/project.pbxproj.tpl")
            .expect("apple project template must exist")
            .contents_utf8()
            .expect("apple project template must be utf-8");
        let project = render_scaffold_template(
            TemplateNamespace::Apple,
            std::path::Path::new("AppName.xcodeproj/project.pbxproj.tpl"),
            project_template,
            &ctx,
        )
        .expect("apple Chromium project render");
        assert!(project.contains("WaterUICEF in Frameworks"));
        assert!(project.contains("WaterUIChromium in Frameworks"));
        assert!(!project.contains("WaterUICefWebView in Frameworks"));

        let app_template = embedded::APPLE
            .get_file("AppName/AppNameApp.swift.tpl")
            .expect("apple app template must exist")
            .contents_utf8()
            .expect("apple app template must be utf-8");
        let app = render_scaffold_template(
            TemplateNamespace::Apple,
            std::path::Path::new("AppName/AppNameApp.swift.tpl"),
            app_template,
            &ctx,
        )
        .expect("apple Chromium app render");
        assert!(app.contains("import WaterUICEF"));
        assert!(app.contains("import WaterUIChromium"));
        assert!(app.contains("installWaterUIChromium()"));
        assert!(
            app.find("prepareWaterUICEFApplication()") < app.find("let app = NSApplication.shared")
        );
        assert!(!app.contains("runWaterUICEFSubprocessIfNeeded()"));
    }

    #[test]
    fn apple_cef_webview_template_links_only_the_standard_cef_component() {
        let mut ctx = app_ctx().with_webview_enabled(true);
        ctx.browser.webview_backend = WebViewBackend::Cef;
        let project_template = embedded::APPLE
            .get_file("AppName.xcodeproj/project.pbxproj.tpl")
            .expect("apple project template must exist")
            .contents_utf8()
            .expect("apple project template must be utf-8");
        let project = render_scaffold_template(
            TemplateNamespace::Apple,
            std::path::Path::new("AppName.xcodeproj/project.pbxproj.tpl"),
            project_template,
            &ctx,
        )
        .expect("apple CEF WebView project render");
        assert!(project.contains("WaterUICEF in Frameworks"));
        assert!(project.contains("WaterUICefWebView in Frameworks"));
        assert!(!project.contains("WaterUIChromium in Frameworks"));

        let app_template = embedded::APPLE
            .get_file("AppName/AppNameApp.swift.tpl")
            .expect("apple app template must exist")
            .contents_utf8()
            .expect("apple app template must be utf-8");
        let app = render_scaffold_template(
            TemplateNamespace::Apple,
            std::path::Path::new("AppName/AppNameApp.swift.tpl"),
            app_template,
            &ctx,
        )
        .expect("apple CEF WebView app render");
        assert!(app.contains("import WaterUICefWebView"));
        assert!(app.contains("installWaterUICefWebView()"));
        assert!(!app.contains("installWaterUIChromium()"));
    }

    #[test]
    fn android_build_gradle_uses_embedded_remote_backend_commit() {
        let ctx = app_ctx();
        let template = embedded::ANDROID
            .get_file("app/build.gradle.kts.tpl")
            .expect("android build.gradle template must exist")
            .contents_utf8()
            .expect("android build.gradle template must be utf-8");

        let rendered = render_scaffold_template(
            TemplateNamespace::Android,
            std::path::Path::new("app/build.gradle.kts.tpl"),
            template,
            &ctx,
        )
        .expect("android build.gradle render");

        assert!(rendered.contains("minSdk = 26"));
        assert!(rendered.contains(&jitpack_dependency_coordinate(
            ANDROID_BACKEND.repository_url,
            ANDROID_BACKEND.commit,
        )));
    }

    #[test]
    fn android_activity_installs_edge_to_edge_and_leases_activity_context() {
        let ctx = app_ctx();
        let activity_template = embedded::ANDROID
            .get_file("app/src/main/java/MainActivity.kt.tpl")
            .expect("android MainActivity template must exist")
            .contents_utf8()
            .expect("android MainActivity template must be utf-8");

        let activity = render_scaffold_template(
            TemplateNamespace::Android,
            std::path::Path::new("app/src/main/java/MainActivity.kt.tpl"),
            activity_template,
            &ctx,
        )
        .expect("android MainActivity render");

        assert!(activity.contains("enableEdgeToEdge()"));
        assert!(activity.contains("waterUiApplication.acquireRuntime(this)"));
        assert!(activity.contains("androidRuntimeLease.close()"));
        assert!(activity.contains("val reportActivityFinished = !isChangingConfigurations"));
        assert!(activity.contains("reportActivityFinished && releasedActiveRuntime"));
        assert!(activity.contains("WATERUI_ACTIVITY_FINISHED"));

        let application_template = embedded::ANDROID
            .get_file("app/src/main/java/WaterUiApplication.kt.tpl")
            .expect("android WaterUiApplication template must exist")
            .contents_utf8()
            .expect("android WaterUiApplication template must be utf-8");
        let application = render_scaffold_template(
            TemplateNamespace::Android,
            std::path::Path::new("app/src/main/java/WaterUiApplication.kt.tpl"),
            application_template,
            &ctx,
        )
        .expect("android WaterUiApplication render");

        assert!(application.starts_with("package com.example.test"));
        assert!(application.contains("activeRuntime?.let { previous ->"));
        assert!(application.contains("releaseWaterUiRuntime(previous.owner)"));
        assert!(application.contains("bootstrapWaterUiRuntime(activity)"));
        assert!(application.contains("if (runtime.generation != generation) return false"));
        assert!(application.contains("return application.releaseRuntime(generation)"));
        assert!(application.contains("Application(), WaterUiRuntimeOwner"));
        assert!(application.contains("processEnvironment = WuiEnvironment.create()"));
        assert!(application.contains("createWaterUiEnvironment(): WuiEnvironment"));
        assert!(application.contains("}.clone()"));

        let manifest_template = embedded::ANDROID
            .get_file("app/src/main/AndroidManifest.xml.tpl")
            .expect("android manifest template must exist")
            .contents_utf8()
            .expect("android manifest template must be utf-8");
        let manifest = render_scaffold_template(
            TemplateNamespace::Android,
            std::path::Path::new("app/src/main/AndroidManifest.xml.tpl"),
            manifest_template,
            &ctx,
        )
        .expect("android manifest render");

        assert!(manifest.contains("android:name=\".WaterUiApplication\""));
        assert!(manifest.contains("android:launchMode=\"singleTask\""));
    }

    #[test]
    fn gtk4_scaffold_uses_embedded_workspace_version() {
        let ctx = app_ctx();
        let tempdir = tempdir().expect("temporary gtk scaffold dir");

        smol::block_on(crate::templates::gtk4::scaffold(
            tempdir.path(),
            &ctx,
            "waterui-test-gtk",
        ))
        .expect("gtk4 scaffold should succeed");

        let cargo_toml = std::fs::read_to_string(tempdir.path().join("Cargo.toml"))
            .expect("gtk4 Cargo.toml should be written");
        assert!(cargo_toml.contains(&format!("version = \"{GTK_BACKEND_VERSION}\"")));
        assert!(!cargo_toml.contains("webview-default"));
    }

    #[test]
    fn generated_native_backends_only_select_webview_when_linked() {
        let mut gtk_ctx = app_ctx().with_webview_enabled(true);
        gtk_ctx.browser.webview_backend = WebViewBackend::Wpe;
        let tempdir = tempdir().expect("temporary gtk webview scaffold dir");
        smol::block_on(crate::templates::gtk4::scaffold(
            tempdir.path(),
            &gtk_ctx,
            "waterui-test-gtk",
        ))
        .expect("gtk4 webview scaffold should succeed");
        let gtk_manifest = std::fs::read_to_string(tempdir.path().join("Cargo.toml"))
            .expect("gtk4 Cargo.toml should be written");
        assert!(gtk_manifest.contains("features = [\"webview-wpe\"]"));

        let mut gtk_cef_ctx = app_ctx().with_webview_enabled(true);
        gtk_cef_ctx.browser.webview_backend = WebViewBackend::Cef;
        let gtk_cef_manifest =
            crate::templates::gtk4::rendered_outputs(&gtk_cef_ctx, "waterui-test-gtk-cef")
                .expect("GTK CEF outputs should render")
                .into_iter()
                .find_map(|(path, content)| {
                    (path == std::path::Path::new("Cargo.toml"))
                        .then(|| String::from_utf8(content).expect("Cargo.toml must be UTF-8"))
                })
                .expect("GTK CEF Cargo.toml output should exist");
        assert!(gtk_cef_manifest.contains("features = [\"webview-cef\"]"));

        let mut hydrolysis_ctx = app_ctx().with_webview_enabled(true);
        hydrolysis_ctx.browser.webview_backend = WebViewBackend::Cef;
        let cargo_toml = crate::templates::hydrolysis::rendered_outputs(
            &hydrolysis_ctx,
            "waterui-test-hydrolysis",
        )
        .expect("hydrolysis outputs should render")
        .into_iter()
        .find_map(|(path, content)| {
            (path == std::path::Path::new("Cargo.toml"))
                .then(|| String::from_utf8(content).expect("Cargo.toml must be UTF-8"))
        })
        .expect("hydrolysis Cargo.toml output should exist");
        let manifest = cargo_toml
            .parse::<toml::Table>()
            .expect("hydrolysis Cargo.toml should parse");
        let features =
            manifest["target"]["cfg(not(target_arch = \"wasm32\"))"]["dependencies"]["hydrolysis"]
                ["features"]
                .as_array()
                .expect("hydrolysis dependency features should be an array")
                .iter()
                .map(|feature| feature.as_str().expect("feature should be a string"))
                .collect::<Vec<_>>();
        assert_eq!(features, ["winit", "webview-cef"]);
        assert_eq!(manifest["package"]["autobins"].as_bool(), Some(false));
        let bins = manifest["bin"]
            .as_array()
            .expect("CEF Hydrolysis manifest should declare binaries");
        assert!(bins.iter().any(|bin| {
            bin["name"].as_str() == Some("waterui-cef-helper")
                && bin["path"].as_str() == Some("src/bin/waterui-cef-helper.rs")
        }));
    }

    #[test]
    fn preview_scaffold_uses_embedded_workspace_version() {
        let tempdir = tempdir().expect("temporary preview scaffold dir");
        let ctx = app_ctx()
            .with_preview_runtime_features(vec!["dynamic_linking".to_string(), "gpu".to_string()])
            .with_preview_app_dependency(
                CrateName::try_from("preview_test_app").expect("test crate name must be valid"),
                tempdir.path().join("app"),
            );

        smol::block_on(crate::templates::preview::scaffold(tempdir.path(), &ctx))
            .expect("preview scaffold should succeed");

        let cargo_toml = std::fs::read_to_string(tempdir.path().join("Cargo.toml"))
            .expect("preview Cargo.toml should be written");
        assert!(cargo_toml.contains(&format!("version = \"{PREVIEW_VERSION}\"")));
        assert!(cargo_toml.contains("default-features = false"));
        let manifest = cargo_toml
            .parse::<toml::Table>()
            .expect("preview Cargo.toml should parse");
        let dev_features = manifest["features"]["dev"]
            .as_array()
            .expect("preview dev feature should be an array")
            .iter()
            .map(|feature| feature.as_str().expect("feature should be a string"))
            .collect::<Vec<_>>();
        assert_eq!(dev_features, ["waterui/dynamic_linking", "waterui/gpu"]);
        assert!(cargo_toml.contains("package = \"preview_test_app\""));
        assert!(cargo_toml.contains("features = [\"dev\"]"));

        let lib_rs = std::fs::read_to_string(tempdir.path().join("src/lib.rs"))
            .expect("preview lib.rs should be written");
        assert!(!lib_rs.contains("waterui_ffi::export!()"));
    }

    #[test]
    fn ffi_scaffold_resolves_waterui_ffi_from_playground_cache_path() {
        let tempdir = tempdir().expect("temporary ffi scaffold dir");
        let project_root = tempdir.path().join("playground");
        let ffi_dir = tempdir
            .path()
            .join("cache")
            .join("managed_backends")
            .join("ffi");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root.clone()),
            crate::project::PackageType::Playground,
        );

        smol::block_on(crate::templates::ffi::scaffold(
            &ffi_dir,
            &ctx,
            "playground-ffi",
        ))
        .expect("ffi scaffold should succeed");

        let cargo_toml = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written");
        let expected_ffi_path = pathdiff::diff_paths(project_root.join("../waterui/ffi"), &ffi_dir)
            .expect("expected waterui ffi dependency diff path");
        let expected_ffi_path = normalize_path_for_config(&expected_ffi_path);

        assert!(cargo_toml.contains(&format!("path = \"{expected_ffi_path}\"")));
        assert!(cargo_toml.contains("dev = [\"waterui_test/dev\"]"));
        let manifest = cargo_toml
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        assert_eq!(
            manifest["dependencies"]["waterui-ffi"]["default-features"].as_bool(),
            Some(false)
        );
        assert_eq!(manifest["package"]["autobins"].as_bool(), Some(false));
        assert!(manifest.get("bin").is_none());
    }

    #[test]
    fn ffi_scaffold_declares_minimal_cef_helper_for_chromium() {
        let tempdir = tempdir().expect("temporary ffi scaffold dir");
        let ffi_dir = tempdir.path().join("managed_backends/ffi");
        let ctx = app_ctx()
            .with_backend_project_path(ffi_dir.clone())
            .with_project_root_path(tempdir.path().to_path_buf())
            .with_chromium_enabled(true);

        smol::block_on(crate::templates::ffi::scaffold(
            &ffi_dir,
            &ctx,
            "chromium-ffi",
        ))
        .expect("Chromium ffi scaffold should succeed");

        let manifest = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        let bins = manifest["bin"]
            .as_array()
            .expect("CEF FFI companion should declare a helper binary");
        assert_eq!(bins.len(), 1);
        assert_eq!(bins[0]["name"].as_str(), Some("waterui-cef-helper"));
        assert_eq!(
            bins[0]["path"].as_str(),
            Some("src/bin/waterui-cef-helper.rs")
        );

        let helper = std::fs::read_to_string(ffi_dir.join("src/bin/waterui-cef-helper.rs"))
            .expect("CEF helper source should be written");
        assert!(helper.contains("waterui_cef_run_packaged_subprocess"));
    }

    #[test]
    fn preview_ffi_scaffold_emits_dylib_only_wrapper() {
        let tempdir = tempdir().expect("temporary preview ffi scaffold dir");
        let project_root = tempdir.path().join("playground");
        let preview_ffi_dir = tempdir
            .path()
            .join("cache")
            .join("managed_backends")
            .join("preview_ffi");
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CLI crate should be inside the WaterUI workspace")
            .to_path_buf();
        let ctx = ctx(
            Some(workspace_root),
            Some(preview_ffi_dir.clone()),
            Some(project_root),
            crate::project::PackageType::Playground,
        );

        smol::block_on(crate::templates::preview_ffi::scaffold(
            &preview_ffi_dir,
            &ctx,
            "playground-preview-ffi",
        ))
        .expect("preview ffi scaffold should succeed");

        let cargo_toml = std::fs::read_to_string(preview_ffi_dir.join("Cargo.toml"))
            .expect("preview ffi Cargo.toml should be written");
        let manifest = cargo_toml
            .parse::<toml::Table>()
            .expect("preview ffi Cargo.toml should parse");
        for (feature, ffi_feature) in [
            (preview_ffi::APPLE_ABI_FEATURE, "waterui-ffi/c-api"),
            (preview_ffi::ANDROID_ABI_FEATURE, "waterui-ffi/android-jni"),
        ] {
            let features = manifest["features"][feature]
                .as_array()
                .expect("platform preview ABI feature should be an array")
                .iter()
                .map(|feature| feature.as_str().expect("feature should be a string"))
                .collect::<Vec<_>>();
            assert_eq!(
                features,
                ["dep:waterui-ffi", ffi_feature, "dep:waterui-preview"]
            );
        }
        assert_eq!(
            manifest["dependencies"]["waterui-ffi"]["optional"].as_bool(),
            Some(true)
        );
        assert_eq!(
            manifest["dependencies"]["waterui-ffi"]["default-features"].as_bool(),
            Some(false)
        );
        assert_eq!(
            manifest["dependencies"]["waterui-preview"]["optional"].as_bool(),
            Some(true)
        );
        assert!(cargo_toml.contains("crate-type = [\"dylib\"]"));
        assert!(cargo_toml.contains("features = [\"dev\"]"));
        assert!(!cargo_toml.contains("[dependencies.waterui]"));
        assert!(!cargo_toml.contains("staticlib"));
        assert!(!cargo_toml.contains("rlib"));
        assert!(!cargo_toml.contains("cdylib"));
    }

    #[test]
    fn generated_ffi_manifest_emits_only_linked_crate_types() {
        let temp = tempfile::tempdir().expect("temp dir");
        let project_root = temp.path().join("project");
        let ffi_dir = temp
            .path()
            .join("cache")
            .join("managed_backends")
            .join("ffi");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root),
            crate::project::PackageType::Playground,
        );

        smol::block_on(crate::templates::ffi::scaffold(
            &ffi_dir,
            &ctx,
            "playground-ffi",
        ))
        .expect("ffi scaffold should succeed");

        let manifest = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        let crate_types = manifest["lib"]["crate-type"]
            .as_array()
            .expect("crate-type should be an array")
            .iter()
            .map(|value| value.as_str().expect("crate type should be a string"))
            .collect::<Vec<_>>();

        // Apple links the staticlib, Android loads the cdylib, and nothing anywhere
        // consumes an rlib of this crate.
        assert_eq!(crate_types, ["staticlib", "cdylib"]);
    }

    #[test]
    fn generated_manifests_keep_debug_info_off_for_dependencies() {
        let temp = tempfile::tempdir().expect("temp dir");
        let project_root = temp.path().join("project");
        let ffi_dir = temp
            .path()
            .join("cache")
            .join("managed_backends")
            .join("ffi");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root),
            crate::project::PackageType::Playground,
        );

        smol::block_on(crate::templates::ffi::scaffold(
            &ffi_dir,
            &ctx,
            "playground-ffi",
        ))
        .expect("ffi scaffold should succeed");

        let manifest = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        let dev = &manifest["profile"]["dev"];

        // Generated crates declare `[workspace]`, so they inherit no profile and have
        // to carry this themselves.
        assert_eq!(dev["debug"].as_integer(), Some(1));
        assert_eq!(dev["package"]["*"]["debug"].as_bool(), Some(false));
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
        assert!(rendered.contains("libwaterui_app.a in Frameworks"));
        assert!(!rendered.contains("-lwaterui_app"));
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
                write_file_if_changed(&full_dest, rendered.as_bytes()).await?;
            } else {
                // Binary file - copy as-is
                write_file_if_changed(&full_dest, file.contents()).await?;
            }
        }

        // Add subdirectories to the stack
        for subdir in current_dir.dirs() {
            dirs_to_process.push(subdir);
        }
    }

    Ok(())
}

/// Render every file of an embedded scaffold directory to its destination
/// path and content, without touching the filesystem.
///
/// This is the same rendering [`scaffold_dir`] performs, exposed so callers
/// can compare a generated backend against what the current templates would
/// produce (managed backends regenerate exactly when the rendering differs).
fn render_dir_outputs(
    namespace: TemplateNamespace,
    embedded_dir: &Dir<'_>,
    ctx: &TemplateContext,
) -> io::Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut outputs = Vec::new();
    let mut dirs_to_process = vec![embedded_dir];
    while let Some(current_dir) = dirs_to_process.pop() {
        for file in current_dir.files() {
            let relative_path = file.path();
            let is_template = relative_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "tpl");
            if is_template {
                let dest_path = ctx.transform_path(&relative_path.with_extension(""));
                let content = file
                    .contents_utf8()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-8"))?;
                let rendered = render_scaffold_template(namespace, relative_path, content, ctx)?;
                outputs.push((dest_path, rendered.into_bytes()));
            } else {
                let dest_path = ctx.transform_path(relative_path);
                outputs.push((dest_path, file.contents().to_vec()));
            }
        }
        for subdir in current_dir.dirs() {
            dirs_to_process.push(subdir);
        }
    }
    Ok(outputs)
}

async fn write_file_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    match fs::read(path).await {
        Ok(existing) if existing == contents => return Ok(()),
        Ok(_) | Err(_) => {}
    }

    fs::write(path, contents).await
}

#[derive(serde::Serialize)]
struct SupportCargoManifest {
    package: SupportPackageSection,
    lib: SupportLibSection,
    profile: cargo_toml::Profiles,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    features: std::collections::BTreeMap<String, Vec<String>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(rename = "default-features", skip_serializing_if = "Option::is_none")]
    default_features: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    features: Vec<String>,
}

/// Build the `[profile.dev]` section every generated crate carries.
///
/// Generated crates declare `[workspace]`, which makes each of them its own
/// workspace root: they inherit nothing from the repository or the user's project,
/// so whatever profile they should build under has to be written into them here.
/// Without this they defaulted to full debug info for the entire dependency graph,
/// which is the bulk of both the link time and the artifact size of a debug build,
/// and none of which anyone steps through — the `WaterUI` runtime is a dependency of
/// these crates, not the code under debug.
///
/// Line tables are kept for the generated crate itself so panics still resolve to
/// file and line.
fn generated_dev_profile() -> cargo_toml::Profiles {
    let mut dev = cargo_toml::Profile {
        debug: Some(cargo_toml::DebugSetting::Lines),
        ..Default::default()
    };
    let mut dependency_override = toml::value::Table::new();
    dependency_override.insert("debug".to_string(), toml::Value::Boolean(false));
    dev.package
        .insert("*".to_string(), toml::Value::Table(dependency_override));

    cargo_toml::Profiles {
        dev: Some(dev),
        ..Default::default()
    }
}

async fn write_support_cargo_toml(
    base_dir: &Path,
    crate_name: &str,
    features: std::collections::BTreeMap<String, Vec<String>>,
    dependencies: std::collections::BTreeMap<String, SupportDependencyValue>,
) -> io::Result<()> {
    let manifest = SupportCargoManifest {
        package: SupportPackageSection {
            name: crate_name.to_string(),
            version: "0.1.0".to_string(),
            edition: "2024".to_string(),
        },
        lib: SupportLibSection {
            // A support app's own crate is only ever consumed as a Rust dependency of
            // the generated FFI crate, and that FFI crate is what the platform links.
            // Emitting `staticlib`/`cdylib` here archived and relinked the entire
            // dependency graph twice more for products nothing ever loads.
            crate_type: vec!["rlib".to_string()],
        },
        profile: generated_dev_profile(),
        features,
        dependencies,
        workspace: SupportWorkspaceSection {},
    };

    let toml_string = toml::to_string_pretty(&manifest)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::create_dir_all(base_dir).await?;
    write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
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
    let toml_string = render_native_backend_bin_cargo_toml(ctx, package_name, dependencies)?;
    fs::create_dir_all(base_dir).await?;
    write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await
}

fn render_native_backend_bin_cargo_toml(
    ctx: &TemplateContext,
    package_name: &str,
    dependencies: &[NativeBackendDependencySpec<'_>],
) -> io::Result<String> {
    use cargo_toml::{Dependency, DependencyDetail, Manifest, Package, Workspace};

    let mut manifest = Manifest::<()>::default();
    let mut package = Package::new(package_name.to_string(), cargo_semver("0.1.0"));
    package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
    manifest.package = Some(package);
    manifest.profile = generated_dev_profile();

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
            .map(std::string::ToString::to_string)
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
                version: Some(cargo_version_req(dependency.version)),
                features,
                ..Default::default()
            })),
        );
    }

    manifest.workspace = Some(Workspace::default());

    toml::to_string_pretty(&manifest)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn dependency_path(path: &Path) -> SupportDependencyValue {
    SupportDependencyValue::Detailed(SupportDependencyDetail {
        package: None,
        version: None,
        path: Some(normalize_path_for_config(path)),
        default_features: None,
        features: Vec::new(),
    })
}

fn dependency_version(version: &str) -> SupportDependencyValue {
    SupportDependencyValue::Simple(version.to_string())
}

#[derive(serde::Serialize)]
struct GeneratedCargoManifest<T> {
    package: GeneratedPackageSection,
    lib: GeneratedLibSection,
    #[serde(rename = "bin", skip_serializing_if = "Vec::is_empty", default)]
    bins: Vec<GeneratedBinSection>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    features: std::collections::BTreeMap<String, Vec<String>>,
    dependencies: std::collections::BTreeMap<String, T>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    target: std::collections::BTreeMap<String, GeneratedTargetSection<T>>,
    workspace: GeneratedWorkspaceSection,
}

#[derive(serde::Serialize)]
struct GeneratedPackageSection {
    name: String,
    version: String,
    edition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    autobins: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    authors: Vec<String>,
}

#[derive(serde::Serialize)]
struct GeneratedLibSection {
    #[serde(rename = "crate-type")]
    crate_type: Vec<String>,
}

#[derive(serde::Serialize)]
struct GeneratedBinSection {
    name: String,
    path: String,
}

#[derive(serde::Serialize)]
struct GeneratedTargetSection<T> {
    dependencies: std::collections::BTreeMap<String, T>,
}

#[derive(serde::Serialize)]
struct GeneratedWorkspaceSection {}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum GeneratedDependencyValue {
    Simple(String),
    Detailed(GeneratedDependencyDetail),
}

#[derive(serde::Serialize, Clone)]
struct GeneratedDependencyDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(rename = "default-features", skip_serializing_if = "Option::is_none")]
    default_features: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    features: Vec<String>,
}

impl GeneratedDependencyValue {
    const fn detailed(detail: GeneratedDependencyDetail) -> Self {
        Self::Detailed(detail)
    }

    fn simple(version: &str) -> Self {
        Self::Simple(version.to_string())
    }
}

impl GeneratedDependencyDetail {
    fn path(path: &Path) -> Self {
        Self {
            version: None,
            path: Some(normalize_path_for_config(path)),
            default_features: None,
            features: Vec::new(),
        }
    }

    fn version(version: &str) -> Self {
        Self {
            version: Some(version.to_string()),
            path: None,
            default_features: None,
            features: Vec::new(),
        }
    }

    const fn with_default_features(mut self, default_features: bool) -> Self {
        self.default_features = Some(default_features);
        self
    }

    fn with_features(mut self, features: &[&str]) -> Self {
        self.features = features
            .iter()
            .map(|feature| (*feature).to_string())
            .collect();
        self
    }
}

fn generated_package(name: &str, authors: Vec<String>) -> GeneratedPackageSection {
    GeneratedPackageSection {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        edition: "2024".to_string(),
        autobins: None,
        authors,
    }
}

fn generated_lib(crate_types: &[&str]) -> GeneratedLibSection {
    GeneratedLibSection {
        crate_type: crate_types
            .iter()
            .map(|crate_type| (*crate_type).to_string())
            .collect(),
    }
}

fn generated_dependency_from_spec(
    ctx: &TemplateContext,
    spec: NativeBackendDependencySpec<'_>,
) -> GeneratedDependencyDetail {
    let detail = if let Some(waterui_path) = &ctx.waterui_path
        && let Some(path_kind) = spec.path_kind
    {
        GeneratedDependencyDetail {
            version: None,
            path: Some(compute_native_backend_dependency_path(
                ctx,
                waterui_path,
                path_kind,
            )),
            default_features: None,
            features: Vec::new(),
        }
    } else {
        GeneratedDependencyDetail::version(spec.version)
    };

    detail.with_features(spec.features)
}

fn render_generated_cargo_toml<T: serde::Serialize>(
    manifest: &GeneratedCargoManifest<T>,
) -> io::Result<String> {
    toml::to_string_pretty(manifest)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

async fn write_generated_cargo_toml(base_dir: &Path, toml_string: String) -> io::Result<()> {
    fs::create_dir_all(base_dir).await?;
    write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await
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
        scaffold_dir, write_file_if_changed,
    };

    /// Write all Android templates to the given directory.
    ///
    /// # Errors
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(
            TemplateNamespace::Android,
            &embedded::ANDROID,
            base_dir,
            ctx,
        )
        .await?;

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
            write_file_if_changed(&local_props, content.as_bytes()).await?;
        }

        Ok(())
    }
}

/// GTK4 backend templates.
pub mod gtk4 {
    use super::{
        GTK_BACKEND_VERSION, NativeBackendDependencyPathKind, NativeBackendDependencySpec, Path,
        TemplateContext, TemplateNamespace, embedded, io, scaffold_dir,
        write_native_backend_bin_cargo_toml,
    };

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

    /// Every file `scaffold` would write, without touching the filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if template or Cargo manifest rendering fails.
    pub fn rendered_outputs(
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<Vec<(std::path::PathBuf, Vec<u8>)>> {
        let mut outputs = super::render_dir_outputs(TemplateNamespace::Gtk4, &embedded::GTK4, ctx)?;
        let mut features = ctx
            .webview_backend_feature()
            .into_iter()
            .collect::<Vec<_>>();
        if ctx.browser.chromium_enabled {
            features.push("chromium");
        }
        let dependencies = [NativeBackendDependencySpec::new(
            "waterui-gtk",
            GTK_BACKEND_VERSION,
            &features,
            Some(NativeBackendDependencyPathKind::BackendsSubdir("gtk")),
        )];
        outputs.push((
            std::path::PathBuf::from("Cargo.toml"),
            super::render_native_backend_bin_cargo_toml(ctx, package_name, &dependencies)?
                .into_bytes(),
        ));
        Ok(outputs)
    }

    /// Generate `GTK4` `Cargo.toml` programmatically using the `cargo_toml` crate.
    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let mut features = ctx
            .webview_backend_feature()
            .into_iter()
            .collect::<Vec<_>>();
        if ctx.browser.chromium_enabled {
            features.push("chromium");
        }
        let dependencies = [NativeBackendDependencySpec::new(
            "waterui-gtk",
            GTK_BACKEND_VERSION,
            &features,
            Some(NativeBackendDependencyPathKind::BackendsSubdir("gtk")),
        )];
        write_native_backend_bin_cargo_toml(base_dir, ctx, package_name, &dependencies).await
    }
}

/// Hydrolysis backend templates.
pub mod hydrolysis {
    use super::{
        GeneratedBinSection, GeneratedCargoManifest, GeneratedDependencyDetail,
        GeneratedDependencyValue, GeneratedTargetSection, GeneratedWorkspaceSection,
        HYDROLYSIS_VERSION, NativeBackendDependencyPathKind, NativeBackendDependencySpec, Path,
        TemplateContext, TemplateNamespace, WATERUI_VERSION, embedded, io, scaffold_dir,
        write_generated_cargo_toml,
    };
    use std::collections::BTreeMap;

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
        scaffold_dir(
            TemplateNamespace::Hydrolysis,
            &embedded::HYDROLYSIS,
            base_dir,
            ctx,
        )
        .await
    }

    /// Every file `scaffold` would write, as backend-relative path and
    /// content, without touching the filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if template rendering fails.
    pub fn rendered_outputs(
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<Vec<(std::path::PathBuf, Vec<u8>)>> {
        let mut outputs =
            super::render_dir_outputs(TemplateNamespace::Hydrolysis, &embedded::HYDROLYSIS, ctx)?;
        outputs.push((
            std::path::PathBuf::from("Cargo.toml"),
            super::render_generated_cargo_toml(&generated_manifest(ctx, package_name))?
                .into_bytes(),
        ));
        Ok(outputs)
    }

    fn generated_manifest(
        ctx: &TemplateContext,
        package_name: &str,
    ) -> GeneratedCargoManifest<GeneratedDependencyValue> {
        let mut package = super::generated_package(package_name, Vec::new());
        package.autobins = Some(false);
        let mut bins = vec![GeneratedBinSection {
            name: package_name.to_string(),
            path: "src/main.rs".to_string(),
        }];
        if requires_cef(ctx) {
            bins.push(GeneratedBinSection {
                name: "waterui-cef-helper".to_string(),
                path: "src/bin/waterui-cef-helper.rs".to_string(),
            });
        }
        GeneratedCargoManifest {
            package,
            lib: super::generated_lib(&["cdylib", "rlib"]),
            bins,
            features: BTreeMap::from([
                ("waterui-preview-mode".to_string(), Vec::new()),
                ("waterui-preview-test-mode".to_string(), Vec::new()),
            ]),
            dependencies: cargo_dependencies(ctx),
            target: cargo_target_dependencies(ctx),
            workspace: GeneratedWorkspaceSection {},
        }
    }

    fn requires_cef(ctx: &TemplateContext) -> bool {
        ctx.browser.chromium_enabled || ctx.webview_backend_feature() == Some("webview-cef")
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let manifest = generated_manifest(ctx, package_name);
        write_generated_cargo_toml(base_dir, super::render_generated_cargo_toml(&manifest)?).await
    }

    fn cargo_dependencies(ctx: &TemplateContext) -> BTreeMap<String, GeneratedDependencyValue> {
        BTreeMap::from([
            (
                ctx.crate_name.to_string(),
                GeneratedDependencyValue::detailed(GeneratedDependencyDetail {
                    version: None,
                    path: Some(ctx.project_root_relative_path()),
                    default_features: None,
                    features: Vec::new(),
                }),
            ),
            (
                "waterui".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui",
                            WATERUI_VERSION,
                            &[],
                            Some(NativeBackendDependencyPathKind::WateruiRoot),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
        ])
    }

    fn cargo_target_dependencies(
        ctx: &TemplateContext,
    ) -> BTreeMap<String, GeneratedTargetSection<GeneratedDependencyValue>> {
        BTreeMap::from([
            (
                "cfg(not(target_arch = \"wasm32\"))".to_string(),
                GeneratedTargetSection {
                    dependencies: native_target_dependencies(ctx),
                },
            ),
            (
                "cfg(target_arch = \"wasm32\")".to_string(),
                GeneratedTargetSection {
                    dependencies: wasm_target_dependencies(ctx),
                },
            ),
        ])
    }

    #[allow(
        clippy::too_many_lines,
        reason = "linear enumeration of native target dependencies reads clearest as one list"
    )]
    fn native_target_dependencies(
        ctx: &TemplateContext,
    ) -> BTreeMap<String, GeneratedDependencyValue> {
        let mut hydrolysis_features = vec!["winit"];
        hydrolysis_features.extend(ctx.webview_backend_feature());
        if ctx.browser.chromium_enabled {
            hydrolysis_features.push("chromium");
        }
        BTreeMap::from([
            (
                "hydrolysis".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis",
                            HYDROLYSIS_VERSION,
                            &hydrolysis_features,
                            Some(NativeBackendDependencyPathKind::BackendsSubdir(
                                "hydrolysis",
                            )),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
            (
                "pollster".to_string(),
                GeneratedDependencyValue::simple("0.4"),
            ),
            (
                "pprof".to_string(),
                GeneratedDependencyValue::detailed(GeneratedDependencyDetail {
                    version: Some("0.15".to_string()),
                    path: None,
                    default_features: None,
                    features: vec!["flamegraph".to_string()],
                }),
            ),
            (
                "waterui-core".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-core",
                            WATERUI_VERSION,
                            &[],
                            Some(NativeBackendDependencyPathKind::WorkspaceSubdir("core")),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
            (
                "waterui-preview".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-preview",
                            WATERUI_VERSION,
                            &[],
                            Some(NativeBackendDependencyPathKind::WorkspaceSubdir(
                                "components/devtools/preview/runtime",
                            )),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
            (
                "waterui-preview-protocol".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-preview-protocol",
                            WATERUI_VERSION,
                            &[],
                            Some(NativeBackendDependencyPathKind::WorkspaceSubdir(
                                "components/devtools/preview/protocol",
                            )),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
            (
                "serde_json".to_string(),
                GeneratedDependencyValue::simple("1"),
            ),
            (
                "waterui-testing".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-testing",
                            WATERUI_VERSION,
                            &[],
                            Some(NativeBackendDependencyPathKind::WorkspaceSubdir("testing")),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
            (
                "hydrolysis-m3".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis-m3",
                            HYDROLYSIS_VERSION,
                            &[],
                            Some(NativeBackendDependencyPathKind::BackendsSubdir(
                                "hydrolysis_m3",
                            )),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
        ])
    }

    fn wasm_target_dependencies(
        ctx: &TemplateContext,
    ) -> BTreeMap<String, GeneratedDependencyValue> {
        BTreeMap::from([
            (
                "hydrolysis".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis",
                            HYDROLYSIS_VERSION,
                            &["web"],
                            Some(NativeBackendDependencyPathKind::BackendsSubdir(
                                "hydrolysis",
                            )),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
            (
                "wasm-bindgen".to_string(),
                GeneratedDependencyValue::simple("0.2"),
            ),
            (
                "hydrolysis-m3".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis-m3",
                            HYDROLYSIS_VERSION,
                            &[],
                            Some(NativeBackendDependencyPathKind::BackendsSubdir(
                                "hydrolysis_m3",
                            )),
                        ),
                    )
                    .with_default_features(false),
                ),
            ),
        ])
    }
}

/// ESP32 firmware harness templates.
pub mod esp32 {
    use super::{Path, TemplateContext, TemplateNamespace, embedded, io, scaffold_dir};

    /// Write all ESP32 harness templates to the given directory.
    ///
    /// The generated `Cargo.toml` and `src/main.rs` are rendered from the
    /// template context (including `ctx.esp32` harness parameters); the
    /// remaining files (toolchain pin, cargo config, sdkconfig, partition
    /// table, build script) are static.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(TemplateNamespace::Esp32, &embedded::ESP32, base_dir, ctx).await
    }
}

/// Native FFI companion crate templates.
pub mod ffi {
    use cargo_toml::{Dependency, DependencyDetail, Manifest, Package, Product, Workspace};

    use super::{
        NativeBackendDependencyPathKind, Path, TemplateContext, TemplateNamespace,
        WATERUI_FFI_VERSION, WATERUI_VERSION, cargo_semver, cargo_version_req,
        compute_native_backend_dependency_path, embedded, fs, generated_dev_profile, io,
        scaffold_dir, write_file_if_changed,
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
        let mut package = Package::new(package_name.to_string(), cargo_semver("0.1.0"));
        package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
        package.autobins = false;
        manifest.package = Some(package);
        manifest.profile = generated_dev_profile();

        // Apple links `lib<ffi>.a` and Android loads `lib<ffi>.so`, so the manifest
        // declares only that union; nothing ever consumes an `rlib` of this crate.
        // Each build then narrows further to the single crate type its platform
        // links, via `RustBuild::with_crate_type_override`.
        manifest.lib = Some(Product {
            crate_type: vec!["staticlib".to_string(), "cdylib".to_string()],
            ..Default::default()
        });
        if ctx.cef_runtime_enabled() {
            manifest.bin.push(Product {
                name: Some("waterui-cef-helper".to_string()),
                path: Some("src/bin/waterui-cef-helper.rs".to_string()),
                ..Default::default()
            });
        }

        manifest.dependencies.insert(
            ctx.crate_name.to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(ctx.project_root_relative_path()),
                ..Default::default()
            })),
        );

        manifest
            .features
            .insert("dev".to_string(), vec![format!("{}/dev", ctx.crate_name)]);

        let waterui_dependency = ctx.waterui_path.as_ref().map_or_else(
            || {
                Dependency::Detailed(Box::new(DependencyDetail {
                    version: Some(cargo_version_req(WATERUI_VERSION)),
                    default_features: false,
                    ..Default::default()
                }))
            },
            |waterui_path| {
                Dependency::Detailed(Box::new(DependencyDetail {
                    path: Some(compute_native_backend_dependency_path(
                        ctx,
                        waterui_path,
                        NativeBackendDependencyPathKind::WateruiRoot,
                    )),
                    default_features: false,
                    ..Default::default()
                }))
            },
        );
        manifest
            .dependencies
            .insert("waterui".to_string(), waterui_dependency);

        let ffi_dependency = ctx.waterui_path.as_ref().map_or_else(
            || {
                Dependency::Detailed(Box::new(DependencyDetail {
                    version: Some(cargo_version_req(WATERUI_FFI_VERSION)),
                    default_features: false,
                    ..Default::default()
                }))
            },
            |waterui_path| {
                Dependency::Detailed(Box::new(DependencyDetail {
                    path: Some(compute_native_backend_dependency_path(
                        ctx,
                        waterui_path,
                        NativeBackendDependencyPathKind::WorkspaceSubdir("ffi"),
                    )),
                    default_features: false,
                    ..Default::default()
                }))
            },
        );
        manifest
            .dependencies
            .insert("waterui-ffi".to_string(), ffi_dependency);

        manifest.workspace = Some(Workspace::default());

        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
        Ok(())
    }
}

/// Root-level templates (Cargo.toml, lib.rs, .gitignore).
pub mod root {
    use crate::templates::WATERUI_VERSION;

    use super::{
        GeneratedCargoManifest, GeneratedDependencyDetail, GeneratedTargetSection,
        GeneratedWorkspaceSection, Path, TemplateContext, TemplateNamespace, embedded, fs, io,
        render_scaffold_template, write_file_if_changed, write_generated_cargo_toml,
    };
    use std::collections::BTreeMap;

    /// Root template files, paired with their destination relative to the
    /// project root. The assets README is what makes the documented `assets!`
    /// workflow work out of the box: the planner walks the assets root
    /// recursively, so the directory has to exist before the first `assets!`
    /// call, and a tracked file is what keeps it present in git.
    static ROOT_TEMPLATES: &[(&str, &str)] = &[
        ("lib.rs.tpl", "src/lib.rs"),
        (".gitignore.tpl", ".gitignore"),
    ];

    /// Write root templates to the given directory.
    ///
    /// `assets_dir` is the project's assets root, relative to `base_dir`, and
    /// comes from the manifest so the scaffold and `Water.toml` cannot disagree.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(
        base_dir: &Path,
        ctx: &TemplateContext,
        assets_dir: &str,
    ) -> io::Result<()> {
        // Generate Cargo.toml programmatically using toml_edit
        generate_cargo_toml(base_dir, ctx).await?;

        let assets_readme = format!("{assets_dir}/README.md");
        let templates = ROOT_TEMPLATES
            .iter()
            .map(|(template, dest)| (*template, (*dest).to_string()))
            .chain(core::iter::once(("assets_readme.md.tpl", assets_readme)));

        // Process remaining templates
        for (template_name, dest) in templates {
            if let Some(file) = embedded::ROOT.get_file(template_name) {
                let dest_path = base_dir.join(&dest);

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
                write_file_if_changed(&dest_path, rendered.as_bytes()).await?;
            }
        }
        Ok(())
    }

    /// Generate Cargo.toml programmatically using serde-compatible structs for type safety.
    async fn generate_cargo_toml(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        let waterui_dependency = waterui_dependency(ctx);
        let manifest = GeneratedCargoManifest {
            package: super::generated_package(ctx.crate_name.as_str(), vec![ctx.author.clone()]),
            lib: super::generated_lib(&["lib"]),
            bins: Vec::new(),
            features: BTreeMap::from([(
                "dev".to_string(),
                vec!["waterui/dynamic_linking".to_string()],
            )]),
            dependencies: BTreeMap::from([("waterui".to_string(), waterui_dependency.clone())]),
            target: native_target_section(waterui_dependency),
            workspace: GeneratedWorkspaceSection {},
        };

        write_generated_cargo_toml(base_dir, super::render_generated_cargo_toml(&manifest)?).await
    }

    fn waterui_dependency(ctx: &TemplateContext) -> GeneratedDependencyDetail {
        ctx.waterui_path
            .as_ref()
            .map_or_else(
                || GeneratedDependencyDetail::version(WATERUI_VERSION),
                |waterui_path| GeneratedDependencyDetail::path(waterui_path),
            )
            .with_default_features(false)
    }

    fn native_target_section(
        waterui_dependency: GeneratedDependencyDetail,
    ) -> BTreeMap<String, GeneratedTargetSection<GeneratedDependencyDetail>> {
        BTreeMap::from([(
            "cfg(not(target_arch = \"wasm32\"))".to_string(),
            GeneratedTargetSection {
                dependencies: BTreeMap::from([(
                    "waterui".to_string(),
                    waterui_dependency.with_features(&["assets", "media", "flow-markdown"]),
                )]),
            },
        )])
    }
}

/// Preview app templates.
pub mod preview {
    use crate::templates::{PREVIEW_VERSION, WATERUI_VERSION};

    use super::{
        Path, SupportDependencyDetail, SupportDependencyValue, TemplateContext, TemplateNamespace,
        dependency_path, dependency_version, embedded, io, scaffold_dir, write_support_cargo_toml,
    };

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
        hex::encode(hasher.finalize())
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
        scaffold_dir(
            TemplateNamespace::Preview,
            &embedded::PREVIEW,
            base_dir,
            ctx,
        )
        .await
    }

    /// Resolves the on-disk directory of a `waterui` workspace member crate from
    /// the workspace's own cargo metadata.
    ///
    /// The preview-support scaffold depends on internal `waterui` crates by path.
    /// Those paths must track the real crate location inside the workspace rather
    /// than a hardcoded relative path, which silently breaks when a crate moves
    /// (e.g. `waterui-preview` relocating from `components/preview` to
    /// `components/devtools/preview/runtime`): a stale path makes the scaffold's
    /// `cargo metadata` fail and aborts the whole preview build.
    pub(super) async fn resolve_workspace_member_dir(
        workspace_root: &Path,
        package_name: &str,
    ) -> io::Result<std::path::PathBuf> {
        let manifest = workspace_root.join("Cargo.toml");
        let metadata = smol::unblock(move || {
            cargo_metadata::MetadataCommand::new()
                .manifest_path(&manifest)
                .no_deps()
                .exec()
        })
        .await
        .map_err(io::Error::other)?;
        let member = metadata
            .packages
            .iter()
            .find(|package| package.name == package_name)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "`{package_name}` is not a member of the waterui workspace at {}",
                        workspace_root.display()
                    ),
                )
            })?;
        member
            .manifest_path
            .as_std_path()
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "failed to derive crate directory for `{package_name}`"
                ))
            })
    }

    /// Generate preview app Cargo.toml programmatically.
    async fn generate_cargo_toml(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        use std::collections::BTreeMap;

        let mut dependencies = BTreeMap::new();

        if let Some(waterui_path) = &ctx.waterui_path {
            // Local path dependencies
            dependencies.insert(
                "waterui".to_string(),
                SupportDependencyValue::Detailed(SupportDependencyDetail {
                    package: None,
                    version: None,
                    path: Some(super::normalize_path_for_config(waterui_path)),
                    default_features: Some(false),
                    features: Vec::new(),
                }),
            );

            // Resolve `waterui-preview` from the workspace metadata so the path
            // tracks the crate if it is moved within the workspace.
            let preview_path =
                resolve_workspace_member_dir(waterui_path, "waterui-preview").await?;
            dependencies.insert(
                "waterui-preview".to_string(),
                dependency_path(&preview_path),
            );
        } else {
            // Registry dependencies
            dependencies.insert(
                "waterui".to_string(),
                SupportDependencyValue::Detailed(SupportDependencyDetail {
                    package: None,
                    version: Some(WATERUI_VERSION.to_string()),
                    path: None,
                    default_features: Some(false),
                    features: Vec::new(),
                }),
            );
            dependencies.insert(
                "waterui-preview".to_string(),
                dependency_version(PREVIEW_VERSION),
            );
        }
        let (app_crate_name, app_path) = ctx.preview_app_dependency.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Preview support runtime requires a user crate dependency",
            )
        })?;
        dependencies.insert(
            "waterui-preview-app".to_string(),
            SupportDependencyValue::Detailed(SupportDependencyDetail {
                package: Some(app_crate_name.to_string()),
                version: None,
                path: Some(super::normalize_path_for_config(app_path)),
                default_features: None,
                features: vec!["dev".to_string()],
            }),
        );
        if !ctx
            .preview_runtime_features
            .iter()
            .any(|feature| feature == "dynamic_linking")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Preview support runtime features must include waterui/dynamic_linking",
            ));
        }
        let features = BTreeMap::from([(
            "dev".to_string(),
            ctx.preview_runtime_features
                .iter()
                .map(|feature| format!("waterui/{feature}"))
                .collect(),
        )]);
        write_support_cargo_toml(base_dir, ctx.crate_name.as_str(), features, dependencies).await
    }
}

/// Preview-only wrapper templates.
pub mod preview_ffi {
    use cargo_toml::{Dependency, DependencyDetail, Manifest, Package, Product, Workspace};

    use super::{
        NativeBackendDependencyPathKind, PREVIEW_VERSION, Path, TemplateContext, TemplateNamespace,
        WATERUI_FFI_VERSION, cargo_semver, cargo_version_req,
        compute_native_backend_dependency_path, embedded, fs, generated_dev_profile, io,
        scaffold_dir, write_file_if_changed,
    };

    /// Preview ABI exported to Apple support applications.
    pub const APPLE_ABI_FEATURE: &str = "apple-preview-abi";
    /// Preview ABI exported to Android support applications.
    pub const ANDROID_ABI_FEATURE: &str = "android-preview-abi";

    /// Write preview-only wrapper templates to the given directory.
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
        scaffold_dir(
            TemplateNamespace::PreviewFfi,
            &embedded::PREVIEW_FFI,
            base_dir,
            ctx,
        )
        .await
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let mut manifest = Manifest::<()>::default();
        let mut package = Package::new(package_name.to_string(), cargo_semver("0.1.0"));
        package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
        manifest.package = Some(package);
        manifest.profile = generated_dev_profile();

        manifest.lib = Some(Product {
            crate_type: vec!["dylib".to_string()],
            ..Default::default()
        });

        manifest.dependencies.insert(
            ctx.crate_name.to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(ctx.project_root_relative_path()),
                features: vec!["dev".to_string()],
                ..Default::default()
            })),
        );

        let ffi_dependency = ctx.waterui_path.as_ref().map_or_else(
            || DependencyDetail {
                version: Some(cargo_version_req(WATERUI_FFI_VERSION)),
                optional: true,
                default_features: false,
                ..Default::default()
            },
            |waterui_path| DependencyDetail {
                path: Some(compute_native_backend_dependency_path(
                    ctx,
                    waterui_path,
                    NativeBackendDependencyPathKind::WorkspaceSubdir("ffi"),
                )),
                optional: true,
                default_features: false,
                ..Default::default()
            },
        );
        manifest.dependencies.insert(
            "waterui-ffi".to_string(),
            Dependency::Detailed(Box::new(ffi_dependency)),
        );

        let preview_dependency = if let Some(waterui_path) = &ctx.waterui_path {
            let waterui_root = Path::new(&compute_native_backend_dependency_path(
                ctx,
                waterui_path,
                NativeBackendDependencyPathKind::WateruiRoot,
            ))
            .to_path_buf();
            let waterui_root = if waterui_root.is_absolute() {
                waterui_root
            } else {
                base_dir.join(waterui_root)
            };
            let preview_path =
                super::preview::resolve_workspace_member_dir(&waterui_root, "waterui-preview")
                    .await?;
            DependencyDetail {
                path: Some(super::normalize_path_for_config(&preview_path)),
                optional: true,
                ..Default::default()
            }
        } else {
            DependencyDetail {
                version: Some(cargo_version_req(PREVIEW_VERSION)),
                optional: true,
                ..Default::default()
            }
        };
        manifest.dependencies.insert(
            "waterui-preview".to_string(),
            Dependency::Detailed(Box::new(preview_dependency)),
        );

        for (feature, ffi_feature) in [
            (APPLE_ABI_FEATURE, "waterui-ffi/c-api"),
            (ANDROID_ABI_FEATURE, "waterui-ffi/android-jni"),
        ] {
            manifest.features.insert(
                feature.to_string(),
                vec![
                    "dep:waterui-ffi".to_string(),
                    ffi_feature.to_string(),
                    "dep:waterui-preview".to_string(),
                ],
            );
        }

        manifest.workspace = Some(Workspace::default());

        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
        Ok(())
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
        hex::encode(hasher.finalize())
    }

    /// Write inspector app templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        generate_cargo_toml(base_dir, ctx).await?;
        scaffold_dir(
            TemplateNamespace::Inspector,
            &embedded::INSPECTOR,
            base_dir,
            ctx,
        )
        .await
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

        write_support_cargo_toml(
            base_dir,
            ctx.crate_name.as_str(),
            BTreeMap::new(),
            dependencies,
        )
        .await
    }
}

#[cfg(test)]
mod template_digest_tests {
    /// The digest must be stable across calls, or every invocation would
    /// invalidate the generated-crate cache and force a full rebuild.
    #[test]
    fn scaffold_template_digest_is_stable() {
        assert_eq!(
            super::scaffold_template_digest(),
            super::scaffold_template_digest()
        );
    }

    /// It must actually depend on template contents. A digest that ignored them
    /// would let a stale generated crate survive a CLI upgrade — the rot this
    /// exists to prevent.
    #[test]
    fn scaffold_template_digest_covers_template_contents() {
        let digest = super::scaffold_template_digest();
        assert_eq!(digest.len(), 16, "digest must be a 16-char hex prefix");

        let hydrolysis_preview = super::embedded::HYDROLYSIS
            .get_file("src/preview_runtime.rs.tpl")
            .expect("the hydrolysis preview runtime template must be embedded");
        assert!(
            !hydrolysis_preview.contents().is_empty(),
            "the template the digest is meant to track must be non-empty"
        );
    }
}
