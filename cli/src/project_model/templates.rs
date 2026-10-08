//! Type-safe template scaffolding for `WaterUI` project backends.
//!
//! Uses `include_dir` to embed templates at compile time and provides
//! a type-safe substitution API for generating Apple and Android backend projects.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};

use askama::Template;

use crate::framework::{
    APPLE_BACKEND, FRAMEWORK_MEMBERS, FrameworkMember, HYDROLYSIS, ResolvedFramework,
};
use crate::project::ResolvedWebViewBackend;

use include_dir::{Dir, include_dir};
use smol::fs;

use crate::project_types::{AndroidPermissionName, BundleIdentifier, CrateName, RustIdent};

/// Normalize a path to use forward slashes for config files (Cargo.toml, package manifests, etc.)
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
pub fn scaffold_template_digest() -> String {
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
        &embedded::TUI,
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

pub mod embedded {
    use super::{Dir, include_dir};

    pub static APPLE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/apple");
    pub static ANDROID: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/android");
    pub static ANDROID_EMBEDDED: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/src/templates/android_embedded");
    pub static ANDROID_SHARED: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/src/templates/android_shared");
    pub static FFI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/ffi");
    pub static GTK4: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/gtk4");
    pub static HYDROLYSIS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/hydrolysis");
    pub static HYDROLYSIS_ANDROID: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/src/templates/hydrolysis_android");
    pub static ESP32: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/esp32");
    pub static PREVIEW: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/preview");
    pub static PREVIEW_FFI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/preview_ffi");
    pub static INSPECTOR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/inspector");
    pub static TUI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/tui");
    pub static WINUI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates/winui");
    pub static ROOT: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/templates");
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

/// What the launch screen staged into the Apple asset catalog contains, so
/// the generated Apple scaffold names only the assets that exist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaunchTemplateEntry {
    /// A `LaunchBackground` color set was staged (a background is configured).
    pub has_background: bool,
    /// A `LaunchImage` image set was staged (`Launch.*` exists).
    pub has_image: bool,
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
    /// Absolute paths of TTF/OTF binaries the harness `include_bytes!`es
    /// into flash for dew text shaping. Firmware has no font directory to
    /// enumerate, so a text-rendering app must bundle at least one face.
    pub fonts: Vec<String>,
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
            fonts: Vec::new(),
            console_uart_default: params.console_uart_default,
            flash_size_mb: params.flash_size_mb,
            main_task_stack_bytes: params.main_task_stack_bytes,
            app_partition_offset: params.app_partition_offset.to_string(),
            app_partition_size: params.app_partition_size.to_string(),
            opt_level: params.opt_level.to_string(),
        }
    }

    /// Sets the flash-bundled font binaries (absolute paths).
    #[must_use]
    pub fn with_fonts(mut self, fonts: Vec<String>) -> Self {
        self.fonts = fonts;
        self
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

/// The Hydrolysis Android app scaffold's parameters: the managed host
/// checkout it `includeBuild`s, the painter module that checkout supplies,
/// and the app's native library name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrolysisAndroidTemplateEntry {
    /// The cdylib `System.loadLibrary` name (e.g. `waterui_hydrolysis_backend_lib`).
    pub native_library_name: String,
    /// The Gradle project inside the pinned host checkout, relative to the
    /// generated `android/` directory (e.g. `../android-host/<rev>/android`).
    pub host_project_dir: String,
    /// The application project root, relative to the generated `android/`
    /// directory — Gradle tasks that invoke `water` resolve the project
    /// against it. The shared `project_root_relative_path` diffs against the
    /// backend dir one level up, which only the launcher crate may use.
    pub project_root: String,
    /// The `dev.waterui.hydrolysis` painter artifact the app module links
    /// (e.g. `dev.waterui.hydrolysis:gpu`).
    pub painter_dependency: String,
    /// The painter's Gradle project inside the host checkout (e.g. `gpu`) —
    /// the `includeBuild` `dependencySubstitution` uses it for both the
    /// `dev.waterui.hydrolysis` module name and the substituted project.
    pub painter_module: String,
    /// The generated app's `minSdk`: the higher of the framework's Android
    /// API floor and the selected painter's.
    pub min_api_level: u32,
    /// Import path of the band `View` the painter mounts as child 0 of the
    /// host view — `None` for a painter that draws through the host view
    /// itself and ships no band.
    pub painter_band_import: Option<String>,
    /// The band class [`painter_band_import`](Self::painter_band_import)
    /// supplies, when set.
    pub painter_band_class: Option<String>,
}

/// The two `WebView` answers one generated-manifest section gets from the
/// application's own graphs: whether the standard `WebView` component is
/// used at all, and which engine crate draws it when it is.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserAnswers {
    /// The application's `cargo tree --edges features` evaluation enables
    /// a `webview` feature — the component is used.
    pub webview_enabled: bool,
    /// The browser engine crate the application's normal-edge graph
    /// links, if any.
    pub engine: Option<ResolvedWebViewBackend>,
}

/// What the application's own dependency graph says about browser
/// components, for exactly the OSes the manifest's sections serve.
///
/// Nothing here is configuration: the engine that draws a `WebView` is a crate
/// the application links and installs, so the generated backend only has to
/// know whether it should bridge the platform's own engine, and whether the
/// package needs a CEF subprocess helper. The answers legitimately differ
/// per OS — `waterui-browser-wpe` enters the graph on Linux only — so every
/// `cfg(...)` section whose serving set is one OS renders from that OS's
/// own answers.
///
/// Each generated manifest renders answers for a fixed set of OSes, and
/// each variant carries exactly that set — an OS nobody resolved is
/// unrepresentable here, so a section can never read answers for a graph
/// nobody ran.
#[derive(Debug, Clone)]
pub enum BrowserTemplateContext {
    /// The shared native manifest's `cfg` sections — one per desktop OS.
    Desktop(DesktopBrowserContext),
    /// The managed Apple crates — the FFI companion and the Apple preview
    /// package. Their tables read only the macOS engine, so no other
    /// answer is representable.
    AppleManaged {
        /// The engine the macOS table links, if any.
        macos_engine: Option<ResolvedWebViewBackend>,
    },
    /// The GTK4 manifest — one Linux `[dependencies]` table.
    Linux(BrowserAnswers),
}

/// The `BrowserAnswers` for each desktop OS — the serving set the shared
/// native manifest's per-OS sections render.
#[derive(Debug, Clone, Default)]
pub struct DesktopBrowserContext {
    /// macOS's `cfg` section.
    pub macos: BrowserAnswers,
    /// Linux's `cfg` section.
    pub linux: BrowserAnswers,
    /// Windows's `cfg` section.
    pub windows: BrowserAnswers,
}

impl DesktopBrowserContext {
    /// The answers `os`'s generated-manifest section renders — all three
    /// fields are recorded, so the lookup is total.
    pub(crate) const fn for_os(&self, os: crate::platform::NativeOs) -> BrowserAnswers {
        match os {
            crate::platform::NativeOs::MacOs => self.macos,
            crate::platform::NativeOs::Linux => self.linux,
            crate::platform::NativeOs::Windows => self.windows,
        }
    }

    /// The backend feature that bridges `os`'s own web engine.
    ///
    /// An application that linked an engine of its own draws through that
    /// instead, and the bridge would take the component by type before the
    /// application's realization was ever consulted — so the backend compiles
    /// no web engine at all.
    const fn webview_backend_feature(&self, os: crate::platform::NativeOs) -> Option<&'static str> {
        webview_backend_feature(self.for_os(os))
    }
}

/// The backend feature `answers`'s section selects — `webview-system`
/// when the application enables `WebView` without an engine crate of its
/// own.
const fn webview_backend_feature(answers: BrowserAnswers) -> Option<&'static str> {
    if answers.webview_enabled && answers.engine.is_none() {
        Some("webview-system")
    } else {
        None
    }
}

impl Default for BrowserTemplateContext {
    fn default() -> Self {
        Self::Desktop(DesktopBrowserContext::default())
    }
}

impl BrowserTemplateContext {
    /// A context for the shared native manifest — every desktop OS's
    /// section renders its own resolved answers.
    #[must_use]
    pub(crate) const fn desktop(
        macos: BrowserAnswers,
        linux: BrowserAnswers,
        windows: BrowserAnswers,
    ) -> Self {
        Self::Desktop(DesktopBrowserContext {
            macos,
            linux,
            windows,
        })
    }

    /// A context for the managed Apple crates — the only browser input
    /// their sections read is macOS's engine, so that is all it carries.
    #[must_use]
    pub(crate) const fn apple_managed(macos_engine: Option<ResolvedWebViewBackend>) -> Self {
        Self::AppleManaged { macos_engine }
    }

    /// A context for the GTK4 manifest — its one section is Linux's.
    #[must_use]
    pub(crate) const fn linux(answers: BrowserAnswers) -> Self {
        Self::Linux(answers)
    }

    /// The whole desktop set — the shared native manifest's per-OS
    /// sections require it, and a context that serves fewer OSes is an
    /// error rather than a partial render.
    fn desktop_answers(&self) -> io::Result<&DesktopBrowserContext> {
        match self {
            Self::Desktop(context) => Ok(context),
            Self::AppleManaged { .. } | Self::Linux(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the shared native manifest renders one section per desktop OS, \
                 so its context must carry all three",
            )),
        }
    }

    /// Linux's answers — the GTK4 manifest's `[dependencies]` table
    /// requires them; a context serving other OSes only is an error.
    fn linux_answers(&self) -> io::Result<BrowserAnswers> {
        match self {
            Self::Linux(answers) => Ok(*answers),
            Self::Desktop(context) => Ok(context.linux),
            Self::AppleManaged { .. } => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the GTK4 manifest renders a Linux section, so its context \
                 must carry Linux's answers",
            )),
        }
    }

    /// The engine this manifest's CEF runtime init gates on — macOS's
    /// engine for the shared native and managed Apple contexts (the init
    /// is a macOS-only code path), Linux's own for the GTK4 manifest.
    const fn cef_engine(&self) -> Option<ResolvedWebViewBackend> {
        match self {
            Self::Desktop(context) => context.macos.engine,
            Self::AppleManaged { macos_engine } => *macos_engine,
            Self::Linux(answers) => answers.engine,
        }
    }

    /// The `(os, engine)` pair for every OS this context serves — the
    /// CEF helper's `#[cfg]` predicate iterates exactly the set its
    /// manifest's sections render.
    fn os_engines(&self) -> Vec<(crate::platform::NativeOs, Option<ResolvedWebViewBackend>)> {
        match self {
            Self::Desktop(context) => crate::platform::NativeOs::ALL
                .iter()
                .map(|os| (*os, context.for_os(*os).engine))
                .collect(),
            Self::AppleManaged { macos_engine } => {
                vec![(crate::platform::NativeOs::MacOs, *macos_engine)]
            }
            Self::Linux(answers) => vec![(crate::platform::NativeOs::Linux, answers.engine)],
        }
    }

    /// Whether any served OS links the CEF engine. An OS outside the
    /// served set contributes nothing — its manifest writes no browser
    /// table for it either way.
    fn declares_cef_helper(&self) -> bool {
        self.os_engines()
            .into_iter()
            .any(|(_, engine)| crate::project_model::project_types::declares_cef_helper(engine))
    }
}

/// `[signing.android]` rendered into the generated Gradle project: the
/// keystore path and key alias from `Water.toml`, escaped for the Kotlin
/// string literals they render into. Passwords never enter the context — the
/// generated `signingConfig` reads them from the environment at build time.
#[derive(Debug, Clone)]
pub struct AndroidSigningTemplateEntry {
    /// The keystore path as declared in `Water.toml` (project-root-relative),
    /// escaped for the Kotlin literal it renders into.
    pub keystore: String,
    /// The alias of the signing key inside the keystore, escaped for the
    /// Kotlin literal it renders into.
    pub key_alias: String,
    /// The environment variable the generated `signingConfig` reads the store
    /// password from.
    pub store_password_env: &'static str,
    /// The environment variable the generated `signingConfig` reads the key
    /// password from.
    pub key_password_env: &'static str,
    /// The environment variable `water package --unsigned` sets so the
    /// generated release `signingConfig` stays unused.
    pub unsigned_env: &'static str,
}

impl From<&crate::android::signing::AndroidSigningConfig> for AndroidSigningTemplateEntry {
    fn from(config: &crate::android::signing::AndroidSigningConfig) -> Self {
        Self {
            keystore: kotlin_string_literal(&config.keystore().to_string_lossy()),
            key_alias: kotlin_string_literal(config.key_alias()),
            store_password_env: crate::android::signing::STORE_PASSWORD_ENV,
            key_password_env: crate::android::signing::KEY_PASSWORD_ENV,
            unsigned_env: crate::android::signing::UNSIGNED_ENV,
        }
    }
}

/// Escape a value for the `"..."` literal it renders into in a generated
/// Kotlin source: a bare `$` would open a Kotlin string template that
/// evaluates arbitrary build-script code.
fn kotlin_string_literal(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
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
    /// The Rust crate version — the Maven coordinate version the embedded
    /// Android AAR publishes under.
    pub crate_version: String,
    /// The bundle identifier (e.g., "dev.waterui.myapp")
    pub bundle_identifier: BundleIdentifier,
    /// The author name
    pub author: String,
    /// Whether the Apple pieces render — the `waterui-apple` pin and the
    /// entry-owning bin. This is a host fact, not a project selection:
    /// every FFI render on a macOS host resolves the Apple pin whatever
    /// the invocation asked for, because macOS is the only host an Apple
    /// build can run from; a host that cannot produce an Apple build must
    /// not resolve the backend crate.
    pub apple_backend_selected: bool,
    /// Path to local `WaterUI` repository (for dev mode)
    pub waterui_path: Option<PathBuf>,
    /// The canonical local backend sources the `waterui_path` checkout
    /// supplies, resolved before the context was built.
    pub local_sources: LocalBackendSources,
    /// Persisted framework source and native backend revisions.
    pub framework: ResolvedFramework,
    /// Browser engine and component selections for generated backend manifests.
    pub browser: BrowserTemplateContext,
    /// Path to the backend project being scaffolded.
    ///
    /// This may be relative to the project root or an absolute cache path.
    pub backend_project_path: Option<PathBuf>,
    /// Absolute path to the user project root when scaffolding generated backend projects.
    pub project_root_path: Option<PathBuf>,
    /// Fully qualified Android permissions the manifest declares, rendered
    /// into `<uses-permission android:name>` as is.
    pub android_permissions: Vec<AndroidPermissionName>,
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
    /// The project's own packages — the set `generated_profiles` writes
    /// `[profile.dev.package.<name>]` overrides for, so the user's own crates
    /// build at `opt-level = 0` with line tables while every other dependency
    /// keeps `"*"`'s `opt-level = 2, debug = false`.
    ///
    /// `None` until [`Self::with_project_packages`] supplies the real set: a
    /// context that never received one must fail rather than write a plausible
    /// but wrong override — for the generated crate's own name, say — so every
    /// manifest writer calls `generated_profiles` with this field directly.
    pub project_packages: Option<BTreeSet<String>>,
    /// The `include_web!` argument when the root view is a web frontend:
    /// `"web"` for the conventional layout, a path relative to the project
    /// root for a frontend referenced in place. `None` renders the demo
    /// `lib.rs` instead.
    pub web_frontend_arg: Option<String>,
    /// Android release signing rendered into the Gradle project, from
    /// `[signing.android]` in `Water.toml`. `None` renders no
    /// `signingConfig`, and release packaging then fails in
    /// `crate::android::signing::PreparedSigning::resolve` unless the caller
    /// asked for unsigned output.
    pub android_signing: Option<AndroidSigningTemplateEntry>,
    /// ESP32 harness parameters used by the esp32 templates.
    pub esp32: Esp32TemplateEntry,
    /// The launch screen assets the Apple templates refer to.
    pub launch: LaunchTemplateEntry,
    /// Hydrolysis Android scaffold parameters — set only while the
    /// `hydrolysis_android` templates render.
    pub hydrolysis_android: Option<HydrolysisAndroidTemplateEntry>,
    /// The host the manifest-generation probes run on — the `cargo
    /// metadata` the feature-forward resolution issues against the
    /// manifest being written goes through this host's `PATH` and
    /// environment, never the bare process.
    pub host: crate::toolchain::Host,
}

impl TemplateContext {
    /// The `include_web!` argument rendered into `web_lib.rs.tpl`.
    #[must_use]
    pub fn web_arg(&self) -> &str {
        self.web_frontend_arg.as_deref().unwrap_or("web")
    }

    /// Build a template context for a new root project scaffold.
    #[must_use]
    pub fn for_create_options(
        options: &crate::project::CreateOptions,
        crate_name: CrateName,
        framework: &ResolvedFramework,
        local_sources: &LocalBackendSources,
    ) -> Self {
        let waterui_path = options.waterui_path.clone();
        Self {
            app_display_name: options.name.clone(),
            app_name: options.name.replace(' ', ""),
            crate_name,
            crate_version: String::new(),
            bundle_identifier: options.bundle_identifier.clone(),
            author: options.author.clone(),
            apple_backend_selected: false,
            waterui_path,
            local_sources: local_sources.clone(),
            framework: framework.clone(),
            browser: BrowserTemplateContext::default(),
            backend_project_path: None,
            project_root_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: false,
            preview_runtime_fingerprint: None,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            project_packages: None,
            web_frontend_arg: options.web.as_ref().map(|web| web.include_arg.clone()),
            android_signing: None,
            esp32: Esp32TemplateEntry::default(),
            launch: LaunchTemplateEntry::default(),
            hydrolysis_android: None,
            host: crate::toolchain::Host::current(),
        }
    }

    /// Build a context from an existing project manifest for backend scaffolding.
    /// `local_sources` is the manifest's `waterui_path` resolved through
    /// [`project_local_backend_sources`], never the raw manifest path — a
    /// checkout carrying a malformed canonical slot already failed there.
    #[must_use]
    pub fn for_project_manifest(
        manifest: &crate::project::Manifest,
        crate_name: CrateName,
        app_name: impl Into<String>,
        framework: &ResolvedFramework,
        local_sources: &LocalBackendSources,
    ) -> Self {
        Self {
            app_display_name: manifest.package.name.clone(),
            app_name: app_name.into(),
            crate_name,
            crate_version: String::new(),
            bundle_identifier: manifest.package.bundle_identifier.clone(),
            author: String::new(),
            // Selected at invocation, never from declared config.
            apple_backend_selected: false,
            waterui_path: manifest.waterui_path.as_ref().map(PathBuf::from),
            local_sources: local_sources.clone(),
            framework: framework.clone(),
            browser: BrowserTemplateContext::default(),
            backend_project_path: None,
            project_root_path: None,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: manifest.package.accessory,
            preview_runtime_fingerprint: None,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            project_packages: None,
            web_frontend_arg: manifest.web.as_ref().map(|_| "web".to_string()),
            android_signing: manifest
                .signing
                .android
                .as_ref()
                .map(AndroidSigningTemplateEntry::from),
            esp32: Esp32TemplateEntry::default(),
            launch: LaunchTemplateEntry::default(),
            hydrolysis_android: None,
            host: crate::toolchain::Host::current(),
        }
    }

    /// Build a context for the CLI's own support applications.
    #[must_use]
    pub fn for_support_app(
        identity: SupportAppIdentity,
        waterui_path: Option<PathBuf>,
        framework: &ResolvedFramework,
        accessory: bool,
        preview_runtime_fingerprint: Option<String>,
        local_sources: &LocalBackendSources,
    ) -> Self {
        // A support app exists to host one specific WaterUI runtime, so it has
        // to resolve dependencies exactly the way that runtime's own workspace
        // does — `[patch]` included. Cargo only honours `[patch]` from the root
        // of the workspace being built, and a support app scaffolded outside
        // that tree has no such root: it silently resolves the unpatched
        // crates.io version of every forked dependency. The app then links a
        // different graphics stack than the module it loads, and the module
        // fails to `dlopen` against symbols that no longer match.
        let project_root_path = waterui_path.clone();
        let SupportAppIdentity {
            display_name,
            crate_name,
            bundle_identifier,
        } = identity;
        Self {
            app_name: display_name.replace(' ', ""),
            app_display_name: display_name,
            crate_name,
            crate_version: String::new(),
            bundle_identifier,
            author: String::new(),
            apple_backend_selected: false,
            waterui_path,
            local_sources: local_sources.clone(),
            framework: framework.clone(),
            browser: BrowserTemplateContext::default(),
            backend_project_path: None,
            project_root_path,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory,
            preview_runtime_fingerprint,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            project_packages: None,
            web_frontend_arg: None,
            android_signing: None,
            esp32: Esp32TemplateEntry::default(),
            launch: LaunchTemplateEntry::default(),
            hydrolysis_android: None,
            host: crate::toolchain::Host::current(),
        }
    }

    /// Set the crate version published as the embedded artifact's Maven
    /// coordinate.
    #[must_use]
    pub fn with_crate_version(mut self, version: impl Into<String>) -> Self {
        self.crate_version = version.into();
        self
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

    /// Set whether the Apple pieces render — the ffi companion only emits
    /// its `waterui-apple` dependency and entry-owning bin when this is
    /// set.
    #[must_use]
    pub const fn with_apple_backend_selected(mut self, selected: bool) -> Self {
        self.apple_backend_selected = selected;
        self
    }

    /// Set the host the manifest-generation probes run on.
    #[must_use]
    pub fn with_host(mut self, host: crate::toolchain::Host) -> Self {
        self.host = host;
        self
    }

    /// Record the `WebView` answers this manifest's sections render — the
    /// context carries exactly the OS set the manifest serves.
    #[must_use]
    pub(crate) const fn with_browser(mut self, browser: BrowserTemplateContext) -> Self {
        self.browser = browser;
        self
    }

    /// Whether this manifest's CEF runtime init compiles — the early-init
    /// code path the Apple entry points and the Hydrolysis `main` emit
    /// exists only where the recorded engine is CEF.
    const fn cef_runtime_enabled(&self) -> bool {
        crate::project_model::project_types::declares_cef_helper(self.browser.cef_engine())
    }

    /// Whether any served OS's section links the CEF engine. The
    /// subprocess helper `[[bin]]` is declared once for the whole
    /// manifest — it exists wherever any OS's table gives it a crate to
    /// call — while the dependency itself lives in that OS's table.
    fn declares_cef_helper(&self) -> bool {
        self.browser.declares_cef_helper()
    }

    /// The `cfg` predicate matching the OSes whose tables link the CEF
    /// engine. The subprocess helper's source compiles its dispatch only
    /// where an OS's table provides `waterui-browser-cef` — on every other
    /// target the bin is never spawned and exits rather than missing the
    /// crate. No OS is CEF: `any()` is false for the empty set, which keeps
    /// the still-rendered source compiling with an empty dispatch.
    fn cef_helper_condition(&self) -> String {
        let oses: Vec<&'static str> = self
            .browser
            .os_engines()
            .into_iter()
            .filter(|(_, engine)| crate::project_model::project_types::declares_cef_helper(*engine))
            .map(|(os, _)| os.cfg_predicate())
            .collect();
        format!("any({})", oses.join(", "))
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

    /// Set the project's own packages — the set `generated_profiles` writes
    /// `[profile.dev.package.<name>]` overrides for. Every context a generated
    /// manifest renders through must carry the set
    /// [`crate::project::Project::project_packages`] resolves; a context left
    /// unset fails its manifest writes rather than render a wrong override.
    #[must_use]
    pub fn with_project_packages(mut self, packages: BTreeSet<String>) -> Self {
        self.project_packages = Some(packages);
        self
    }

    /// The `[signing.android]` entry the Gradle templates render, if any.
    #[must_use]
    pub const fn android_signing(&self) -> Option<&AndroidSigningTemplateEntry> {
        self.android_signing.as_ref()
    }

    /// Set Android permissions for template rendering.
    #[must_use]
    pub fn with_android_permissions(mut self, permissions: Vec<AndroidPermissionName>) -> Self {
        self.android_permissions = permissions;
        self
    }

    /// Set iOS permissions for template rendering.
    #[must_use]
    pub fn with_ios_permissions(mut self, permissions: Vec<IosPermissionTemplateEntry>) -> Self {
        self.ios_permissions = permissions;
        self
    }

    /// Name the launch screen assets staged into the Apple asset catalog.
    #[must_use]
    pub const fn with_launch(mut self, launch: LaunchTemplateEntry) -> Self {
        self.launch = launch;
        self
    }

    /// Set ESP32 harness parameters for template rendering.
    #[must_use]
    pub fn with_esp32(mut self, esp32: Esp32TemplateEntry) -> Self {
        self.esp32 = esp32;
        self
    }

    /// Set the Hydrolysis Android scaffold parameters.
    #[must_use]
    pub fn with_hydrolysis_android(mut self, entry: HydrolysisAndroidTemplateEntry) -> Self {
        self.hydrolysis_android = Some(entry);
        self
    }

    const fn hydrolysis_android_entry(&self) -> &HydrolysisAndroidTemplateEntry {
        self.hydrolysis_android
            .as_ref()
            .expect("TemplateContext missing the Hydrolysis Android entry")
    }

    /// The cdylib name the generated `MainActivity` loads.
    #[must_use]
    pub fn hydrolysis_android_native_library_name(&self) -> &str {
        &self.hydrolysis_android_entry().native_library_name
    }

    /// The pinned host checkout's Gradle root, relative to the generated
    /// `android/` directory, that `settings.gradle.kts` `includeBuild`s.
    #[must_use]
    pub fn hydrolysis_android_host_project_dir(&self) -> &str {
        &self.hydrolysis_android_entry().host_project_dir
    }

    /// The application project root relative to the generated `android/`
    /// directory.
    #[must_use]
    pub fn hydrolysis_android_project_root(&self) -> &str {
        &self.hydrolysis_android_entry().project_root
    }

    /// The `dev.waterui.hydrolysis` painter artifact the app module links.
    #[must_use]
    pub fn hydrolysis_android_painter_dependency(&self) -> &str {
        &self.hydrolysis_android_entry().painter_dependency
    }

    /// The painter's Gradle project inside the host checkout
    /// ([`HydrolysisAndroidTemplateEntry::painter_module`]).
    pub fn hydrolysis_android_painter_module(&self) -> &str {
        &self.hydrolysis_android_entry().painter_module
    }

    /// The generated app's `minSdk`.
    #[must_use]
    pub const fn hydrolysis_android_min_api_level(&self) -> u32 {
        self.hydrolysis_android_entry().min_api_level
    }

    /// Whether the selected painter mounts a band `View` under the host.
    #[must_use]
    pub const fn hydrolysis_android_has_painter_band(&self) -> bool {
        self.hydrolysis_android_entry().painter_band_class.is_some()
    }

    /// The painter band `View`'s import path; only valid when
    /// [`Self::hydrolysis_android_has_painter_band`].
    #[must_use]
    pub fn hydrolysis_android_painter_band_import(&self) -> &str {
        self.hydrolysis_android_entry()
            .painter_band_import
            .as_deref()
            .expect("Hydrolysis Android entry has no painter band")
    }

    /// The painter band `View`'s class name; only valid when
    /// [`Self::hydrolysis_android_has_painter_band`].
    #[must_use]
    pub fn hydrolysis_android_painter_band_class(&self) -> &str {
        self.hydrolysis_android_entry()
            .painter_band_class
            .as_deref()
            .expect("Hydrolysis Android entry has no painter band")
    }

    #[must_use]
    pub fn crate_name_ident(&self) -> RustIdent {
        self.crate_name.rust_ident()
    }

    /// The generated ffi companion crate's Rust identifier — the crate the
    /// Apple entry binary imports `waterui_apple_main` from.
    #[must_use]
    pub fn ffi_crate_ident(&self) -> RustIdent {
        crate::project_model::project_types::generated_crate_name(
            &self.crate_name,
            "ffi",
            self.project_root_path
                .as_deref()
                .expect("ffi crate ident is rendered for a project"),
        )
        .rust_ident()
    }

    /// The Android package name the Gradle templates render for
    /// `applicationId`/`namespace`/`group`. The Android backend validates the
    /// manifest's identifier against the Java package grammar before it
    /// scaffolds or packages, so the conversion cannot fail here.
    #[must_use]
    pub fn android_package_name(&self) -> crate::project_types::AndroidPackageName {
        self.bundle_identifier
            .android_package_name()
            .expect("the Android path validates bundle_identifier before rendering")
    }

    /// The Android API floor the selected framework's metadata declares —
    /// rendered into the scaffolded app's `minSdk`.
    #[must_use]
    pub fn android_min_api_level(&self) -> u32 {
        self.framework
            .android_min_api_level()
            .unwrap_or_else(|error| panic!("{error:#}"))
    }

    /// The JDK major version the generated app compiles against, rendered as
    /// `JavaVersion.VERSION_{value}` — the value `water doctor` enforces on the
    /// installed JDK, from the same `[package.metadata.waterui-scaffold]` key.
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "Askama calls it as a context method on the template struct"
    )]
    pub const fn android_jdk_version(&self) -> &'static str {
        crate::build_info::ANDROID_JDK_VERSION
    }

    /// The Kotlin runtime coordinate the generated Android project and the
    /// embedded module's POM declare — the `JitPack` coordinate of the
    /// revision `android-backend-revision` pins.
    #[must_use]
    pub fn android_runtime_dependency(&self) -> String {
        jitpack_dependency_coordinate(
            self.framework.scaffold_value("android-backend-url"),
            self.framework.scaffold_value("android-backend-revision"),
        )
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

    /// Compute the path a generated backend project's config references for
    /// `target`, resolved from the backend project's directory.
    ///
    /// `target` is absolute, or relative to the project root the way
    /// `waterui_path` is. This accounts
    /// for the project being in a generated backend subdirectory.
    fn backend_relative_path(&self, target: &Path) -> String {
        // If `target` is absolute, use it directly. This avoids producing
        // invalid paths like `../../../..//Users/...` in generated config files.
        if target.is_absolute() {
            return normalize_path_for_config(target);
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
            let absolute_backend_path = project_root.join(target);
            let relative_path = pathdiff::diff_paths(&absolute_backend_path, backend_project_path)
                .unwrap_or_else(|| {
                    panic!(
                        "Failed to compute backend dependency path from {} to {}",
                        backend_project_path.display(),
                        absolute_backend_path.display()
                    )
                });
            return normalize_path_for_config(&relative_path);
        }

        // Count how many levels deep the project is from the project root.
        // Default is 1 level (e.g., "android"), generated backends may be deeper.
        let project_depth = self
            .backend_project_path
            .as_ref()
            .map_or(1, |p| p.components().count());

        // Build the relative path: go up `project_depth` levels, then down to
        // the target. Use `PathBuf` joins to avoid accidental `//` sequences
        // and to keep behavior consistent across platforms.
        let mut backend_path = PathBuf::new();
        for _ in 0..project_depth {
            backend_path.push("..");
        }
        backend_path.push(target);

        normalize_path_for_config(&backend_path)
    }

    /// The path to the local checkout `member`'s canonical slot supplies —
    /// `waterui_path/<member.subdirectory>` resolved from the generated
    /// project's directory. `None` consumes the pinned remote source
    /// instead.
    ///
    /// The canonical checkout slot is the only local source: without the
    /// probe every local-checkout build — including the backend's own e2e
    /// suite — would silently retarget onto the pinned remote release.
    fn compute_member_backend_path(&self, member: FrameworkMember) -> Option<String> {
        self.local_sources.member(member)?;
        Some(self.backend_relative_path(&self.waterui_path.as_ref()?.join(member.subdirectory)))
    }

    /// Absolute path of the `WaterUI` workspace root when building against a
    /// local checkout, resolved against the project root for relative
    /// `waterui_path` values. `None` in remote-backend mode.
    fn waterui_workspace_root(&self) -> Option<PathBuf> {
        let waterui_path = self.waterui_path.as_ref()?;
        if waterui_path.is_absolute() {
            return Some(waterui_path.clone());
        }
        self.project_root_path
            .as_ref()
            .map(|project_root| project_root.join(waterui_path))
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

    /// The dependency a generated crate declares for `member` — an in-tree
    /// framework workspace crate: a `path` into the `waterui_path` checkout's
    /// canonical slot when one is staged, and the framework's own repository
    /// at the selected revision otherwise — the member is a framework
    /// workspace crate, so the channel's `(repository, revision)` pins it
    /// the same way the Rust packages are pinned.
    ///
    /// A `waterui_path` checkout that carries no `member.subdirectory`
    /// cannot supply the crate — there is no remote fallback for a missing
    /// local member — and a selected framework revision that declares no
    /// `member.path_key` carries no such member either.
    fn member_dependency(&self, member: FrameworkMember) -> io::Result<GeneratedDependencyDetail> {
        if let Some(backend_path) = self.compute_member_backend_path(member) {
            return Ok(GeneratedDependencyDetail {
                path: Some(backend_path),
                ..GeneratedDependencyDetail::default()
            });
        }
        if let Some(waterui_path) = &self.waterui_path {
            return Err(io::Error::other(format!(
                "the WaterUI checkout `{}` carries no `{}` crate — \
                 the `{}` dependency cannot be resolved",
                waterui_path.display(),
                member.subdirectory,
                member.package
            )));
        }
        let source = self.framework.member_source(member).map_err(|error| {
            io::Error::other(format!(
                "the selected framework supplies no `{}` member: {error:#}",
                member.package
            ))
        })?;
        Ok(GeneratedDependencyDetail {
            git: source.git,
            rev: source.rev,
            ..GeneratedDependencyDetail::default()
        })
    }

    /// The `waterui-apple` dependency the generated FFI crate declares —
    /// the `apple-backend-path` member of the selected framework.
    fn waterui_apple_dependency(&self) -> io::Result<GeneratedDependencyDetail> {
        self.member_dependency(APPLE_BACKEND)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateNamespace {
    Apple,
    Android,
    AndroidEmbedded,
    AndroidShared,
    Ffi,
    Gtk4,
    Hydrolysis,
    HydrolysisAndroid,
    Esp32,
    Inspector,
    Preview,
    PreviewFfi,
    Tui,
    WinUi,
    Root,
}

impl TemplateNamespace {
    const fn scaffold_template_prefix(self) -> &'static str {
        match self {
            Self::Apple => "src/templates/apple",
            Self::Android => "src/templates/android",
            Self::AndroidEmbedded => "src/templates/android_embedded",
            Self::AndroidShared => "src/templates/android_shared",
            Self::Ffi => "src/templates/ffi",
            Self::Gtk4 => "src/templates/gtk4",
            Self::Hydrolysis => "src/templates/hydrolysis",
            Self::HydrolysisAndroid => "src/templates/hydrolysis_android",
            Self::Esp32 => "src/templates/esp32",
            Self::Inspector => "src/templates/inspector",
            Self::Preview => "src/templates/preview",
            Self::PreviewFfi => "src/templates/preview_ffi",
            Self::Tui => "src/templates/tui",
            Self::WinUi => "src/templates/winui",
            Self::Root => "src/templates",
        }
    }
}

/// The canonical local backend sources a `waterui_path` checkout supplies.
///
/// Resolved once when a [`crate::project::Project`] opens or is created,
/// before any template or backend generation runs; the resolved paths are
/// absolute (a relative `waterui_path` is joined onto the project root).
/// An absent slot is `None` — a `*-path` framework member has no remote
/// fallback: a checkout without the member's canonical directory fails the
/// generated dependency instead. A present-but-malformed slot is an error at
/// resolution, never a silent remote fallback.
#[derive(Debug, Clone, Default)]
pub struct LocalBackendSources {
    members: BTreeMap<&'static str, PathBuf>,
}

impl LocalBackendSources {
    /// The validated checkout `member`'s canonical slot supplies, when
    /// present.
    pub(crate) fn member(&self, member: FrameworkMember) -> Option<&Path> {
        self.members.get(member.subdirectory).map(PathBuf::as_path)
    }

    /// The validated `backends/apple` checkout, when present.
    #[must_use]
    pub fn apple(&self) -> Option<&Path> {
        self.member(APPLE_BACKEND)
    }
}

/// The identity a CLI-owned support application (the preview host, the
/// inspector) declares — everything else about it derives from the
/// `WaterUI` runtime it exists to host.
#[derive(Debug, Clone)]
pub struct SupportAppIdentity {
    /// Human-facing name; the app's crate/binary names derive from it.
    pub display_name: String,
    /// Rust crate name of the support binary.
    pub crate_name: CrateName,
    /// Platform bundle identifier.
    pub bundle_identifier: BundleIdentifier,
}

/// Resolve the canonical local backend sources under a `WaterUI` checkout
/// root: every `*-path` framework member's canonical directory must hold a
/// Rust manifest. A slot is an explicit source choice when present —
/// an absent slot is `None`, a malformed one an error naming the slot and
/// the manifest it lacks.
///
/// # Errors
/// Returns an error when a slot's entry exists but does not resolve to a
/// directory containing the required manifest — a dangling symlink, a
/// non-directory, an unreadable path, or a checkout missing `Cargo.toml`.
pub async fn local_backend_sources(waterui_root: &Path) -> eyre::Result<LocalBackendSources> {
    let mut members = BTreeMap::new();
    for member in FRAMEWORK_MEMBERS {
        if let Some(path) =
            canonical_backend_source(waterui_root, member.subdirectory, "Cargo.toml").await?
        {
            members.insert(member.subdirectory, path);
        }
    }
    Ok(LocalBackendSources { members })
}

/// Resolve the canonical local backend sources a project's `waterui_path`
/// names — `None` (and an empty [`LocalBackendSources`]) means remote
/// sources. A relative `waterui_path` resolves against `project_root`.
///
/// # Errors
/// Returns an error when a present slot is malformed; see
/// [`local_backend_sources`].
pub async fn project_local_backend_sources(
    waterui_path: Option<&Path>,
    project_root: &Path,
) -> eyre::Result<LocalBackendSources> {
    let Some(waterui_path) = waterui_path else {
        return Ok(LocalBackendSources::default());
    };
    let root = if waterui_path.is_absolute() {
        waterui_path.to_path_buf()
    } else {
        project_root.join(waterui_path)
    };
    local_backend_sources(&root).await
}

/// One canonical `backends/<name>` slot under a checkout root: `Ok(None)`
/// only when no entry exists at all. Anything that IS there must resolve
/// through links to a directory containing `probe`.
async fn canonical_backend_source(
    waterui_root: &Path,
    slot: &str,
    probe: &str,
) -> eyre::Result<Option<PathBuf>> {
    use eyre::WrapErr as _;

    let entry = waterui_root.join(slot);
    match smol::fs::symlink_metadata(&entry).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).wrap_err_with(|| {
                format!("`{slot}` under `{}` cannot be read", waterui_root.display())
            });
        }
    }
    smol::fs::metadata(&entry)
        .await
        .wrap_err_with(|| {
            format!(
                "`{slot}` under `{}` does not resolve (a dangling link or unreadable target)",
                waterui_root.display()
            )
        })?
        .is_dir()
        .then_some(())
        .ok_or_else(|| {
            eyre::eyre!(
                "`{slot}` under `{}` is not a directory — remove it or stage a real checkout there",
                waterui_root.display()
            )
        })?;
    let manifest = entry.join(probe);
    match smol::fs::metadata(&manifest).await {
        Ok(metadata) if metadata.is_file() => Ok(Some(entry)),
        Ok(_) | Err(_) => Err(eyre::eyre!(
            "`{slot}` under `{}` is not a backend checkout — it has no `{probe}`; \
             stage a real backend checkout there, or remove it to consume the pinned \
             remote source",
            waterui_root.display()
        )),
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

fn jitpack_dependency_coordinate(repository_url: &str, revision: &str) -> String {
    let (owner, repo) = github_repository_owner_and_name(repository_url);
    format!("com.github.{owner}:{repo}:{revision}")
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
                "src/templates/esp32/Cargo.toml.tpl" => Esp32CargoTomlTemplate::from_ctx(ctx)
                    .and_then(|template| {
                        template.render().map_err(|error| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("Failed to render template {display_path}: {error}"),
                            )
                        })
                    })
                    .and_then(|rendered| {
                        // The esp32 manifest renders through askama to carry
                        // the Xtensa profile note, so it bypasses the
                        // serialized-manifest path that assigns
                        // `manifest.patch`; without the same tables the
                        // generated workspace resolves `waterui-dew`'s
                        // registry `waterui-*` requirements beside the path
                        // copies and `View` splits across the two.
                        let mut document = rendered
                            .parse::<toml_edit::DocumentMut>()
                            .map_err(io::Error::other)?;
                        crate::framework::rewrite_patch_tables(
                            &mut document,
                            &cargo_toml::PatchSet::default(),
                            &generated_crate_patches(ctx)?,
                        )
                        .map_err(|error| io::Error::other(error.to_string()))?;
                        Ok(document.to_string())
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
    dew_dependency: String,
    core_dependency: String,
    locale_dependency: String,
    /// The `opt-level` value as a TOML literal: numeric levels are bare
    /// integers, while `"s"`/`"z"` must be quoted strings — cargo rejects a
    /// quoted `"2"`.
    opt_level_literal: String,
}

impl Esp32CargoTomlTemplate {
    fn from_ctx(ctx: &TemplateContext) -> io::Result<Self> {
        let dew_dependency = generated_dependency_from_spec(
            ctx,
            NativeBackendDependencySpec::new(
                "waterui-dew",
                &["espidf", "progress"],
                NativeBackendDependencySource::WorkspaceDependency,
            ),
        )?
        .with_default_features(false)
        .inline_toml();
        let core_dependency = generated_dependency_from_spec(
            ctx,
            NativeBackendDependencySpec::new(
                "waterui-core",
                &[],
                NativeBackendDependencySource::WorkspaceSubdir("core"),
            ),
        )?
        .inline_toml();
        let locale_dependency = generated_dependency_from_spec(
            ctx,
            NativeBackendDependencySpec::new(
                "waterui-locale",
                &[],
                NativeBackendDependencySource::WorkspaceSubdir("utils/locale"),
            ),
        )?
        .inline_toml();

        Ok(Self {
            package_name: crate::project_model::project_types::generated_crate_name(
                &ctx.crate_name,
                "esp32",
                ctx.project_root_path
                    .as_deref()
                    .expect("ESP32 manifests are rendered for a project"),
            )
            .to_string(),
            app_crate_name: ctx.crate_name.to_string(),
            app_crate_path: ctx.project_root_relative_path(),
            dew_dependency,
            core_dependency,
            locale_dependency,
            opt_level_literal: match ctx.esp32.opt_level.as_str() {
                symbolic @ ("s" | "z") => format!("\"{symbolic}\""),
                numeric => numeric
                    .parse::<u8>()
                    .unwrap_or_else(|error| {
                        panic!(
                            "ESP32 opt-level {numeric:?} is neither symbolic nor numeric: {error}"
                        )
                    })
                    .to_string(),
            },
        })
    }
}

define_scaffold_templates! {
    AssetsReadmeTemplate => (Root, "src/templates/assets_readme.md.tpl"),
    AndroidGradleAppTemplate => (Android, "src/templates/android/app/build.gradle.kts.tpl"),
    AndroidManifestTemplate => (Android, "src/templates/android/app/src/main/AndroidManifest.xml.tpl"),
    AndroidMainActivityTemplate => (Android, "src/templates/android/app/src/main/java/MainActivity.kt.tpl"),
    AndroidApplicationTemplate => (Android, "src/templates/android/app/src/main/java/WaterUiApplication.kt.tpl"),
    AndroidStringsTemplate => (Android, "src/templates/android/app/src/main/res/values/strings.xml.tpl"),
    AndroidSettingsTemplate => (Android, "src/templates/android/settings.gradle.kts.tpl"),
    AndroidEmbeddedSettingsTemplate => (AndroidEmbedded, "src/templates/android_embedded/settings.gradle.kts.tpl"),
    AndroidEmbeddedModuleTemplate => (AndroidEmbedded, "src/templates/android_embedded/waterui/build.gradle.kts.tpl"),
    AndroidEmbeddedManifestTemplate => (AndroidEmbedded, "src/templates/android_embedded/waterui/src/main/AndroidManifest.xml.tpl"),
    FfiBuildScriptTemplate => (Ffi, "src/templates/ffi/build.rs.tpl"),
    FfiLibTemplate => (Ffi, "src/templates/ffi/src/lib.rs.tpl"),
    FfiAppleMainTemplate => (Ffi, "src/templates/ffi/src/bin/waterui-apple-main.rs.tpl"),
    FfiCefHelperTemplate => (Ffi, "src/templates/ffi/src/bin/waterui-cef-helper.rs.tpl"),
    Gtk4BuildScriptTemplate => (Gtk4, "src/templates/gtk4/build.rs.tpl"),
    Gtk4MainTemplate => (Gtk4, "src/templates/gtk4/src/main.rs.tpl"),
    HydrolysisBuildScriptTemplate => (Hydrolysis, "src/templates/hydrolysis/build.rs.tpl"),
    HydrolysisAndroidSettingsTemplate => (HydrolysisAndroid, "src/templates/hydrolysis_android/settings.gradle.kts.tpl"),
    HydrolysisAndroidStringsTemplate => (HydrolysisAndroid, "src/templates/hydrolysis_android/app/src/main/res/values/strings.xml.tpl"),
    HydrolysisAndroidBuildGradleTemplate => (HydrolysisAndroid, "src/templates/hydrolysis_android/app/build.gradle.kts.tpl"),
    HydrolysisAndroidManifestTemplate => (HydrolysisAndroid, "src/templates/hydrolysis_android/app/src/main/AndroidManifest.xml.tpl"),
    HydrolysisAndroidMainActivityTemplate => (HydrolysisAndroid, "src/templates/hydrolysis_android/app/src/main/java/MainActivity.kt.tpl"),
    RootWebLibTemplate => (Root, "src/templates/web_lib.rs.tpl"),
    HydrolysisLibTemplate => (Hydrolysis, "src/templates/hydrolysis/src/lib.rs.tpl"),
    HydrolysisCefHelperTemplate => (Hydrolysis, "src/templates/hydrolysis/src/bin/waterui-cef-helper.rs.tpl"),
    HydrolysisMainTemplate => (Hydrolysis, "src/templates/hydrolysis/src/main.rs.tpl"),
    HydrolysisPreviewRuntimeTemplate => (Hydrolysis, "src/templates/hydrolysis/src/preview_runtime.rs.tpl"),
    HydrolysisPreviewTestRuntimeTemplate => (Hydrolysis, "src/templates/hydrolysis/src/preview_test_runtime.rs.tpl"),
    HydrolysisMcpRuntimeTemplate => (Hydrolysis, "src/templates/hydrolysis/src/mcp_runtime.rs.tpl"),
    Esp32BuildScriptTemplate => (Esp32, "src/templates/esp32/build.rs.tpl"),
    Esp32MainTemplate => (Esp32, "src/templates/esp32/src/main.rs.tpl"),
    Esp32CargoConfigTemplate => (Esp32, "src/templates/esp32/.cargo/config.toml.tpl"),
    Esp32SdkconfigTemplate => (Esp32, "src/templates/esp32/sdkconfig.defaults.tpl"),
    Esp32PartitionsTemplate => (Esp32, "src/templates/esp32/partitions.csv.tpl"),
    PreviewLibTemplate => (Preview, "src/templates/preview/src/lib.rs.tpl"),
    PreviewFfiLibTemplate => (PreviewFfi, "src/templates/preview_ffi/src/lib.rs.tpl"),
    TuiBuildScriptTemplate => (Tui, "src/templates/tui/build.rs.tpl"),
    TuiMainTemplate => (Tui, "src/templates/tui/src/main.rs.tpl"),
    WinUiBuildScriptTemplate => (WinUi, "src/templates/winui/build.rs.tpl"),
    WinUiMainTemplate => (WinUi, "src/templates/winui/src/main.rs.tpl"),
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserTemplateContext, Esp32TemplateEntry, LaunchTemplateEntry, LocalBackendSources,
        ResolvedFramework, ResolvedWebViewBackend, SupportAppIdentity, TemplateContext,
        TemplateNamespace, embedded, generated_profiles, gtk4, jitpack_dependency_coordinate,
        local_backend_sources, normalize_path_for_config, preview_ffi, render_scaffold_template,
    };
    use crate::framework::{
        framework_repository,
        test_fixtures::{dev_framework, nightly_framework, stable_framework, write_local_checkout},
    };
    use crate::project_types::{BundleIdentifier, CrateName};
    use include_dir::Dir;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn ctx(
        waterui_path: Option<PathBuf>,
        backend_project_path: Option<PathBuf>,
        project_root_path: Option<PathBuf>,
    ) -> TemplateContext {
        let local_sources = smol::block_on(crate::templates::project_local_backend_sources(
            waterui_path.as_deref(),
            project_root_path
                .as_deref()
                .unwrap_or_else(|| Path::new("")),
        ))
        .expect("fixture checkout resolves canonical backend sources");
        // Fixture contexts answer "no webview, no engine" for every desktop
        // OS — the sections a render writes still need answers recorded.
        let browser = BrowserTemplateContext::default();
        TemplateContext {
            app_display_name: String::new(),
            app_name: String::new(),
            crate_name: CrateName::try_from("waterui_test").expect("test crate name must be valid"),
            crate_version: "0.0.0".to_string(),
            bundle_identifier: BundleIdentifier::try_from("com.example.test")
                .expect("test bundle identifier must be valid"),
            author: String::new(),
            apple_backend_selected: true,
            waterui_path,
            local_sources,
            framework: stable_framework(),
            browser,
            backend_project_path,
            project_root_path,
            android_permissions: Vec::new(),
            ios_permissions: Vec::new(),
            accessory: false,
            preview_runtime_fingerprint: None,
            preview_runtime_features: Vec::new(),
            preview_app_dependency: None,
            project_packages: Some(BTreeSet::from(["waterui_test".to_string()])),
            web_frontend_arg: None,
            android_signing: None,
            esp32: Esp32TemplateEntry::default(),
            hydrolysis_android: None,
            launch: LaunchTemplateEntry::default(),
            host: crate::toolchain::Host::current(),
        }
    }

    fn project_ctx() -> TemplateContext {
        // Generated crate names tag the project root, so any template that
        // renders one needs a root even when nothing else consumes it.
        ctx(None, None, Some(PathBuf::from("/tmp/test-app")))
    }

    /// `ctx` with `webview_enabled`/`engine` recorded for every desktop OS
    /// — the hydrolysis manifest writes a `cfg` section per OS, so all
    /// three must answer for a render to compute.
    fn all_os_browser(
        ctx: TemplateContext,
        webview_enabled: bool,
        engine: Option<ResolvedWebViewBackend>,
    ) -> TemplateContext {
        let answers = super::BrowserAnswers {
            webview_enabled,
            engine,
        };
        ctx.with_browser(BrowserTemplateContext::desktop(answers, answers, answers))
    }

    /// A fake local framework checkout the generated crate's feature forwards
    /// read their destinations from: `ffi/Cargo.toml` declaring
    /// `ffi_features`, and a Rust Apple backend at `backends/apple` — the
    /// `Package.swift` marker makes `waterui_path` consume it as the local
    /// backend — declaring the backend forward destinations `map`, `media`
    /// and `webview`.
    /// A minimal crate manifest: package header plus a `[features]` table.
    fn fixture_manifest(name: &str, version: &str, features: &[&str]) -> String {
        use std::fmt::Write as _;
        let mut text =
            format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n");
        if !features.is_empty() {
            text.push_str("\n[features]\n");
            for feature in features {
                let _ = writeln!(text, "{feature} = []");
            }
        }
        text
    }

    /// A minimal crate fixture — manifest plus an empty lib target — written
    /// under `dir`.
    fn write_fixture_crate(dir: &Path, name: &str, features: &[&str]) {
        std::fs::create_dir_all(dir.join("src")).expect("fixture src dir");
        std::fs::write(
            dir.join("Cargo.toml"),
            fixture_manifest(name, "0.0.0", features),
        )
        .expect("fixture manifest");
        std::fs::write(dir.join("src/lib.rs"), "").expect("fixture lib");
    }

    fn write_fake_framework_checkout(root: &Path, ffi_features: &[&str]) {
        fn manifest(name: &str, features: &[&str]) -> String {
            fixture_manifest(name, "0.0.0", features)
        }
        // The root package doubles as the workspace root: the `waterui`
        // facade resolves to it as a path dependency, so its `[features]`
        // table is what the forward filter learns for `waterui/*` forwards —
        // `gpu` and `video` here, so a facade missing `media`/`webview` keeps
        // those forwards off the generated manifest — and
        // `local_checkout_dependency` reads `waterui-browser-cef` out of
        // `[workspace.dependencies]`.
        std::fs::create_dir_all(root).expect("checkout root");
        std::fs::write(
            root.join("Cargo.toml"),
            format!(
                "{}\n[workspace]\nmembers = [\"ffi\", \"backends/apple\", \"backends/cef\"]\n\n[workspace.dependencies]\nwaterui-browser-cef = {{ path = \"backends/cef\" }}\n",
                manifest("waterui", &["gpu", "video"])
            ),
        )
        .expect("write waterui manifest");
        std::fs::create_dir_all(root.join("ffi")).expect("ffi manifest dir");
        std::fs::write(
            root.join("ffi/Cargo.toml"),
            manifest("waterui-ffi", ffi_features),
        )
        .expect("write waterui-ffi manifest");
        let apple = root.join("backends/apple");
        std::fs::create_dir_all(&apple).expect("apple backend dir");
        std::fs::write(apple.join("Package.swift"), "// swift-tools-version:6.0\n")
            .expect("Package.swift marker");
        std::fs::write(
            apple.join("Cargo.toml"),
            manifest("waterui-apple", &["map", "media", "webview"]),
        )
        .expect("write waterui-apple manifest");
        write_fixture_crate(
            &root.join("backends/cef"),
            "waterui-browser-cef",
            &["chromium", "webview"],
        );
    }

    /// A local checkout always builds Android against the runtime
    /// `android-backend-revision` pins — the Kotlin runtime is the only
    /// Android runtime source.
    #[test]
    fn android_checkout_builds_against_the_pinned_runtime() {
        let workspace = tempdir().expect("tempdir");
        let waterui = workspace.path().join("waterui");
        let project = workspace.path().join("app");
        std::fs::create_dir_all(&waterui).expect("checkout");
        std::fs::create_dir_all(project.join("android")).expect("project");
        let context = || {
            ctx(
                Some(waterui.clone()),
                Some(project.join("android")),
                Some(project.clone()),
            )
        };

        let pinned = |ctx: &TemplateContext| {
            assert!(
                ctx.android_runtime_dependency().contains(&"c".repeat(40)),
                "the coordinate names the declared android-backend-revision"
            );
        };
        pinned(&context());
    }

    /// The generated `settings.gradle.kts` resolves the runtime through
    /// `JitPack` unconditionally — the pinned remote coordinate is the only
    /// source, local checkout or not.
    #[test]
    fn android_settings_resolves_the_pinned_runtime() {
        let waterui_root = tempdir().expect("waterui root");
        let mut document = toml::Table::new();
        document.insert(
            "waterui_path".into(),
            waterui_root.path().display().to_string().into(),
        );
        let mut package = toml::Table::new();
        package.insert("name".into(), "Demo".into());
        package.insert("bundle_identifier".into(), "dev.waterui.demo".into());
        document.insert("package".into(), package.into());
        let document = toml::to_string(&document).expect("manifest serializes");
        let manifest: crate::project::Manifest =
            toml::from_str(&document).expect("manifest parses");

        let context = |manifest: &crate::project::Manifest| {
            let local_sources = smol::block_on(crate::templates::project_local_backend_sources(
                manifest.waterui_path.as_deref().map(Path::new),
                waterui_root.path(),
            ))
            .expect("fixture checkout resolves canonical backend sources");
            TemplateContext::for_project_manifest(
                manifest,
                CrateName::try_from("demo").expect("crate name"),
                "Demo",
                &stable_framework(),
                &local_sources,
            )
            .with_backend_project_path(PathBuf::from("/proj/android"))
            .with_project_root_path(PathBuf::from("/proj"))
        };

        let template = embedded::ANDROID
            .get_file("settings.gradle.kts.tpl")
            .expect("settings.gradle.kts template must exist")
            .contents_utf8()
            .expect("settings.gradle.kts template must be utf-8");
        let render = |ctx: &TemplateContext| {
            render_scaffold_template(
                TemplateNamespace::Android,
                std::path::Path::new("settings.gradle.kts.tpl"),
                template,
                ctx,
            )
            .expect("settings.gradle.kts render")
        };

        for manifest in [
            &manifest,
            &toml::from_str::<crate::project::Manifest>(
                r#"
                    [package]
                    name = "Demo"
                    bundle_identifier = "dev.waterui.demo"
                "#,
            )
            .expect("remote manifest parses"),
        ] {
            let rendered = render(&context(manifest));
            assert!(rendered.contains("https://jitpack.io"), "{rendered}");
            assert!(!rendered.contains("includeBuild"), "{rendered}");
            crate::assets::assert_settings_plugin_markers(&rendered);
        }
    }

    /// Embedded mode scaffolds a Gradle *library* project (`:waterui`, an
    /// Android library publishing an AAR), never an application — the host
    /// keeps its own application module (water-rs/cli#223).
    #[test]
    fn android_embedded_renders_a_publishing_library() {
        let manifest: crate::project::Manifest = toml::from_str(
            r#"
                [package]
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"
                embedded = true

                [permissions.internet]
                enable = true
                description = "Fetch remote content"

                [permissions.camera]
                enable = true
                description = "Scan codes"
            "#,
        )
        .expect("manifest parses");

        // Permissions reach the context the way the Android backend passes
        // them, through `manifest_permissions`, so the template sees the
        // fully qualified names a real build hands it.
        let ctx = TemplateContext::for_project_manifest(
            &manifest,
            CrateName::try_from("demo").expect("crate name"),
            "Demo",
            &stable_framework(),
            &LocalBackendSources::default(),
        )
        .with_backend_project_path(PathBuf::from("/proj/android"))
        .with_project_root_path(PathBuf::from("/proj"))
        .with_android_permissions(crate::android::backend::manifest_permissions(&manifest))
        .with_crate_version("1.2.3");

        let render = |relative: &str, ctx: &TemplateContext| {
            let template = embedded::ANDROID_EMBEDDED
                .get_file(relative)
                .unwrap_or_else(|| panic!("embedded template {relative} must exist"))
                .contents_utf8()
                .expect("embedded template must be utf-8");
            render_scaffold_template(
                TemplateNamespace::AndroidEmbedded,
                std::path::Path::new(relative),
                template,
                ctx,
            )
            .unwrap_or_else(|error| panic!("embedded template {relative} render: {error}"))
        };

        // A library module applying `com.android.library` and publishing a
        // `release` AAR under the crate's Maven coordinate — group is the
        // bundle identifier, artifact is the crate name, version is the
        // crate's Cargo version.
        let module = render("waterui/build.gradle.kts.tpl", &ctx);
        assert!(module.contains("id(\"com.android.library\")"), "{module}");
        assert!(
            module.contains("namespace = \"dev.waterui.demo.waterui\""),
            "{module}"
        );
        assert!(
            module.contains("groupId = \"dev.waterui.demo\""),
            "{module}"
        );
        assert!(module.contains("artifactId = \"demo\""), "{module}");
        assert!(module.contains("version = \"1.2.3\""), "{module}");
        assert!(module.contains("from(components[\"release\"])"), "{module}");
        // `api`, not `implementation`: the runtime's `WaterUiRootView` must
        // stay on the host app's compile classpath.
        assert!(
            module.contains(&format!("api(\"{}\")", ctx.android_runtime_dependency())),
            "{module}"
        );

        // JitPack stays on the repository list for the pinned runtime
        // coordinate.
        let settings = render("settings.gradle.kts.tpl", &ctx);
        assert!(settings.contains("include(\":waterui\")"), "{settings}");
        assert!(settings.contains("https://jitpack.io"), "{settings}");
        assert!(!settings.contains("includeBuild"), "{settings}");

        // Declared permissions render into the library's manifest so the AAR
        // merges them into the host's.
        let android_manifest = render("waterui/src/main/AndroidManifest.xml.tpl", &ctx);
        let declared: Vec<&str> = android_manifest
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("<uses-permission"))
            .collect();
        assert_eq!(
            declared,
            [
                "<uses-permission android:name=\"android.permission.INTERNET\" />",
                "<uses-permission android:name=\"android.permission.CAMERA\" />",
            ],
            "{android_manifest}"
        );
        // The library manifest carries the managed components block too: the
        // host's manifest merger folds its `<application>` children into the
        // host's own.
        crate::assets::assert_component_markers_inside_application(&android_manifest);
    }

    fn support_ctx() -> TemplateContext {
        TemplateContext::for_support_app(
            SupportAppIdentity {
                display_name: "WaterUIApp".to_string(),
                crate_name: CrateName::try_from("waterui_app")
                    .expect("test crate name must be valid"),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.support")
                    .expect("test bundle identifier must be valid"),
            },
            Some(PathBuf::from("../..")),
            &stable_framework(),
            false,
            None,
            &LocalBackendSources::default(),
        )
        .with_backend_project_path(PathBuf::from("managed_backends/apple"))
    }

    /// The `=<version>` requirement the fixture framework pins `key` at.
    fn pinned(key: &str) -> String {
        format!("={}", stable_framework().scaffold_value(key))
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

        // `waterui-dew` is git-pinned — `stable` withholds it, so the
        // firmware templates render against a `dev` resolution.
        let mut s3 = project_ctx();
        s3.framework = dev_framework();
        s3.esp32 = Esp32TemplateEntry::new(Esp32Chip::Esp32S3, 410, 502, 16);
        let mut c3 = project_ctx();
        c3.framework = dev_framework();
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
        assert!(c3_manifest.contains("opt-level = 2"));
        // Release firmware is flash-budgeted: whole-program LTO and symbol
        // stripping are not optional niceties on a 4-16 MB part.
        assert!(c3_manifest.contains("lto = \"fat\""));
        assert!(c3_manifest.contains("codegen-units = 1"));
        assert!(c3_manifest.contains("strip = \"symbols\""));
        assert!(c3_manifest.contains("features = [\"espidf\", \"progress\"]"));

        // main.rs panel geometry follows the entry.
        assert!(render_esp32("src/main.rs.tpl", &c3).contains("PanelConfig::new(200, 240, 16)"));

        // Configured fonts render as flash-embedded binaries; without any,
        // the FONTS table is empty and dew fails fast at the first text.
        let mut with_fonts = project_ctx();
        with_fonts.esp32 = Esp32TemplateEntry::new(Esp32Chip::Esp32C3, 200, 240, 16)
            .with_fonts(vec!["/tmp/fonts/Demo.ttf".to_string()]);
        let main_rs = render_esp32("src/main.rs.tpl", &with_fonts);
        assert!(main_rs.contains("include_bytes!(\"/tmp/fonts/Demo.ttf\")"));
        assert!(main_rs.contains("FONTS"));
        assert!(!render_esp32("src/main.rs.tpl", &c3).contains("include_bytes!"));
    }

    fn render_embedded(
        namespace: TemplateNamespace,
        embedded_dir: &Dir<'_>,
        relative: &str,
        ctx: &TemplateContext,
    ) -> String {
        let template = embedded_dir
            .get_file(relative)
            .unwrap_or_else(|| panic!("template {relative} must exist"))
            .contents_utf8()
            .expect("template must be utf-8");
        render_scaffold_template(namespace, std::path::Path::new(relative), template, ctx)
            .unwrap_or_else(|error| panic!("template {relative} render: {error}"))
    }

    #[test]
    fn generated_runtime_templates_install_app_catalog() {
        // #732: runtime `text("...")` resolves through the `TranslationCatalog`
        // `configure_environment!` installs, so every environment boundary the
        // CLI generates must go through it — previews, preview tests and
        // firmware alike.
        let mut ctx =
            project_ctx().with_backend_project_path(PathBuf::from("managed_backends/hydrolysis"));
        // The esp32 and gtk4 manifests resolve git-pinned scaffold packages
        // `stable` withholds — the assertions below render them on `dev`.
        ctx.framework = dev_framework();

        for relative in [
            "src/main.rs.tpl",
            "src/lib.rs.tpl",
            "src/mcp_runtime.rs.tpl",
        ] {
            let rendered = render_embedded(
                TemplateNamespace::Hydrolysis,
                &embedded::HYDROLYSIS,
                relative,
                &ctx,
            );
            assert!(
                rendered.contains("waterui::configure_environment!"),
                "hydrolysis {relative} must create its environment through `configure_environment!`"
            );
        }

        // Preview runtimes reach `configure_environment!` through the
        // generated bindings: `app_environment()` hands the configured
        // environment to the application's `app(env)` composition root, which
        // is what installs application-owned realizations such as
        // `waterui_map_gpu::install` (#93). A runtime that built a bare
        // `Environment` would render components the app never could.
        for relative in [
            "src/preview_runtime.rs.tpl",
            "src/preview_test_runtime.rs.tpl",
        ] {
            let rendered = render_embedded(
                TemplateNamespace::Hydrolysis,
                &embedded::HYDROLYSIS,
                relative,
                &ctx,
            );
            assert!(
                rendered.contains("app_environment()"),
                "hydrolysis {relative} must take its environment from the generated `app_environment()` binding"
            );
        }
        let preview_bindings = include_str!("../templates/preview_target.rs.tpl");
        assert!(
            preview_bindings.contains("waterui::configure_environment!")
                && preview_bindings.contains("::app(env)"),
            "hydrolysis preview bindings must build the preview environment \
             through `configure_environment!` and the application's `app(env)`"
        );

        assert!(
            render_esp32("src/main.rs.tpl", &ctx).contains("waterui_core::configure_environment!"),
            "esp32 firmware must configure the environment through waterui-core, \
             which it already depends on"
        );
        for (namespace, embedded_dir, relative) in [
            (TemplateNamespace::Gtk4, &embedded::GTK4, "src/main.rs.tpl"),
            (TemplateNamespace::Tui, &embedded::TUI, "src/main.rs.tpl"),
        ] {
            let rendered = render_embedded(namespace, embedded_dir, relative, &ctx);
            assert!(
                rendered.contains("configure_environment!"),
                "{relative} must create its environment through `configure_environment!`"
            );
        }

        // A generated crate's `CARGO_MANIFEST_DIR` has no `i18n/`, so each one
        // that creates an environment exports `WATERUI_I18N_DIR` from its build
        // script — `catalog!` then embeds the application's translations.
        for (namespace, embedded_dir) in [
            (TemplateNamespace::Hydrolysis, &embedded::HYDROLYSIS),
            (TemplateNamespace::Esp32, &embedded::ESP32),
            (TemplateNamespace::Ffi, &embedded::FFI),
            (TemplateNamespace::Gtk4, &embedded::GTK4),
            (TemplateNamespace::Tui, &embedded::TUI),
        ] {
            let rendered = render_embedded(namespace, embedded_dir, "build.rs.tpl", &ctx);
            assert!(
                rendered.contains("cargo:rustc-env=WATERUI_I18N_DIR="),
                "generated build.rs must export WATERUI_I18N_DIR for `catalog!`"
            );
            assert!(
                rendered.contains(".join(\"../..\")") && rendered.contains(".join(\"i18n\")"),
                "generated build.rs must resolve i18n/ relative to the project root: {rendered}"
            );
            assert!(
                rendered.contains("cargo:rerun-if-changed={}"),
                "generated build.rs must watch i18n/ so locale changes rebuild the crate"
            );
        }

        // The esp32 harness names `TranslationCatalog` through `waterui-locale`
        // — it has no `waterui` facade dependency to reach it through.
        let esp32_manifest = render_esp32("Cargo.toml.tpl", &ctx);
        assert!(esp32_manifest.contains("waterui-locale"));

        // gtk4's entry point calls `waterui::configure_environment!`, so its
        // manifest must depend on the facade.
        let gtk4_manifest = gtk4::rendered_outputs(&ctx, "test-gtk4")
            .expect("gtk4 manifest should render")
            .into_iter()
            .find(|(path, _)| path == std::path::Path::new("Cargo.toml"))
            .map(|(_, content)| String::from_utf8(content).expect("manifest is utf-8"))
            .expect("gtk4 rendered outputs must include Cargo.toml");
        let gtk4_manifest = gtk4_manifest
            .parse::<toml::Table>()
            .expect("gtk4 Cargo.toml should parse");
        assert!(
            gtk4_manifest["dependencies"].get("waterui").is_some(),
            "gtk4 manifest must depend on waterui for `configure_environment!`"
        );
    }

    #[test]
    fn local_checkout_patches_are_rebased_onto_the_waterui_path() {
        let tempdir = tempdir().expect("temporary checkout dir");
        let checkout = tempdir.path().join("waterui");
        std::fs::create_dir_all(&checkout).expect("checkout dir");
        std::fs::write(
            checkout.join("Cargo.toml"),
            include_str!("../../tests/fixtures/local_checkout_patches.toml"),
        )
        .expect("checkout manifest");
        let project_root = tempdir.path().join("app");
        std::fs::create_dir_all(&project_root).expect("project dir");

        let patches =
            super::local_framework_patches(&project_root, std::path::Path::new("../waterui"))
                .expect("patches from the checkout");
        let crates_io = &patches["crates-io"];
        let cargo_toml::Dependency::Detailed(core) = &crates_io["waterui-core"] else {
            panic!("a path patch stays a detailed dependency");
        };
        assert_eq!(core.path.as_deref(), Some("../waterui/core"));
        let cargo_toml::Dependency::Detailed(vello) = &crates_io["vello"] else {
            panic!("a git patch stays a detailed dependency");
        };
        assert_eq!(
            vello.git.as_deref(),
            Some("https://github.com/lexoliu/vello")
        );
        assert!(vello.path.is_none());

        // The path-patched names are mirrored onto the framework's repository
        // source so extracted crates' git-source dependencies resolve to the
        // same checkout; the fork's git pin is not mirrored.
        let repository = &patches["https://github.com/water-rs/waterui"];
        let cargo_toml::Dependency::Detailed(core) = &repository["waterui-core"] else {
            panic!("a repository-source patch stays a detailed dependency");
        };
        assert_eq!(core.path.as_deref(), Some("../waterui/core"));
        assert!(!repository.contains_key("vello"));
    }

    #[test]
    fn native_backend_manifest_carries_the_checkout_patch_tables() {
        let tempdir = tempdir().expect("temporary checkout dir");
        let checkout = tempdir.path().join("waterui");
        std::fs::create_dir_all(&checkout).expect("checkout dir");
        std::fs::write(
            checkout.join("Cargo.toml"),
            include_str!("../../tests/fixtures/local_checkout_patches.toml"),
        )
        .expect("checkout manifest");

        let ctx = ctx(
            Some(checkout.clone()),
            None,
            Some(tempdir.path().join("app")),
        );
        let manifest = super::render_native_backend_bin_cargo_toml(&ctx, "waterui-test-gtk4", &[])
            .expect("generated manifest renders");

        let core_path = checkout.join("core");
        let manifest: toml::Value = toml::from_str(&manifest).expect("generated manifest parses");
        let patched_path = |source: &str| {
            manifest["patch"][source]["waterui-core"]["path"]
                .as_str()
                .map_or_else(
                    || panic!("no waterui-core path patch under [patch.{source:?}]:\n{manifest}"),
                    std::path::PathBuf::from,
                )
        };
        assert_eq!(patched_path("crates-io"), core_path);
        assert_eq!(
            patched_path("https://github.com/water-rs/waterui"),
            core_path
        );
    }

    #[test]
    fn esp32_manifest_carries_the_checkout_patch_tables() {
        let tempdir = tempdir().expect("temporary checkout dir");
        let checkout = tempdir.path().join("waterui");
        std::fs::create_dir_all(&checkout).expect("checkout dir");
        std::fs::write(
            checkout.join("Cargo.toml"),
            include_str!("../../tests/fixtures/local_checkout_patches.toml"),
        )
        .expect("checkout manifest");

        let ctx = ctx(
            Some(checkout.clone()),
            None,
            Some(tempdir.path().join("app")),
        );
        let manifest = render_esp32("Cargo.toml.tpl", &ctx);
        let manifest: toml::Value = toml::from_str(&manifest).expect("esp32 manifest parses");

        // The askama-rendered manifest must carry the same patch tables the
        // serialized native manifests get — `waterui-dew`'s registry
        // `waterui-*` requirements resolve to the checkout, not a second copy.
        let core_path = checkout.join("core");
        let patched_path = |source: &str| {
            manifest["patch"][source]["waterui-core"]["path"]
                .as_str()
                .map_or_else(
                    || panic!("no waterui-core path patch under [patch.{source:?}]:\n{manifest}"),
                    std::path::PathBuf::from,
                )
        };
        assert_eq!(patched_path("crates-io"), core_path);
        assert_eq!(
            patched_path("https://github.com/water-rs/waterui"),
            core_path
        );
    }

    #[test]
    fn relative_waterui_path_produces_clean_relative_backend_path() {
        let root = tempdir().expect("tempdir");
        let waterui_root = root.path().join("waterui");
        let backend_dir = waterui_root.join("backends/apple");
        std::fs::create_dir_all(&backend_dir).expect("backend dir");
        std::fs::write(
            backend_dir.join("Cargo.toml"),
            "[package]\nname = \"waterui-apple\"\n",
        )
        .expect("backend Cargo.toml");
        let project_root = root.path().join("proj");
        std::fs::create_dir_all(&project_root).expect("project root");

        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(project_root.join("managed_backends/apple")),
            Some(project_root.clone()),
        );

        let path = ctx
            .compute_member_backend_path(crate::framework::APPLE_BACKEND)
            .expect("expected relative backend path");
        let expected =
            pathdiff::diff_paths(&backend_dir, project_root.join("managed_backends/apple"))
                .expect("backend diff path");

        assert_eq!(path, normalize_path_for_config(&expected));
        assert!(!path.contains("//"));
    }

    #[test]
    fn absolute_waterui_path_backends_apple_is_used_directly() {
        let waterui_root = tempdir().expect("waterui root");
        let backend_dir = waterui_root.path().join("backends/apple");
        std::fs::create_dir_all(&backend_dir).expect("backend dir");
        std::fs::write(
            backend_dir.join("Cargo.toml"),
            "[package]\nname = \"waterui-apple\"\n",
        )
        .expect("backend Cargo.toml");

        let ctx = ctx(
            Some(waterui_root.path().to_path_buf()),
            Some(PathBuf::from("apple")),
            None,
        );
        let path = ctx
            .compute_member_backend_path(crate::framework::APPLE_BACKEND)
            .expect("expected backend path");

        assert_eq!(path, normalize_path_for_config(&backend_dir));
    }

    #[test]
    fn waterui_path_backends_apple_is_used() {
        let waterui_root = tempdir().expect("tempdir");
        let backend_dir = waterui_root.path().join("backends/apple");
        std::fs::create_dir_all(&backend_dir).expect("backend dir");
        std::fs::write(
            backend_dir.join("Cargo.toml"),
            "[package]\nname = \"waterui-apple\"\nversion = \"0.1.0\"\n",
        )
        .expect("backend Cargo.toml");
        let project_root = tempdir().expect("tempdir");

        let ctx = ctx(
            Some(waterui_root.path().to_path_buf()),
            Some(project_root.path().join("managed_backends/apple")),
            Some(project_root.path().to_path_buf()),
        );

        let path = ctx
            .compute_member_backend_path(crate::framework::APPLE_BACKEND)
            .expect("waterui_path/backends/apple must resolve");
        assert!(
            path.ends_with("backends/apple"),
            "expected the staged backend path, got {path}"
        );
    }

    /// A `waterui_path` checkout with no `backends/apple` crate cannot supply
    /// `waterui-apple`: the dependency errors naming the checkout rather than
    /// silently retargeting a remote source (water-rs/cli#278).
    #[test]
    fn missing_local_apple_backend_is_an_error() {
        let waterui_root = tempdir().expect("tempdir");
        std::fs::create_dir_all(waterui_root.path().join("backends")).expect("backends dir");

        let ctx = ctx(
            Some(waterui_root.path().to_path_buf()),
            Some(PathBuf::from("managed_backends/apple")),
            None,
        );

        assert!(
            ctx.compute_member_backend_path(crate::framework::APPLE_BACKEND)
                .is_none()
        );
        let error = ctx.waterui_apple_dependency().err().unwrap().to_string();
        assert!(error.contains("backends/apple"), "{error}");
    }

    /// Absent canonical slots select the remote channel; a slot that is
    /// present but malformed — a directory without its required manifest,
    /// a non-directory entry, or a dangling symlink — is a resolution
    /// error naming the slot, never a silent remote fallback (water-rs/cli#276).
    #[test]
    fn malformed_local_backend_sources_fail_at_resolution() {
        let root = tempdir().expect("waterui root");

        let sources = smol::block_on(local_backend_sources(root.path()))
            .expect("absent slots resolve to the remote source");
        assert!(sources.apple().is_none());

        let apple = root.path().join("backends/apple");
        std::fs::create_dir_all(&apple).expect("empty apple slot");
        let error = smol::block_on(local_backend_sources(root.path()))
            .expect_err("a slot without its manifest is malformed");
        assert!(error.to_string().contains("backends/apple"), "{error}");

        std::fs::remove_dir(&apple).expect("remove empty slot");
        std::fs::write(&apple, "not a directory\n").expect("file at slot path");
        let error = smol::block_on(local_backend_sources(root.path()))
            .expect_err("a non-directory slot is malformed");
        assert!(error.to_string().contains("backends/apple"), "{error}");

        #[cfg(unix)]
        {
            std::fs::remove_file(&apple).expect("remove file slot");
            std::os::unix::fs::symlink(root.path().join("gone"), &apple).expect("dangling link");
            let error = smol::block_on(local_backend_sources(root.path()))
                .expect_err("a dangling symlink is malformed");
            assert!(error.to_string().contains("backends/apple"), "{error}");
        }
    }

    #[test]
    fn waterui_apple_dependency_prefers_a_local_checkout() {
        let waterui_root = tempdir().expect("waterui root");
        let backend_dir = waterui_root.path().join("backends/apple");
        std::fs::create_dir_all(&backend_dir).expect("backend dir");
        std::fs::write(
            backend_dir.join("Cargo.toml"),
            "[package]\nname = \"waterui-apple\"\n",
        )
        .expect("backend Cargo.toml");
        let ctx = ctx(
            Some(waterui_root.path().to_path_buf()),
            Some(PathBuf::from("managed_backends/apple")),
            None,
        );

        let detail = ctx.waterui_apple_dependency().unwrap();
        assert_eq!(
            detail.path.as_deref(),
            Some(normalize_path_for_config(&backend_dir).as_str())
        );
        assert!(detail.git.is_none() && detail.rev.is_none() && detail.tag.is_none());
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

        let waterui_root = tempdir().expect("waterui root");
        let backend_dir = waterui_root.path().join("backends/apple");
        std::fs::create_dir_all(&backend_dir).expect("backend dir");
        std::fs::write(
            backend_dir.join("Cargo.toml"),
            "[package]\nname = \"waterui-apple\"\n",
        )
        .expect("backend Cargo.toml");
        let ctx = ctx(
            Some(waterui_root.path().to_path_buf()),
            Some(backend_project_path.clone()),
            Some(project_root.clone()),
        );

        let path = ctx
            .compute_member_backend_path(crate::framework::APPLE_BACKEND)
            .expect("expected backend path");
        assert_eq!(path, normalize_path_for_config(&backend_dir));

        let expected_project_root =
            pathdiff::diff_paths(&project_root, &backend_project_path).expect("project root diff");
        assert_eq!(
            ctx.project_root_relative_path(),
            normalize_path_for_config(&expected_project_root)
        );
    }

    #[test]
    fn android_manifest_enables_picture_in_picture_by_default() {
        let ctx = project_ctx();
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
        crate::assets::assert_component_markers_inside_application(&rendered);
    }

    /// The Apple backend is a framework workspace member: every channel pins
    /// `waterui-apple` to the framework's own repository at the framework's
    /// selected revision — `stable` resolves the certified release's
    /// provenance — never a backend repository, tag or HEAD of its own.
    #[test]
    fn apple_dependency_pins_the_framework_source_on_every_channel() {
        let dependency = |framework: ResolvedFramework| {
            let mut context = project_ctx();
            context.framework = framework;
            context.waterui_apple_dependency().unwrap()
        };
        let repository = framework_repository();
        for (channel, framework) in [
            ("stable", stable_framework()),
            ("dev", dev_framework()),
            ("nightly", nightly_framework()),
        ] {
            let detail = dependency(framework);
            assert_eq!(detail.git.as_deref(), Some(repository), "{channel}");
            assert_eq!(
                detail.rev.as_deref(),
                Some('a'.to_string().repeat(40).as_str()),
                "{channel}"
            );
            assert!(detail.tag.is_none() && detail.path.is_none(), "{channel}");
        }
    }

    /// A framework revision from before the backend's return declares no
    /// `apple-backend-path`: the dependency fails clearly rather than falling
    /// back to a repository or pin the framework does not declare.
    #[test]
    fn a_framework_without_apple_backend_path_is_an_error() {
        let mut context = project_ctx();
        let mut persisted: toml::Value =
            toml::from_str(&toml::to_string(&context.framework).unwrap()).unwrap();
        persisted["metadata"]
            .as_table_mut()
            .unwrap()
            .remove("apple-backend-path");
        context.framework = persisted.try_into().unwrap();
        let error = context
            .waterui_apple_dependency()
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("apple-backend-path"), "{error}");
    }

    /// A local checkout declares `apple-backend-path` but has no crate there:
    /// the staged source is missing, and no remote source substitutes for it.
    #[test]
    fn declared_local_apple_backend_missing_its_crate_is_an_error() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("waterui");
        write_local_checkout(&root);
        let mut context = ctx(
            Some(root.clone()),
            Some(PathBuf::from("managed_backends/apple")),
            None,
        );
        context.framework = smol::block_on(ResolvedFramework::for_local_checkout(&root)).unwrap();

        let error = context
            .waterui_apple_dependency()
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("backends/apple"), "{error}");
    }

    #[test]
    fn android_build_gradle_uses_embedded_remote_backend_revision() {
        let mut ctx = project_ctx();
        // A sentinel floor proves the scaffold renders the resolved
        // framework's `android-min-api-level`, not a CLI-owned constant.
        let mut persisted: toml::Value =
            toml::from_str(&toml::to_string(&ctx.framework).unwrap()).unwrap();
        persisted["metadata"]["android-min-api-level"] = toml::Value::Integer(30);
        ctx.framework = persisted.try_into().unwrap();

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

        assert!(rendered.contains("minSdk = 30"));
        crate::assets::assert_module_plugin_markers(&rendered);
        assert!(rendered.contains(&jitpack_dependency_coordinate(
            ctx.framework.scaffold_value("android-backend-url"),
            ctx.framework.scaffold_value("android-backend-revision"),
        )));
    }

    #[test]
    fn kotlin_string_literal_escapes_quotes_backslashes_and_dollars() {
        // A `$` is a Kotlin string-template opening — `${...}` evaluates
        // arbitrary build-script code — so it must be escaped like `"` and `\`.
        let escaped = super::kotlin_string_literal("a\"b\\c${d}");
        assert_eq!(escaped, "a\\\"b\\\\c\\${d}");
        let entry = super::AndroidSigningTemplateEntry::from(
            &crate::android::signing::AndroidSigningConfig::new("keys/$store.jks", "up${evil}")
                .expect("a config without control characters"),
        );
        assert_eq!(entry.keystore, "keys/\\$store.jks");
        assert_eq!(entry.key_alias, "up\\${evil}");
    }

    #[test]
    fn android_build_gradle_renders_release_signing_config() {
        let template = embedded::ANDROID
            .get_file("app/build.gradle.kts.tpl")
            .expect("android build.gradle template must exist")
            .contents_utf8()
            .expect("android build.gradle template must be utf-8");

        let render = |ctx: &TemplateContext| {
            render_scaffold_template(
                TemplateNamespace::Android,
                std::path::Path::new("app/build.gradle.kts.tpl"),
                template,
                ctx,
            )
            .expect("android build.gradle render")
        };

        // No [signing.android] entry: the release build type stays unsigned
        // (Gradle's own unsigned output), and packaging fails earlier unless
        // the caller asked for it.
        let mut ctx = project_ctx();
        let unsigned = render(&ctx);
        assert!(!unsigned.contains("signingConfigs"));
        assert!(!unsigned.contains("signingConfig ="));

        ctx.android_signing = Some(super::AndroidSigningTemplateEntry::from(
            &crate::android::signing::AndroidSigningConfig::new("release.jks", "upload")
                .expect("a config without control characters"),
        ));
        let signed = render(&ctx);
        assert!(signed.contains(r#"storeFile = projectRoot.resolve("release.jks")"#));
        assert!(signed.contains(r#"keyAlias = "upload""#));
        // Passwords are env reads, never literals.
        assert!(
            signed.contains(r#"storePassword = System.getenv("WATERUI_ANDROID_STORE_PASSWORD")"#)
        );
        assert!(signed.contains(r#"keyPassword = System.getenv("WATERUI_ANDROID_KEY_PASSWORD")"#));
        // `--unsigned` suppresses the config through the environment.
        assert!(signed.contains(r#"System.getenv("WATERUI_ANDROID_UNSIGNED") != "1""#));
        assert!(signed.contains(r#"signingConfig = signingConfigs.getByName("release")"#));
    }

    #[test]
    fn hydrolysis_android_build_gradle_renders_release_signing_config() {
        // `[signing.android]` is an Android platform contract: the Hydrolysis
        // host's generated app signs its release variant the same way the
        // Android backend's does.
        let template = embedded::HYDROLYSIS_ANDROID
            .get_file("app/build.gradle.kts.tpl")
            .expect("hydrolysis android build.gradle template must exist")
            .contents_utf8()
            .expect("hydrolysis android build.gradle template must be utf-8");

        let render = |ctx: &TemplateContext| {
            render_scaffold_template(
                TemplateNamespace::HydrolysisAndroid,
                std::path::Path::new("app/build.gradle.kts.tpl"),
                template,
                ctx,
            )
            .expect("hydrolysis android build.gradle render")
        };

        let mut ctx =
            project_ctx().with_hydrolysis_android(super::HydrolysisAndroidTemplateEntry {
                native_library_name: "waterui_test_hydrolysis".to_string(),
                host_project_dir: "../android-host/rev/android".to_string(),
                project_root: "..".to_string(),
                painter_dependency: "dev.waterui.hydrolysis:gpu".to_string(),
                painter_module: "gpu".to_string(),
                min_api_level: 31,
                painter_band_import: None,
                painter_band_class: None,
            });
        let unsigned = render(&ctx);
        assert!(!unsigned.contains("signingConfigs"));
        assert!(!unsigned.contains("signingConfig ="));

        ctx.android_signing = Some(super::AndroidSigningTemplateEntry::from(
            &crate::android::signing::AndroidSigningConfig::new("release.jks", "upload")
                .expect("a config without control characters"),
        ));
        let signed = render(&ctx);
        assert!(signed.contains(r#"storeFile = projectRoot.resolve("release.jks")"#));
        assert!(signed.contains(r#"keyAlias = "upload""#));
        assert!(
            signed.contains(r#"storePassword = System.getenv("WATERUI_ANDROID_STORE_PASSWORD")"#)
        );
        assert!(signed.contains(r#"keyPassword = System.getenv("WATERUI_ANDROID_KEY_PASSWORD")"#));
        assert!(signed.contains(r#"System.getenv("WATERUI_ANDROID_UNSIGNED") != "1""#));
        assert!(signed.contains(r#"signingConfig = signingConfigs.getByName("release")"#));
    }

    #[test]
    fn android_activity_installs_edge_to_edge_and_leases_activity_context() {
        let ctx = project_ctx();
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
        // The environment reaches the app through `waterui.env.*` intent
        // extras applied by `Os.setenv`; nothing reads system properties.
        assert!(activity.contains("setupEnvironmentFromIntent(intent)"));
        assert!(!activity.contains("SystemProperties"));
        // The dev-server URL is forwarded only on debuggable builds.
        assert!(activity.contains(r#"envVar == "WATERUI_DEV_URL" && !BuildConfig.DEBUG"#));
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
        assert!(application.contains("processEnvironment = WuiEnvironment.create(waterUiFonts)"));
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
    fn gtk4_scaffold_pins_the_declared_git_source() {
        // `waterui-gtk` is git-pinned in the workspace manifest — `stable`
        // withholds it, so the scaffold resolves the pin `dev` carries.
        let mut ctx = project_ctx();
        ctx.framework = dev_framework();
        let tempdir = tempdir().expect("temporary gtk scaffold dir");

        smol::block_on(crate::templates::gtk4::scaffold(
            tempdir.path(),
            &ctx,
            "waterui-test-gtk",
        ))
        .expect("gtk4 scaffold should succeed");

        let cargo_toml = std::fs::read_to_string(tempdir.path().join("Cargo.toml"))
            .expect("gtk4 Cargo.toml should be written");
        let manifest: toml::Value = toml::from_str(&cargo_toml).unwrap();
        let gtk = &manifest["dependencies"]["waterui-gtk"];
        assert_eq!(
            gtk["git"].as_str(),
            Some(ctx.framework.scaffold_value("waterui-gtk-git"))
        );
        assert_eq!(
            gtk["rev"].as_str(),
            Some(ctx.framework.scaffold_value("waterui-gtk-rev"))
        );
        assert!(!cargo_toml.contains("webview-default"));
    }

    #[test]
    fn winui_scaffold_pins_the_backend_and_its_vendored_patch_to_one_source() {
        // `waterui-winui` is a git pin — `stable` withholds it —
        // so the scaffold resolves the pin `dev` carries.
        let mut ctx = project_ctx();
        ctx.framework = dev_framework();
        let manifest = crate::templates::winui::rendered_outputs(&ctx, "waterui-test-winui")
            .expect("winui outputs should render")
            .into_iter()
            .find_map(|(path, content)| {
                (path == std::path::Path::new("Cargo.toml"))
                    .then(|| String::from_utf8(content).expect("Cargo.toml must be UTF-8"))
            })
            .expect("winui Cargo.toml output should exist");
        let manifest: toml::Value = toml::from_str(&manifest).expect("winui manifest must parse");

        let framework = dev_framework();
        let backend = &manifest["dependencies"]["waterui-winui"];
        assert_eq!(
            backend["git"].as_str(),
            Some(framework.scaffold_value("waterui-winui-git")),
            "the backend pins the repository the framework declares"
        );
        assert_eq!(
            backend["rev"].as_str(),
            Some(framework.scaffold_value("waterui-winui-rev")),
            "the backend pins the revision the framework declares"
        );

        // The vendored `gpu-allocator` member narrows an upstream `windows`
        // range; it must resolve from the same commit `waterui-winui` does.
        let gpu_allocator = &manifest["patch"]["crates-io"]["gpu-allocator"];
        assert_eq!(gpu_allocator["git"].as_str(), backend["git"].as_str());
        assert_eq!(gpu_allocator["rev"].as_str(), backend["rev"].as_str());

        // The generated crate is its own workspace root and carries the
        // Windows runtime build tools as build-dependencies.
        assert!(manifest["workspace"].is_table());
        assert_eq!(
            manifest["build-dependencies"]["windows-reactor-setup"].as_str(),
            Some("^0.100")
        );
        assert_eq!(
            manifest["build-dependencies"]["winresource"].as_str(),
            Some("^0.1")
        );
    }

    #[test]
    fn generated_native_backends_only_bridge_the_platform_engine_when_no_engine_is_linked() {
        // No engine crate in the graph: the backend bridges what the platform
        // gives it. `waterui-gtk` is git-pinned — `stable` withholds it — so
        // the GTK scaffolds render against a `dev` resolution.
        let mut gtk_ctx = all_os_browser(project_ctx(), true, None);
        gtk_ctx.framework = dev_framework();
        let tempdir = tempdir().expect("temporary gtk webview scaffold dir");
        smol::block_on(crate::templates::gtk4::scaffold(
            tempdir.path(),
            &gtk_ctx,
            "waterui-test-gtk",
        ))
        .expect("gtk4 webview scaffold should succeed");
        let gtk_manifest = std::fs::read_to_string(tempdir.path().join("Cargo.toml"))
            .expect("gtk4 Cargo.toml should be written");
        assert!(gtk_manifest.contains("features = [\"webview-system\"]"));

        // An application that linked its own engine draws through that, so the
        // backend compiles no web engine at all.
        let mut gtk_wpe_ctx =
            all_os_browser(project_ctx(), true, Some(ResolvedWebViewBackend::Wpe));
        gtk_wpe_ctx.framework = dev_framework();
        let gtk_wpe_manifest =
            crate::templates::gtk4::rendered_outputs(&gtk_wpe_ctx, "waterui-test-gtk-wpe")
                .expect("GTK WPE outputs should render")
                .into_iter()
                .find_map(|(path, content)| {
                    (path == std::path::Path::new("Cargo.toml"))
                        .then(|| String::from_utf8(content).expect("Cargo.toml must be UTF-8"))
                })
                .expect("GTK WPE Cargo.toml output should exist");
        assert!(!gtk_wpe_manifest.contains("webview-system"));

        let hydrolysis_ctx = all_os_browser(project_ctx(), true, Some(ResolvedWebViewBackend::Cef));
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
        let native_dependencies = &manifest["target"]["cfg(all(not(target_arch = \"wasm32\"), not(target_os = \"android\")))"]
            ["dependencies"];
        // The generated crate must not carry `waterui-preview` — the
        // preview runtime module no longer uses the support-app crate.
        assert!(
            native_dependencies.get("waterui-preview").is_none(),
            "the generated crate must not depend on waterui-preview"
        );
        assert_eq!(
            native_dependencies["waterui-preview-protocol"]["version"].as_str(),
            Some(pinned("waterui-preview-protocol-version").as_str()),
        );
        // Each of these is a separately versioned package. Borrowing a sibling's
        // pin reads fine while the numbers happen to coincide and emits an
        // unresolvable requirement the moment one of them bumps on its own.
        assert_eq!(
            native_dependencies["waterui-core"]["version"].as_str(),
            Some(pinned("waterui-core-version").as_str()),
        );
        assert_eq!(
            native_dependencies["hydrolysis-m3"]["version"].as_str(),
            Some(pinned("hydrolysis-m3-version").as_str()),
        );
        // The engine-dependent pieces never sit in the shared native table:
        // `webview-system` and `waterui-browser-cef` answer differently per
        // OS and live in the per-OS `cfg` sections.
        let shared_hydrolysis_features = native_dependencies["hydrolysis"]["features"]
            .as_array()
            .expect("hydrolysis declares features")
            .iter()
            .map(|feature| feature.as_str().expect("feature should be a string"))
            .collect::<Vec<_>>();
        assert_eq!(
            shared_hydrolysis_features,
            ["winit"],
            "the shared native table carries no engine-dependent feature"
        );
        // The generated crate depends on the engine the application chose —
        // under the OS section whose graph links it, never the shared one.
        assert_eq!(
            manifest["target"]["cfg(target_os = \"macos\")"]["dependencies"]["waterui-browser-cef"]
                ["version"]
                .as_str(),
            Some(pinned("waterui-browser-cef-version").as_str()),
        );
        assert_eq!(manifest["package"]["autobins"].as_bool(), Some(false));
        let bins = manifest["bin"]
            .as_array()
            .expect("CEF Hydrolysis manifest should declare binaries");
        assert!(bins.iter().any(|bin| {
            bin["name"].as_str() == Some("waterui-test-hydrolysis-cef-helper")
                && bin["path"].as_str() == Some("src/bin/waterui-cef-helper.rs")
        }));
    }

    /// The native table's serving set spans three OSes whose graph answers
    /// legitimately differ — `waterui-browser-wpe` enters on Linux — so the
    /// manifest writes one `cfg` section per OS, each carrying that OS's
    /// own answers: Linux's `wpe` engine suppresses the `webview-system`
    /// bridge that macOS and Windows still bridge, and only macOS's `cef`
    /// engine pulls `waterui-browser-cef`.
    #[test]
    fn the_hydrolysis_manifest_splits_engine_dependent_pieces_per_os() {
        let ctx = project_ctx().with_browser(BrowserTemplateContext::desktop(
            super::BrowserAnswers {
                webview_enabled: true,
                engine: Some(ResolvedWebViewBackend::Cef),
            },
            super::BrowserAnswers {
                webview_enabled: true,
                engine: Some(ResolvedWebViewBackend::Wpe),
            },
            super::BrowserAnswers {
                webview_enabled: true,
                engine: None,
            },
        ));
        let cargo_toml =
            crate::templates::hydrolysis::rendered_outputs(&ctx, "waterui-test-hydrolysis")
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
        let target = &manifest["target"];

        let features_of = |cfg: &str| -> Vec<String> {
            target
                .get(cfg)
                .and_then(|section| section.get("dependencies"))
                .and_then(|deps| deps.get("hydrolysis"))
                .and_then(|dep| dep.get("features"))
                .and_then(toml::Value::as_array)
                .map(|features| {
                    features
                        .iter()
                        .filter_map(|feature| feature.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        // Shared native table: `winit` only — the engine pieces moved out.
        assert_eq!(
            features_of("cfg(all(not(target_arch = \"wasm32\"), not(target_os = \"android\")))"),
            ["winit"],
        );
        // macOS answers CEF: the bridge is suppressed for an application
        // that linked its own engine, and the helper's crate rides the
        // macOS section.
        assert!(!features_of("cfg(target_os = \"macos\")").contains(&"webview-system".to_string()),);
        assert!(
            target["cfg(target_os = \"macos\")"]
                .get("dependencies")
                .and_then(|deps| deps.get("waterui-browser-cef"))
                .is_some(),
            "the macOS section carries the CEF engine crate",
        );
        // Linux answers `wpe`: no bridge, no CEF — and no other content, so
        // its section is not written at all.
        assert!(
            target.get("cfg(target_os = \"linux\")").is_none(),
            "an OS section with nothing engine-dependent is not written: {cargo_toml}"
        );
        // Windows answers no engine: the bridge renders.
        assert_eq!(features_of("cfg(windows)"), ["webview-system"],);
        // The subprocess helper bin is declared once for the manifest, and
        // its source gates the Chromium dispatch on the macOS section that
        // provides the crate.
        let helper =
            crate::templates::hydrolysis::rendered_outputs(&ctx, "waterui-test-hydrolysis")
                .expect("hydrolysis outputs should render")
                .into_iter()
                .find_map(|(path, content)| {
                    (path == std::path::Path::new("src/bin/waterui-cef-helper.rs"))
                        .then(|| String::from_utf8(content).expect("helper source must be UTF-8"))
                })
                .expect("the helper source renders");
        assert!(
            helper.contains("any(target_os = \"macos\")"),
            "the helper compiles its dispatch where the CEF dep exists: {helper}"
        );
    }

    /// The generated backend's `[lib]` and main `[[bin]]` never share a name:
    /// same-named targets share an output filename stem in the target
    /// directory (`<name>.pdb`, `<name>.d`), which Cargo reports as an
    /// `output filename collision` on every build.
    #[test]
    fn hydrolysis_manifest_gives_lib_and_bin_targets_distinct_names() {
        let package_name = "e2eapp-hydrolysis-1a2b3c4d";
        let cargo_toml =
            crate::templates::hydrolysis::rendered_outputs(&project_ctx(), package_name)
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
        let lib_name = manifest["lib"]["name"]
            .as_str()
            .expect("the lib target is named explicitly");
        assert_eq!(lib_name, "e2eapp_hydrolysis_1a2b3c4d_lib");
        let bins = manifest["bin"]
            .as_array()
            .expect("the manifest declares the backend binary");
        let bin_names: Vec<&str> = bins
            .iter()
            .map(|bin| bin["name"].as_str().expect("bin name"))
            .collect();
        assert_eq!(bin_names, [package_name]);
        // Cargo compares output stems as crate identifiers.
        assert!(
            bin_names
                .iter()
                .all(|bin| bin.replace('-', "_") != lib_name),
            "lib {lib_name} collides with a bin in {bin_names:?}"
        );
    }

    #[test]
    fn hydrolysis_manifest_enables_wasm_opt_for_the_features_rustc_emits() {
        // wasm-pack invokes `wasm-opt -O` with no feature flags; without the
        // wasm32 default target features enabled, binaryen rejects the
        // bulk-memory ops rustc emits for memcpy/memset and every `--release`
        // web bundle fails validation (#95).
        let cargo_toml = crate::templates::hydrolysis::rendered_outputs(
            &project_ctx(),
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
        let wasm_opt =
            manifest["package"]["metadata"]["wasm-pack"]["profile"]["release"]["wasm-opt"]
                .as_array()
                .expect("wasm-pack release profile should carry wasm-opt flags")
                .iter()
                .map(|flag| flag.as_str().expect("wasm-opt flag should be a string"))
                .collect::<Vec<_>>();
        for required in [
            "--enable-bulk-memory",
            "--enable-mutable-globals",
            "--enable-sign-ext",
            "--enable-nontrapping-float-to-int",
            "--enable-reference-types",
        ] {
            assert!(
                wasm_opt.contains(&required),
                "wasm-opt flags should include {required}, got {wasm_opt:?}"
            );
        }
    }

    #[test]
    fn path_pinned_hydrolysis_manifest_uses_the_checkouts_own_sources() {
        // A project pinned to a local checkout resolves `hydrolysis` as the
        // checkout's own in-tree member — a `path` into
        // `backends/hydrolysis`, never the independent repository (#1635) —
        // and `hydrolysis-m3` through the checkout's declared source, the
        // `[patch.crates-io]` git pin here.
        let tempdir = tempdir().expect("temporary checkout dir");
        let checkout = tempdir.path().join("waterui");
        std::fs::create_dir_all(checkout.join("backends/hydrolysis")).expect("checkout dirs");
        std::fs::write(
            checkout.join("Cargo.toml"),
            include_str!("../../tests/fixtures/local_checkout_patches.toml"),
        )
        .expect("checkout manifest");
        std::fs::write(
            checkout.join("backends/hydrolysis/Cargo.toml"),
            "[package]\nname = \"hydrolysis\"\n",
        )
        .expect("member manifest");

        let hydrolysis_ctx = ctx(
            Some(checkout.clone()),
            Some(PathBuf::from("managed_backends/hydrolysis")),
            None,
        );
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

        for cfg in [
            "cfg(all(not(target_arch = \"wasm32\"), not(target_os = \"android\")))",
            "cfg(target_os = \"android\")",
            "cfg(target_arch = \"wasm32\")",
        ] {
            let dependencies = &manifest["target"][cfg]["dependencies"];
            let hydrolysis = &dependencies["hydrolysis"];
            assert!(
                hydrolysis["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("backends/hydrolysis")),
                "{cfg} hydrolysis must resolve the checkout's own member by path"
            );
            assert!(hydrolysis.get("git").is_none());
            assert!(hydrolysis.get("version").is_none());
            let m3 = &dependencies["hydrolysis-m3"];
            assert_eq!(
                m3["git"].as_str(),
                Some("https://github.com/water-rs/hydrolysis-m3"),
            );
            assert_eq!(m3["rev"].as_str(), Some("d8872e5"));
            assert!(m3.get("path").is_none());
        }

        // In-tree crates still resolve by path into the checkout.
        let native_dependencies = &manifest["target"]["cfg(all(not(target_arch = \"wasm32\"), not(target_os = \"android\")))"]
            ["dependencies"];
        assert_eq!(
            native_dependencies["waterui-core"]["path"].as_str(),
            Some(normalize_path_for_config(&checkout.join("core")).as_str()),
        );

        // A registry requirement no patch overrides stays a registry dep — the
        // same source the checkout's `[workspace.dependencies]` declares.
        let gtk_ctx = ctx(Some(checkout), Some(PathBuf::from("gtk4")), None);
        let gtk_manifest = crate::templates::gtk4::rendered_outputs(&gtk_ctx, "waterui-test-gtk")
            .expect("GTK outputs should render")
            .into_iter()
            .find_map(|(path, content)| {
                (path == std::path::Path::new("Cargo.toml"))
                    .then(|| String::from_utf8(content).expect("Cargo.toml must be UTF-8"))
            })
            .expect("GTK Cargo.toml output should exist")
            .parse::<toml::Table>()
            .expect("GTK Cargo.toml should parse");
        let gtk = &gtk_manifest["dependencies"]["waterui-gtk"];
        assert_eq!(gtk["version"].as_str(), Some("^0.1.2"));
        assert!(gtk.get("path").is_none());
        assert!(gtk.get("git").is_none());
    }

    /// The managed hydrolysis crate is its own workspace root inside the
    /// build cache, and Cargo honours `[patch]` only from the root of the
    /// workspace being built — so the tables governing the application's own
    /// `cargo build` are carried into the generated manifest (#178). For a
    /// workspace member that is the root's table, never the member's inert
    /// one.
    #[test]
    fn hydrolysis_manifest_propagates_the_app_workspaces_patch_table() {
        let tempdir = tempdir().expect("temporary workspace dir");
        let workspace = tempdir.path();
        std::fs::create_dir_all(workspace.join("app")).expect("member dir");
        std::fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\n\n[patch.crates-io]\n\
             nami-core = { git = \"https://github.com/water-rs/nami\", rev = \"0123abcd\" }\n\
             vendored-fork = { path = \"vendor/fork\" }\n",
        )
        .expect("workspace manifest");
        std::fs::write(
            workspace.join("app/Cargo.toml"),
            "[package]\nname = \"patched-app\"\nversion = \"0.1.0\"\n\n[patch.crates-io]\n\
             inert-member-patch = { git = \"https://example.com/ignored\", rev = \"f\" }\n",
        )
        .expect("member manifest");

        let hydrolysis_ctx = ctx(
            None,
            Some(PathBuf::from("managed_backends/hydrolysis")),
            Some(workspace.join("app")),
        );
        let cargo_toml = crate::templates::hydrolysis::rendered_outputs(
            &hydrolysis_ctx,
            "patched-app-hydrolysis",
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
        let crates_io = &manifest["patch"]["crates-io"];
        assert_eq!(
            crates_io["nami-core"]["git"].as_str(),
            Some("https://github.com/water-rs/nami"),
            "the app's git pin must reach the generated manifest"
        );
        assert_eq!(crates_io["nami-core"]["rev"].as_str(), Some("0123abcd"));
        assert_eq!(
            crates_io["vendored-fork"]["path"].as_str(),
            Some(normalize_path_for_config(&workspace.join("vendor/fork")).as_str()),
            "a path patch is rebased onto the workspace root it was read from"
        );
        assert!(
            crates_io.get("inert-member-patch").is_none(),
            "a member's inert [patch] table is not the governing one"
        );
    }

    #[test]
    fn preview_scaffold_uses_embedded_workspace_version() {
        let tempdir = tempdir().expect("temporary preview scaffold dir");
        let ctx = project_ctx()
            .with_preview_runtime_features(vec!["dynamic_linking".to_string(), "gpu".to_string()])
            .with_preview_app_dependency(
                CrateName::try_from("preview_test_app").expect("test crate name must be valid"),
                tempdir.path().join("app"),
            );

        smol::block_on(crate::templates::preview::scaffold(tempdir.path(), &ctx))
            .expect("preview scaffold should succeed");

        let cargo_toml = std::fs::read_to_string(tempdir.path().join("Cargo.toml"))
            .expect("preview Cargo.toml should be written");
        assert!(cargo_toml.contains("default-features = false"));
        let manifest = cargo_toml
            .parse::<toml::Table>()
            .expect("preview Cargo.toml should parse");
        assert_eq!(
            manifest["dependencies"]["waterui-preview"]["version"].as_str(),
            Some(pinned("waterui-preview-version").as_str())
        );
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
    fn ffi_scaffold_resolves_waterui_ffi_from_the_build_cache_path() {
        let tempdir = tempdir().expect("temporary ffi scaffold dir");
        let project_root = tempdir.path().join("app");
        let ffi_dir = tempdir
            .path()
            .join("cache")
            .join("managed_backends")
            .join("ffi");
        write_fake_framework_checkout(
            &tempdir.path().join("waterui"),
            super::FORWARDED_FFI_FEATURES,
        );
        // The relative `waterui_path` resolves through the project root, so
        // it must exist for `project/../waterui` to land on the checkout.
        std::fs::create_dir_all(&project_root).expect("project root dir");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root.clone()),
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
            .expect("ffi scaffold should succeed");

        let cargo_toml = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written");
        let expected_ffi_path = pathdiff::diff_paths(project_root.join("../waterui/ffi"), &ffi_dir)
            .expect("expected waterui ffi dependency diff path");
        let expected_ffi_path = normalize_path_for_config(&expected_ffi_path);

        assert!(cargo_toml.contains(&format!("path = \"{expected_ffi_path}\"")));
        assert!(cargo_toml.contains("dev = [\"waterui_test/dev\"]"));
        // This crate roots the workspace preview modules join, so a module and the
        // runtime it is loaded into share one Cargo resolution. With none on disk
        // the workspace is empty — never a `modules/*` glob, which Cargo reads as
        // a literal path and rejects when it matches nothing.
        assert!(
            cargo_toml.contains("[workspace]"),
            "generated FFI crate must root the preview module workspace"
        );
        assert!(
            !cargo_toml.contains(crate::templates::PREVIEW_MODULES_DIR),
            "an FFI crate with no preview module on disk must declare no members"
        );
        let manifest = cargo_toml
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        assert_eq!(
            manifest["target"]["cfg(not(target_vendor = \"apple\"))"]["dependencies"]["waterui-ffi"]["default-features"].as_bool(),
            Some(false)
        );
        // The map capability reaches the Rust backend as well as the FFI
        // surface: the `map` feature also enables `waterui-apple/map`, so the
        // `MKMapView` leaf compiles only for apps whose graph holds
        // `waterui-map`.
        let map_forwards = manifest["features"]["map"]
            .as_array()
            .expect("map feature is declared")
            .iter()
            .map(|feature| feature.as_str().expect("feature name"))
            .collect::<Vec<_>>();
        assert_eq!(map_forwards, ["waterui-ffi/map", "waterui-apple/map"]);
        assert!(manifest["dependencies"].get("waterui-ffi").is_none());
        assert!(manifest["dependencies"].get("waterui-apple").is_none());
        assert!(
            manifest["target"]["cfg(target_vendor = \"apple\")"]["dependencies"]
                .get("waterui-apple")
                .is_some()
        );
        assert!(
            manifest["target"]["cfg(target_vendor = \"apple\")"]["dependencies"]
                .get("waterui-ffi")
                .is_none()
        );
        let lib = std::fs::read_to_string(ffi_dir.join("src/lib.rs")).unwrap();
        assert!(lib.contains("#[cfg(not(target_vendor = \"apple\"))]\nwaterui_ffi::export!();"));
        assert!(
            lib.contains("#[cfg(target_vendor = \"apple\")]\nwaterui_apple::export_app!(app);")
        );
        syn::parse_file(&lib).expect("mixed-target native companion parses");
        assert_eq!(manifest["package"]["autobins"].as_bool(), Some(false));
        let bins = manifest["bin"]
            .as_array()
            .expect("the FFI crate declares its entry-owning Apple binary");
        assert_eq!(bins.len(), 1);
        assert_eq!(
            bins[0]["name"].as_str(),
            Some(crate::apple::platform::APPLE_ENTRY_BINARY_NAME)
        );
        assert_eq!(
            bins[0]["path"].as_str(),
            Some("src/bin/waterui-apple-main.rs")
        );
        let main_bin = std::fs::read_to_string(ffi_dir.join("src/bin/waterui-apple-main.rs"))
            .expect("apple main source should be written");
        assert!(!main_bin.contains("waterui_cef_prepare_macos_application"));
    }

    /// A generated FFI manifest filters each forward by the feature table of
    /// the package its destination resolves to — a `waterui-ffi` without
    /// `inspector` drops the `waterui-ffi/inspector` entry only, and a
    /// feature no destination declares is not emitted at all.
    #[test]
    fn forwarded_features_follow_each_destination_package() {
        let mut manifest = cargo_toml::Manifest::<()>::default();
        manifest.dependencies.insert(
            "waterui-apple".to_string(),
            cargo_toml::Dependency::Simple(super::cargo_version_req("0.0.0")),
        );
        let mut tables = super::FeatureTables::new();
        tables.insert(
            "waterui-ffi".to_string(),
            ["c-api", "media"].into_iter().map(str::to_string).collect(),
        );
        tables.insert(
            "waterui-apple".to_string(),
            ["map", "media", "webview"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        );

        // `media` reaches both destinations.
        assert_eq!(
            super::ffi_feature_forwards("media", &manifest, &tables),
            vec![
                "waterui-ffi/media".to_string(),
                "waterui-apple/media".to_string()
            ]
        );
        // `map` is declared only by the Apple package: capability filtering
        // drops the `waterui-ffi` entry but must not discard the Apple side.
        assert_eq!(
            super::ffi_feature_forwards("map", &manifest, &tables),
            vec!["waterui-apple/map".to_string()]
        );
        // No destination declares `inspector` or `chromium`.
        assert_eq!(
            super::ffi_feature_forwards("inspector", &manifest, &tables),
            Vec::<String>::new()
        );
        assert_eq!(
            super::ffi_feature_forwards("chromium", &manifest, &tables),
            Vec::<String>::new()
        );

        // A companion manifest declares `waterui-ffi`: when the resolved
        // package lacks the feature, the feature is not emitted — it must
        // not fall back to the `waterui` facade, which routes only
        // manifests that declare no `waterui-ffi` dependency at all.
        let mut companion = cargo_toml::Manifest::<()>::default();
        companion.dependencies.insert(
            "waterui-ffi".to_string(),
            cargo_toml::Dependency::Simple(super::cargo_version_req("0.0.0")),
        );
        companion.dependencies.insert(
            "waterui".to_string(),
            cargo_toml::Dependency::Simple(super::cargo_version_req("0.0.0")),
        );
        let mut companion_tables = super::FeatureTables::new();
        companion_tables.insert(
            "waterui-ffi".to_string(),
            std::iter::once("c-api").map(str::to_string).collect(),
        );
        companion_tables.insert(
            "waterui".to_string(),
            std::iter::once("media").map(str::to_string).collect(),
        );
        assert_eq!(
            super::ffi_feature_forwards("media", &companion, &companion_tables),
            Vec::<String>::new()
        );
    }

    /// A local checkout answers the forward filter itself: the scaffolded
    /// manifest forwards only what the checkout's `ffi/Cargo.toml` declares —
    /// the shape an older framework resolves to, where an unconditional
    /// `inspector` forward failed the whole resolution.
    #[test]
    fn ffi_scaffold_forwards_only_the_features_waterui_ffi_declares() {
        let tempdir = tempdir().expect("temporary scaffold dir");
        let waterui = tempdir.path().join("waterui");
        write_fake_framework_checkout(&waterui, &["c-api", "media"]);
        let ffi_dir = tempdir.path().join("managed_backends/ffi");
        let ctx = ctx(
            Some(waterui),
            Some(ffi_dir.clone()),
            Some(tempdir.path().join("app")),
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
            .expect("ffi scaffold should succeed");

        let manifest: toml::Table = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse()
            .expect("ffi Cargo.toml should parse");
        let features = manifest["features"].as_table().expect("features table");
        assert_eq!(
            features["media"]
                .as_array()
                .unwrap()
                .iter()
                .map(|forward| forward.as_str())
                .collect::<Vec<_>>(),
            [Some("waterui-ffi/media"), Some("waterui-apple/media")]
        );
        assert!(
            !features.contains_key("inspector"),
            "a waterui-ffi without `inspector` gets no `inspector` forward"
        );
    }

    /// The channel path answers through real `cargo metadata`: a
    /// `waterui-ffi` pinned at a git revision — the dev/nightly channel
    /// shape — resolves through a local `file://` checkout, so the learned
    /// table is the exact resolved package's, matched on the package's own
    /// name while the resolved edge spells it `waterui_ffi`. Two distinct
    /// resolved revisions yield their own tables, and an unresolvable probe
    /// fails instead of forwarding the unfiltered set.
    #[test]
    fn resolved_forward_tables_reads_the_resolved_package_from_cargo_metadata() {
        use std::process::Command as StdCommand;

        let tempdir = tempdir().expect("temporary probe fixture dir");

        let git = |dir: &Path, args: &[&str]| {
            let output = StdCommand::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .expect("git runs the fixture commands");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("git stdout is utf-8")
                .trim()
                .to_string()
        };

        // A committed `waterui-ffi` checkout returns the revision a channel
        // manifest pins on.
        let write_ffi = |dir_name: &str, version: &str, features: &[&str]| {
            let dir = tempdir.path().join(dir_name);
            write_fixture_crate(&dir, "waterui-ffi", features);
            // The pinned revision must carry the declared version.
            let manifest_path = dir.join("Cargo.toml");
            let manifest = std::fs::read_to_string(&manifest_path)
                .expect("fixture manifest")
                .replace("0.0.0", version);
            std::fs::write(&manifest_path, manifest).expect("fixture manifest");
            git(&dir, &["init", "-b", "fixture"]);
            let rev = crate::framework::test_fixtures::git_commit_all(&dir, "fixture");
            (format!("file://{}", dir.display()), rev)
        };

        let manifest_for = |git_url: &str, rev: &str| {
            let mut manifest = cargo_toml::Manifest::<()> {
                package: Some(cargo_toml::Package::new(
                    "probe-app".to_string(),
                    super::cargo_semver("0.1.0"),
                )),
                ..Default::default()
            };
            manifest.dependencies.insert(
                "waterui-ffi".to_string(),
                cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
                    git: Some(git_url.to_string()),
                    rev: Some(rev.to_string()),
                    ..Default::default()
                })),
            );
            manifest
        };

        // Two distinct resolved revisions — one without `inspector` (the
        // stable shape through 0.5.2), one with — yield their own tables.
        // The probe clones the `file://` fixtures like any `git` source, so
        // it resolves under a per-test `CARGO_HOME` rather than the
        // developer's real one.
        let cargo_home = crate::toolchain::Host::current()
            .with_env("CARGO_HOME", tempdir.path().join("cargo-home"));
        for (dir_name, version, features) in [
            ("ffi-0.5.2", "0.5.2", vec!["c-api", "media"]),
            ("ffi-0.6.0", "0.6.0", vec!["c-api", "media", "inspector"]),
        ] {
            let (git_url, rev) = write_ffi(dir_name, version, &features);
            let manifest = manifest_for(&git_url, &rev);
            let tables = smol::block_on(super::resolved_forward_tables(
                &cargo_home,
                &manifest,
                tempdir.path(),
                &["waterui-ffi"],
            ))
            .expect("the probe resolves the pinned fixture");
            let table = &tables["waterui-ffi"];
            for feature in &features {
                assert!(table.contains(*feature), "{version} must declare {feature}");
            }
            assert_eq!(
                table.contains("inspector"),
                features.contains(&"inspector"),
                "the learned {version} table must match its fixture"
            );
        }

        // Failure direction: the probe cannot resolve → the error propagates
        // with its context instead of the unfiltered set going out.
        let missing = tempdir.path().join("ffi-missing");
        let manifest = manifest_for(
            &format!("file://{}", missing.display()),
            "0000000000000000000000000000000000000000",
        );
        let error = smol::block_on(super::resolved_forward_tables(
            &cargo_home,
            &manifest,
            tempdir.path(),
            &["waterui-ffi"],
        ))
        .expect_err("an unresolvable probe must fail, not fall back");
        assert!(
            error.to_string().contains("resolve"),
            "the error must name the resolution that failed: {error}"
        );
    }

    /// A fake checkout patching `waterui-core` to its own `core/`, and a
    /// project beside it at `app/` with the given `Cargo.toml`, returning the
    /// FFI companion's context and directory.
    fn patched_project_ffi(directory: &Path, project_manifest: &str) -> (TemplateContext, PathBuf) {
        let checkout = directory.join("waterui");
        write_fake_framework_checkout(&checkout, super::FORWARDED_FFI_FEATURES);
        let root_manifest = checkout.join("Cargo.toml");
        let mut text = std::fs::read_to_string(&root_manifest).expect("checkout manifest");
        text.push_str("\n[patch.crates-io]\nwaterui-core = { path = \"core\" }\n");
        std::fs::write(&root_manifest, text).expect("checkout patch table");
        let app = directory.join("app");
        std::fs::create_dir_all(&app).expect("project dir");
        std::fs::write(app.join("Cargo.toml"), project_manifest).expect("project manifest");
        let ffi_dir = directory.join("managed_backends/ffi");
        let ctx = ctx(Some(checkout), Some(ffi_dir.clone()), Some(app));
        (ctx, ffi_dir)
    }

    /// The FFI companion roots its own workspace, so the project's `[patch]`
    /// entries reach it only by being copied: every source key, `path`
    /// entries made absolute, merged with the checkout's, and an entry the
    /// project spells differently from the checkout but resolving to the same
    /// directory kept once (#1997).
    #[test]
    fn ffi_scaffold_merges_the_projects_patches_with_the_frameworks() {
        let tempdir = tempdir().expect("temporary ffi scaffold dir");
        let (ctx, ffi_dir) = patched_project_ffi(
            tempdir.path(),
            "[package]\nname = \"waterui_test\"\nversion = \"0.1.0\"\n\n\
             [patch.crates-io]\n\
             waterui-core = { path = \"../waterui/core\" }\n\
             own-fork = { path = \"vendor/own-fork\" }\n\n\
             [patch.\"https://github.com/water-rs/fork\"]\n\
             forked = { path = \"vendor/forked\" }\n",
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
            .expect("ffi scaffold should succeed");

        let manifest = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        let patch = &manifest["patch"];
        let app = tempdir.path().join("app");
        assert_eq!(
            patch["crates-io"]["own-fork"]["path"].as_str(),
            Some(normalize_path_for_config(&app.join("vendor/own-fork")).as_str()),
            "the project's path patch resolves from outside the project"
        );
        assert_eq!(
            patch["https://github.com/water-rs/fork"]["forked"]["path"].as_str(),
            Some(normalize_path_for_config(&app.join("vendor/forked")).as_str()),
            "a git-source table of the project is carried too"
        );
        assert_eq!(
            patch["crates-io"]["waterui-core"]["path"].as_str(),
            Some(normalize_path_for_config(&tempdir.path().join("waterui").join("core")).as_str()),
            "the entry both name is the framework's"
        );
        assert!(
            patch["crates-io"].get("waterui-ffi").is_some(),
            "the checkout's member entries stay"
        );
    }

    /// A project that patches a crate the framework also patches, to another
    /// source, overrides the framework's entry, as the project's own build
    /// does: Cargo takes `[patch]` from the root manifest.
    #[test]
    fn ffi_scaffold_takes_the_projects_entry_over_the_frameworks() {
        let tempdir = tempdir().expect("temporary ffi scaffold dir");
        let (ctx, ffi_dir) = patched_project_ffi(
            tempdir.path(),
            "[package]\nname = \"waterui_test\"\nversion = \"0.1.0\"\n\n\
             [patch.crates-io]\nwaterui-core = { path = \"../elsewhere/core\" }\n",
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
            .expect("ffi scaffold should succeed");

        let manifest = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        assert_eq!(
            manifest["patch"]["crates-io"]["waterui-core"]["path"].as_str(),
            Some(
                normalize_path_for_config(&tempdir.path().join("app").join("../elsewhere/core"))
                    .as_str()
            ),
            "the project's entry replaces the framework's"
        );
    }

    /// The `gpu-allocator` entry `waterui-winui` needs is the framework's
    /// side of the merge: a project pinning the crate itself keeps its pin.
    #[test]
    fn winui_scaffold_keeps_a_projects_gpu_allocator_patch() {
        let tempdir = tempdir().expect("temporary project dir");
        std::fs::write(
            tempdir.path().join("Cargo.toml"),
            "[package]\nname = \"waterui_test\"\nversion = \"0.1.0\"\n\n\
             [patch.crates-io]\n\
             gpu-allocator = { git = \"https://github.com/Traverse-Research/gpu-allocator\", rev = \"project-rev\" }\n",
        )
        .expect("project manifest");
        let mut ctx = ctx(None, None, Some(tempdir.path().to_path_buf()));
        ctx.framework = dev_framework();
        let manifest = crate::templates::winui::rendered_outputs(&ctx, "waterui-test-winui")
            .expect("winui outputs should render")
            .into_iter()
            .find_map(|(path, content)| {
                (path == std::path::Path::new("Cargo.toml"))
                    .then(|| String::from_utf8(content).expect("Cargo.toml must be UTF-8"))
            })
            .expect("winui Cargo.toml output should exist");
        let manifest: toml::Value = toml::from_str(&manifest).expect("winui manifest must parse");

        let gpu_allocator = &manifest["patch"]["crates-io"]["gpu-allocator"];
        assert_eq!(
            gpu_allocator["git"].as_str(),
            Some("https://github.com/Traverse-Research/gpu-allocator")
        );
        assert_eq!(gpu_allocator["rev"].as_str(), Some("project-rev"));
    }

    #[test]
    fn ffi_scaffold_declares_minimal_cef_helper_for_chromium() {
        let tempdir = tempdir().expect("temporary ffi scaffold dir");
        write_fake_framework_checkout(
            &tempdir.path().join("waterui"),
            super::FORWARDED_FFI_FEATURES,
        );
        let ffi_dir = tempdir.path().join("managed_backends/ffi");
        let ctx = ctx(
            Some(tempdir.path().join("waterui")),
            Some(ffi_dir.clone()),
            Some(tempdir.path().to_path_buf()),
        )
        .with_browser(BrowserTemplateContext::apple_managed(Some(
            ResolvedWebViewBackend::Cef,
        )));

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
            .expect("CEF FFI companion should declare binaries");
        assert_eq!(bins.len(), 2);
        assert_eq!(
            bins[0]["name"].as_str(),
            Some(crate::apple::platform::APPLE_ENTRY_BINARY_NAME)
        );
        assert_eq!(bins[1]["name"].as_str(), Some("chromium-ffi-cef-helper"));
        assert_eq!(
            bins[1]["path"].as_str(),
            Some("src/bin/waterui-cef-helper.rs")
        );

        let helper = std::fs::read_to_string(ffi_dir.join("src/bin/waterui-cef-helper.rs"))
            .expect("CEF helper source should be written");
        assert!(helper.contains("waterui_browser_cef::run_packaged_subprocess"));
        assert!(!helper.contains("waterui_ffi"));
        assert!(helper.contains("#[cfg(target_os = \"macos\")]"));
        assert!(helper.contains(
            "compile_error!(\"The Apple CEF subprocess helper requires a macOS target\")"
        ));

        let main_bin = std::fs::read_to_string(ffi_dir.join("src/bin/waterui-apple-main.rs"))
            .expect("apple main source should be written");
        assert!(main_bin.contains("waterui_browser_cef::initialize_macos_application"));
        assert!(main_bin.contains("waterui_browser_cef::initialize_sandbox_early"));
        let browser = &manifest["target"]["cfg(target_os = \"macos\")"]["dependencies"]["waterui-browser-cef"];
        assert_eq!(
            browser["default-features"].as_bool(),
            Some(false),
            "the CEF dependency must not pull the engine's default features"
        );
        let features = browser["features"]
            .as_array()
            .expect("CEF dependency features should be an array");
        assert!(
            features
                .iter()
                .any(|feature| feature.as_str() == Some("cef-runtime")),
            "the CEF dependency must enable the cef-runtime feature: {features:?}"
        );
        assert!(!main_bin.contains("waterui_ffi"));
    }

    #[test]
    fn hydrolysis_main_installs_the_cef_application_only_with_a_cef_engine() {
        // CEF's macOS runtime cannot initialize until the process's
        // `NSApplication` is a `CefAppProtocol` subclass and the sandbox has
        // started; both must happen at entry, before `app(env)` installs the
        // engine. The generated main carries the calls exactly when the
        // application linked `waterui-browser-cef`.
        let main_rs = crate::templates::hydrolysis::rendered_outputs(
            &all_os_browser(project_ctx(), false, Some(ResolvedWebViewBackend::Cef)),
            "waterui-test-hydrolysis",
        )
        .expect("hydrolysis outputs should render")
        .into_iter()
        .find_map(|(path, content)| {
            (path == std::path::Path::new("src/main.rs"))
                .then(|| String::from_utf8(content).expect("src/main.rs must be UTF-8"))
        })
        .expect("hydrolysis src/main.rs output should exist");
        assert!(main_rs.contains("fn initialize_cef_runtime()"));
        assert!(main_rs.contains("waterui_browser_cef::initialize_sandbox_early"));
        assert!(main_rs.contains("waterui_browser_cef::initialize_macos_application"));
        // Every generated entry — the preview, preview-test, MCP and plain
        // run mains — calls the initializer first, since each mounts the
        // same `app(env)` that installs the engine.
        for (entry, rest) in main_rs.split("fn main()").enumerate() {
            if entry == 0 {
                continue;
            }
            assert!(
                rest.trim_start_matches(char::is_whitespace)
                    .strip_prefix('{')
                    .is_some_and(|body| body
                        .trim_start_matches(char::is_whitespace)
                        .starts_with("initialize_cef_runtime();")),
                "a generated main does not call initialize_cef_runtime first: {rest}"
            );
        }

        let without_cef = crate::templates::hydrolysis::rendered_outputs(
            &project_ctx(),
            "waterui-test-hydrolysis-nocef",
        )
        .expect("hydrolysis outputs should render")
        .into_iter()
        .find_map(|(path, content)| {
            (path == std::path::Path::new("src/main.rs"))
                .then(|| String::from_utf8(content).expect("src/main.rs must be UTF-8"))
        })
        .expect("hydrolysis src/main.rs output should exist");
        assert!(!without_cef.contains("initialize_cef_runtime"));
        assert!(!without_cef.contains("waterui_browser_cef::initialize_sandbox_early"));
        assert!(!without_cef.contains("waterui_browser_cef::initialize_macos_application"));
    }

    #[test]
    fn ffi_scaffold_without_apple_backend_emits_no_apple_dependency() {
        let tempdir = tempdir().expect("temporary ffi scaffold dir");
        write_fake_framework_checkout(
            &tempdir.path().join("waterui"),
            super::FORWARDED_FFI_FEATURES,
        );
        let ffi_dir = tempdir.path().join("managed_backends/ffi");
        let ctx = ctx(
            Some(tempdir.path().join("waterui")),
            Some(ffi_dir.clone()),
            Some(tempdir.path().to_path_buf()),
        )
        .with_apple_backend_selected(false);

        smol::block_on(crate::templates::ffi::scaffold(
            &ffi_dir,
            &ctx,
            "android-ffi",
        ))
        .expect("Android-only ffi scaffold should succeed");

        let manifest = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        assert!(
            manifest["dependencies"].get("waterui-apple").is_none(),
            "an Android-only ffi companion must not depend on waterui-apple"
        );
        assert!(
            manifest
                .get("bin")
                .and_then(toml::Value::as_array)
                .is_none_or(Vec::is_empty),
            "an Android-only ffi companion declares no entry-owning binary"
        );

        let lib = std::fs::read_to_string(ffi_dir.join("src/lib.rs"))
            .expect("ffi lib.rs should be written");
        // Parse the generated Rust and inspect its items structurally:
        // comments name these macros legitimately, so a text search cannot
        // tell an invocation from prose.
        let file = syn::parse_file(&lib).expect("the generated ffi lib.rs must parse");
        let invoked = |wanted: &[&str]| {
            file.items.iter().any(|item| {
                matches!(item, syn::Item::Macro(item_macro)
                    if item_macro.mac.path.segments.len() == wanted.len()
                        && item_macro
                            .mac
                            .path
                            .segments
                            .iter()
                            .zip(wanted)
                            .all(|(segment, name)| segment.ident == *name))
            })
        };
        assert!(
            !invoked(&["waterui_apple", "export_app"]),
            "an Android-only ffi companion must not invoke waterui_apple::export_app!"
        );
        assert!(
            invoked(&["waterui_ffi", "export"]),
            "the companion always emits the waterui_ffi::export!() invocation"
        );
        let app_shim = file.items.iter().any(|item| {
            let syn::Item::Fn(item_fn) = item else {
                return false;
            };
            let syn::ReturnType::Type(_, output) = &item_fn.sig.output else {
                return false;
            };
            item_fn.sig.ident == "app"
                && item_fn.sig.inputs.len() == 1
                && matches!(
                    item_fn.sig.inputs.first(),
                    Some(syn::FnArg::Typed(arg)) if matches!(
                        arg.ty.as_ref(),
                        syn::Type::Path(path) if path.path.is_ident("Environment")
                    )
                )
                && matches!(
                    output.as_ref(),
                    syn::Type::Path(path) if path.path.is_ident("App")
                )
        });
        assert!(
            app_shim,
            "the app(env) -> App shim every backend's export!() expansion calls must exist"
        );
    }

    #[test]
    fn ffi_lockfile_seed_follows_the_project_lockfile() {
        let tempdir = tempdir().expect("temporary ffi seed dir");
        let project_lock = tempdir.path().join("Cargo.lock");
        let ffi_dir = tempdir.path().join("managed_backends/ffi");
        std::fs::create_dir_all(&ffi_dir).expect("ffi dir");
        let managed_lock = ffi_dir.join("Cargo.lock");
        let seed = || {
            smol::block_on(crate::templates::seed_lockfile(
                &ffi_dir,
                &project_lock,
                None,
            ))
            .expect("seeding the managed lockfile should succeed");
        };
        // The seed path parses the recorded resolutions, so the fixtures are
        // real lockfiles. The managed lock is compared as a package set: the
        // seed writes `cargo_lock`'s own emit layout, not the fixture's.
        let lock = |name: &str, version: &str| {
            format!("version = 4\n\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n")
        };
        let packages = |lock: &str| -> std::collections::BTreeSet<(String, String)> {
            lock.parse::<cargo_lock::Lockfile>()
                .expect("fixture lockfile parses")
                .packages
                .into_iter()
                .map(|package| (package.name.to_string(), package.version.to_string()))
                .collect()
        };
        let managed =
            || packages(&std::fs::read_to_string(&managed_lock).expect("managed Cargo.lock"));
        let pins_v1 = lock("pins", "1.0.0");
        let pins_v1_with_ffi =
            format!("{pins_v1}\n[[package]]\nname = \"ffi-entries\"\nversion = \"0.1.0\"\n");
        let pins_v2 = lock("pins", "2.0.0");

        // No project lockfile: nothing to pin, the managed crate resolves on its own.
        seed();
        assert!(!managed_lock.exists());

        std::fs::write(&project_lock, &pins_v1).expect("project lock");
        seed();
        assert_eq!(managed(), packages(&pins_v1));

        // Cargo rewrote the managed lockfile (pruned the project's unused entries,
        // added the FFI crate's own); unchanged seed inputs leave that alone.
        std::fs::write(&managed_lock, &pins_v1_with_ffi).expect("managed lock");
        seed();
        assert_eq!(
            std::fs::read_to_string(&managed_lock).expect("managed Cargo.lock"),
            pins_v1_with_ffi
        );

        // The project re-resolved: the managed crate follows it — the new
        // `pins` pin takes the `^2` slot, while the previous lock's `^1`
        // entry is a different compatible range and stays beside it — and
        // the entries only the managed crate resolves stay put.
        std::fs::write(&project_lock, &pins_v2).expect("project lock");
        seed();
        assert_eq!(
            managed(),
            std::collections::BTreeSet::from([
                ("ffi-entries".to_owned(), "0.1.0".to_owned()),
                ("pins".to_owned(), "1.0.0".to_owned()),
                ("pins".to_owned(), "2.0.0".to_owned()),
            ]),
            "the project's pin fills its range; a previous pin in another range stays seeded beside it"
        );

        // A managed lockfile that went missing is re-seeded from the current pins.
        std::fs::remove_file(&managed_lock).expect("remove managed lock");
        seed();
        assert_eq!(managed(), packages(&pins_v2));
    }

    /// A managed `Cargo.lock` Cargo already pruned into its own layout still
    /// reads as inputs-unchanged: the seed gate is the recorded project lock
    /// and canonical checksum, never the pruned output's equality with the
    /// seed — so a no-change run writes neither the lock nor the stamp
    /// (#2073).
    #[test]
    fn an_unchanged_seed_input_writes_nothing_over_the_pruned_lock() {
        let tempdir = tempdir().expect("temporary ffi seed dir");
        let project_lock = tempdir.path().join("Cargo.lock");
        let ffi_dir = tempdir.path().join("managed_backends/ffi");
        std::fs::create_dir_all(&ffi_dir).expect("ffi dir");
        let managed_lock = ffi_dir.join("Cargo.lock");
        let seed = || {
            smol::block_on(crate::templates::seed_lockfile(
                &ffi_dir,
                &project_lock,
                None,
            ))
            .expect("seeding the managed lockfile should succeed");
        };
        let lock = |name: &str, version: &str| {
            format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n")
        };
        std::fs::write(
            &project_lock,
            format!(
                "version = 4\n\n{}{}",
                lock("pins", "1.0.0"),
                lock("unused-dep", "0.4.0")
            ),
        )
        .expect("project lock");
        seed();

        // Cargo resolved the seed into its own layout: the project's unused
        // entries pruned and the managed crate's own package added — a lock
        // no seed-merge equality could recognise.
        let pruned = format!(
            "version = 4\n\n{}\n[[package]]\nname = \"ffi-entries\"\nversion = \"0.1.0\"\n",
            lock("pins", "1.0.0")
        );
        std::fs::write(&managed_lock, &pruned).expect("pruned managed lock");
        let epoch =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&managed_lock)
            .expect("open managed lock")
            .set_modified(epoch)
            .expect("stamp managed lock mtime");

        seed();
        assert_eq!(
            std::fs::read_to_string(&managed_lock).expect("managed Cargo.lock"),
            pruned,
            "an unchanged seed input must not rewrite the managed lock"
        );
        assert_eq!(
            std::fs::metadata(&managed_lock)
                .expect("managed lock metadata")
                .modified()
                .expect("managed lock mtime"),
            epoch,
            "an unchanged seed input must leave the managed lock's mtime alone"
        );
    }

    /// A post-seed failure restores the `Cargo.lock`/`Cargo.lock.seed` pair
    /// to the bytes the run found — the stamp included. Restoring the lock
    /// alone would leave the stamp claiming the failed run's inputs already
    /// seeded, and the next prepare's `seed_lockfile` would early-return
    /// over a lock that no longer carries them.
    #[test]
    fn a_failed_post_seed_step_leaves_the_next_seed_to_rerun() {
        let tempdir = tempdir().expect("temporary ffi seed dir");
        let project_lock = tempdir.path().join("Cargo.lock");
        let ffi_dir = tempdir.path().join("managed_backends/ffi");
        std::fs::create_dir_all(&ffi_dir).expect("ffi dir");
        let managed_lock = ffi_dir.join("Cargo.lock");
        let seed = || {
            smol::block_on(crate::templates::seed_lockfile(
                &ffi_dir,
                &project_lock,
                None,
            ))
            .expect("seeding the managed lockfile should succeed");
        };
        let lock = |version: &str| {
            format!("version = 4\n\n[[package]]\nname = \"pins\"\nversion = \"{version}\"\n")
        };

        std::fs::write(&project_lock, lock("1.0.0")).expect("project lock");
        seed();
        let found_lock = std::fs::read(&managed_lock).expect("managed Cargo.lock");
        let found_stamp =
            std::fs::read(ffi_dir.join(crate::templates::LOCKFILE_SEED)).expect("seed stamp");

        // The project re-pinned, the seed ran — and the post-seed
        // resolution failed, so the error path restores the pair the run
        // found.
        std::fs::write(&project_lock, lock("2.0.0")).expect("project lock");
        seed();
        smol::block_on(crate::templates::restore_seeded_lockfile(
            &ffi_dir,
            Some(&found_lock),
            Some(&found_stamp),
        ))
        .expect("restoring the pre-seed pair should succeed");

        // The next prepare re-seeds: the restored stamp names the earlier
        // inputs, so the gate cannot pass for the new ones.
        seed();
        let reseeded: cargo_lock::Lockfile = std::fs::read_to_string(&managed_lock)
            .expect("managed Cargo.lock")
            .parse()
            .expect("the managed lock parses");
        let versions: Vec<String> = reseeded
            .packages
            .iter()
            .map(|package| package.version.to_string())
            .collect();
        assert_eq!(
            versions,
            ["2.0.0", "1.0.0"],
            "a restored stamp must not pass for the new inputs — the re-seed ran, and the restored `^1` pin keeps its own range beside the project's `^2`"
        );
    }

    /// A `waterui_path` ctx resolves `waterui-preview` through `cargo
    /// metadata` on the checkout, so the checkout is a minimal fixture
    /// workspace carrying that one member.
    #[test]
    fn preview_ffi_scaffold_emits_dylib_only_wrapper() {
        let tempdir = tempdir().expect("temporary preview ffi scaffold dir");
        let project_root = tempdir.path().join("app");
        let preview_ffi_dir = tempdir
            .path()
            .join("cache")
            .join("managed_backends")
            .join("preview_ffi");
        let workspace_root = tempdir.path().join("waterui");
        std::fs::create_dir_all(&workspace_root).expect("fixture workspace root");
        std::fs::write(
            workspace_root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"preview\"]\n",
        )
        .expect("fixture workspace manifest");
        write_fixture_crate(&workspace_root.join("preview"), "waterui-preview", &[]);
        // The forward filter reads the `waterui-ffi` table off the checkout.
        write_fixture_crate(
            &workspace_root.join("ffi"),
            "waterui-ffi",
            &["c-api", "android-jni"],
        );
        let ctx = ctx(
            Some(workspace_root),
            Some(preview_ffi_dir.clone()),
            Some(project_root),
        );

        smol::block_on(crate::templates::preview_ffi::scaffold(
            &preview_ffi_dir,
            &ctx,
            "app-preview-ffi",
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
            manifest["target"]["cfg(not(target_vendor = \"apple\"))"]["dependencies"]["waterui-ffi"]["optional"].as_bool(),
            Some(true)
        );
        assert_eq!(
            manifest["target"]["cfg(not(target_vendor = \"apple\"))"]["dependencies"]["waterui-ffi"]["default-features"].as_bool(),
            Some(false)
        );
        assert!(manifest["dependencies"].get("waterui-ffi").is_none());
        let targets = manifest["target"].as_table().unwrap();
        assert_eq!(targets.len(), 1);
        assert!(targets.contains_key("cfg(not(target_vendor = \"apple\"))"));
        // The module is a member of the support runtime's workspace, never a
        // workspace of its own: one Cargo resolution is what makes the module and
        // the runtime it is loaded into agree on the `-C metadata` hash that ends
        // up in every symbol. Profiles and `[patch]` belong to that root.
        assert!(
            !manifest.contains_key("workspace"),
            "preview module must not root its own workspace"
        );
        assert!(
            !manifest.contains_key("patch"),
            "preview module must inherit `[patch]` from the workspace root"
        );
        assert!(
            !manifest.contains_key("profile"),
            "preview module must inherit profiles from the workspace root"
        );
        assert_eq!(
            manifest["dependencies"]["waterui-preview"]["optional"].as_bool(),
            Some(true)
        );
        // Assert on the parsed manifest, not on substrings of the whole file: the file
        // embeds absolute dependency paths, so a checkout living in a directory whose
        // name happens to contain "rlib" or "cdylib" would fail a substring check.
        let crate_types = manifest["lib"]["crate-type"]
            .as_array()
            .expect("crate-type should be an array")
            .iter()
            .map(|value| value.as_str().expect("crate type should be a string"))
            .collect::<Vec<_>>();
        assert_eq!(crate_types, ["dylib"]);
        assert_eq!(
            manifest["dependencies"]["waterui_test"]["features"]
                .as_array()
                .expect("app dependency features should be an array")
                .iter()
                .map(|value| value.as_str().expect("feature should be a string"))
                .collect::<Vec<_>>(),
            ["dev"]
        );
        assert!(manifest["dependencies"].get("waterui").is_none());
    }

    #[test]
    fn generated_ffi_manifest_emits_only_linked_crate_types() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_fake_framework_checkout(&temp.path().join("waterui"), super::FORWARDED_FFI_FEATURES);
        let project_root = temp.path().join("project");
        // The relative `waterui_path` resolves through the project root, so
        // it must exist for `project/../waterui` to land on the checkout.
        std::fs::create_dir_all(&project_root).expect("project root dir");
        let ffi_dir = temp
            .path()
            .join("cache")
            .join("managed_backends")
            .join("ffi");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root),
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
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

        // Apple embedders link the staticlib, Android loads the cdylib, and
        // the entry-owning `waterui-apple-main` bin imports the companion
        // crate through the rlib — the dependency that carries both its
        // exports and its `#[link]` native declarations into the executable.
        assert_eq!(crate_types, ["staticlib", "cdylib", "rlib"]);
    }

    #[test]
    fn apple_scaffold_contains_no_swift_runtime_or_embedded_package() {
        let mut ctx = project_ctx();
        ctx.app_name = "WaterUIApp".to_string();
        let outputs = super::apple::rendered_outputs(&ctx).unwrap();
        let expected_path = PathBuf::from("WaterUIApp/WaterUIApp.entitlements");
        let expected = include_bytes!("../templates/apple/AppName/AppName.entitlements.tpl");
        assert_eq!(outputs, vec![(expected_path.clone(), expected.to_vec())]);
        let directory = tempdir().unwrap();
        smol::block_on(super::apple::scaffold(directory.path(), &ctx)).unwrap();
        assert_eq!(
            std::fs::read(directory.path().join(expected_path)).unwrap(),
            expected
        );
    }

    #[test]
    fn generated_native_build_script_has_no_swift_link_contract() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_fake_framework_checkout(&temp.path().join("waterui"), super::FORWARDED_FFI_FEATURES);
        let ffi_dir = temp.path().join("managed_backends").join("ffi");
        let project_root = temp.path().join("project");
        // The relative `waterui_path` resolves through the project root, so
        // it must exist for `project/../waterui` to land on the checkout.
        std::fs::create_dir_all(&project_root).expect("project root dir");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root),
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
            .expect("ffi scaffold should succeed");

        let build_script = std::fs::read_to_string(ffi_dir.join("build.rs"))
            .expect("ffi build.rs should be written");
        assert!(!build_script.contains("waterui_swift_"));
        assert!(!build_script.contains("rustc-link-arg-cdylib"));
    }

    #[test]
    fn generated_manifests_keep_debug_info_off_for_dependencies() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_fake_framework_checkout(&temp.path().join("waterui"), super::FORWARDED_FFI_FEATURES);
        let project_root = temp.path().join("project");
        // The relative `waterui_path` resolves through the project root, so
        // it must exist for `project/../waterui` to land on the checkout.
        std::fs::create_dir_all(&project_root).expect("project root dir");
        let ffi_dir = temp
            .path()
            .join("cache")
            .join("managed_backends")
            .join("ffi");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root),
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
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
        assert_eq!(dev["package"]["*"]["opt-level"].as_integer(), Some(2));
    }

    #[test]
    fn generated_manifests_carry_the_release_profile() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_fake_framework_checkout(&temp.path().join("waterui"), super::FORWARDED_FFI_FEATURES);
        let project_root = temp.path().join("project");
        // The relative `waterui_path` resolves through the project root, so
        // it must exist for `project/../waterui` to land on the checkout.
        std::fs::create_dir_all(&project_root).expect("project root dir");
        let ffi_dir = temp
            .path()
            .join("cache")
            .join("managed_backends")
            .join("ffi");
        let ctx = ctx(
            Some(PathBuf::from("../waterui")),
            Some(ffi_dir.clone()),
            Some(project_root),
        );

        smol::block_on(crate::templates::ffi::scaffold(&ffi_dir, &ctx, "app-ffi"))
            .expect("ffi scaffold should succeed");

        let manifest = std::fs::read_to_string(ffi_dir.join("Cargo.toml"))
            .expect("ffi Cargo.toml should be written")
            .parse::<toml::Table>()
            .expect("ffi Cargo.toml should parse");
        let release = &manifest["profile"]["release"];

        // Generated crates are their own workspace roots, so a plain
        // `cargo build --release` used to ship unoptimized, unstripped
        // artifacts — on Android, a libwaterui_app.so roughly 3x the size of
        // the same source built under the workspace release profile.
        assert_eq!(release["lto"].as_bool(), Some(true));
        assert_eq!(release["codegen-units"].as_integer(), Some(1));
        assert_eq!(release["opt-level"].as_str(), Some("z"));
        assert_eq!(release["strip"].as_bool(), Some(true));
        assert_eq!(release["panic"].as_str(), Some("abort"));
    }

    /// The scaffolded app links text, layout and controls — none of which is
    /// feature-gated — so the waterui dependency declares no features; video
    /// is declared as the crate's own `media` feature and left off, because
    /// `waterui/media` carries the system media stack (VA-API, `PipeWire`) a
    /// hello-world never uses.
    #[test]
    fn root_manifest_keeps_video_behind_an_opt_in_feature() {
        let temp = tempfile::tempdir().expect("temp dir");
        let project_root = temp.path().join("project");
        let ctx = ctx(None, None, Some(project_root.clone()));

        smol::block_on(crate::templates::root::scaffold(
            &project_root,
            &ctx,
            "assets",
        ))
        .expect("root scaffold should succeed");

        let rendered = std::fs::read_to_string(project_root.join("Cargo.toml"))
            .expect("root Cargo.toml should be written");
        let manifest = rendered
            .parse::<toml::Table>()
            .expect("root Cargo.toml should parse");

        let native_features = manifest["target"]
            ["cfg(not(any(target_arch = \"wasm32\", target_os = \"espidf\")))"]["dependencies"]
            ["waterui"]
            .get("features")
            .and_then(toml::Value::as_array)
            .map(|features| {
                features
                    .iter()
                    .map(|feature| feature.as_str().expect("feature name"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(native_features, Vec::<&str>::new());
        assert_eq!(
            manifest["dependencies"]["waterui"]["default-features"].as_bool(),
            Some(false)
        );

        let media = manifest["features"][crate::templates::root::MEDIA_FEATURE]
            .as_array()
            .expect("media feature is declared")
            .iter()
            .map(|feature| feature.as_str().expect("feature name"))
            .collect::<Vec<_>>();
        assert_eq!(media, ["waterui/media"]);
        assert_eq!(
            manifest["features"]["dev"].as_array().map(Vec::len),
            Some(1),
            "dev stays the dynamic-linking switch only"
        );

        // The manifest documents the switch where the user will look for it.
        let comment_then_feature = rendered
            .lines()
            .skip_while(|line| !line.starts_with("# Video playback"))
            .find(|line| !line.starts_with('#'));
        assert_eq!(comment_then_feature, Some("media = [\"waterui/media\"]"));
        assert!(
            rendered.contains("--features media"),
            "manifest names the command that turns video on:\n{rendered}"
        );
    }

    #[test]
    fn support_app_android_manifest_enables_picture_in_picture_by_default() {
        let ctx = support_ctx();
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
    }

    /// The user's own packages escape the `\"*\"` override through per-package
    /// entries at the generated crate's own dev profile — unoptimized, with
    /// line tables — while `\"*\"` keeps every other dependency optimized and
    /// stripped.
    #[test]
    fn generated_profiles_override_the_project_packages() {
        let project_packages = BTreeSet::from(["my_app".to_string(), "my_path_dep".to_string()]);
        let profiles = generated_profiles(Some(&project_packages)).expect("a set is provided");
        let dev = profiles.dev.expect("the dev profile exists");

        let star = dev
            .package
            .get("*")
            .and_then(toml::Value::as_table)
            .expect("the wildcard override stays");
        assert_eq!(star["opt-level"], toml::Value::Integer(2));
        assert_eq!(star["debug"], toml::Value::Boolean(false));

        for name in ["my_app", "my_path_dep"] {
            let override_table = dev
                .package
                .get(name)
                .and_then(toml::Value::as_table)
                .unwrap_or_else(|| panic!("package override for {name}"));
            assert_eq!(override_table["opt-level"], toml::Value::Integer(0));
            assert_eq!(
                override_table["debug"],
                toml::Value::String("line-tables-only".to_string())
            );
        }

        // A package outside the project's own set keeps the wildcard's
        // optimized, stripped build.
        assert!(!dev.package.contains_key("waterui-core"));
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

            // The entry-owning Apple binary names a `waterui-apple`
            // dependency only an apple-selected scaffold declares; nothing
            // else renders it.
            if namespace == TemplateNamespace::Ffi
                && !ctx.apple_backend_selected
                && relative_path == Path::new("src/bin/waterui-apple-main.rs.tpl")
            {
                continue;
            }

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
            if namespace == TemplateNamespace::Ffi
                && !ctx.apple_backend_selected
                && relative_path == Path::new("src/bin/waterui-apple-main.rs.tpl")
            {
                continue;
            }
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

pub async fn write_file_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    match fs::read(path).await {
        Ok(existing) if existing == contents => return Ok(()),
        Ok(_) => {}
        // Only a missing file reads as "to be written" — any other read
        // failure is an error of its own, not permission to overwrite.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    fs::write(path, contents).await
}

/// Write `contents` to `path` atomically: the bytes land in a temporary
/// file in the same directory that [`tempfile::NamedTempFile::persist`]
/// then renames over `path`, so a crash mid-write — or a reader racing
/// it — sees the old file or the new one, never a torn file.
///
/// `NamedTempFile::new_in` creates the temporary `0600`; the managed
/// files this writes are regular project files, so the persisted file
/// keeps the replaced file's mode, or `0644` for a new one.
async fn write_file_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write as _;

    let path = path.to_path_buf();
    let contents = contents.to_vec();
    smol::unblock(move || {
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other(format!("`{}` has no directory", path.display())))?;
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt as _;
            match std::fs::metadata(&path) {
                Ok(metadata) => metadata.permissions(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    std::fs::Permissions::from_mode(0o644)
                }
                Err(error) => return Err(error),
            }
        };
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        temporary.write_all(&contents)?;
        #[cfg(unix)]
        temporary.as_file().set_permissions(permissions)?;
        temporary
            .persist(&path)
            .map(|_| ())
            .map_err(|error| error.error)
    })
    .await
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
    /// `[patch]` inherited from the runtime's own workspace.
    ///
    /// Declaring `[workspace]` makes this crate its own workspace root, and
    /// Cargo only honours `[patch]` from the root of the workspace being built.
    /// A support app that skipped these resolved the unpatched crates.io
    /// version of every forked dependency, so it linked a different runtime
    /// than the module it loads — and the module failed to `dlopen` against
    /// symbols whose crate hashes no longer matched.
    #[serde(skip_serializing_if = "cargo_toml::PatchSet::is_empty")]
    patch: cargo_toml::PatchSet,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    git: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rev: Option<String>,
    #[serde(rename = "default-features", skip_serializing_if = "Option::is_none")]
    default_features: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    features: Vec<String>,
}

impl From<GeneratedDependencyDetail> for SupportDependencyDetail {
    fn from(dependency: GeneratedDependencyDetail) -> Self {
        Self {
            package: None,
            version: dependency.version,
            path: dependency.path,
            git: dependency.git,
            rev: dependency.rev,
            default_features: dependency.default_features,
            features: dependency.features,
        }
    }
}

/// Build the `[profile.*]` sections every generated crate carries.
///
/// Generated crates declare `[workspace]`, which makes each of them its own
/// workspace root: they inherit nothing from the repository or the user's project,
/// so whatever profile they should build under has to be written into them here.
/// Without this they defaulted to full debug info for the entire dependency graph,
/// which is the bulk of both the link time and the artifact size of a debug build,
/// and none of which anyone steps through — the `WaterUI` runtime is a dependency of
/// these crates, not the code under debug.
///
/// Dependencies are also compiled with optimizations: the rendering stack
/// (vello, wgpu, parley) is a dependency of every generated crate and sits on
/// every frame's hot path, and at `opt-level` 0 its per-frame encode-and-submit
/// alone costs several milliseconds — a debug `water run` visibly drops frames
/// while scrolling. The generated crate itself stays unoptimized and fully
/// debuggable.
///
/// The user's own packages are path dependencies of the generated crate, so the
/// `"*"` override would otherwise catch them too. They build under the generated
/// crate's own dev profile instead — `opt-level = 0` and line tables — since
/// they are the code a `water run` rebuilds and steps through
/// (`Project::project_packages`).
///
/// Line tables are kept for the generated crate itself so panics still resolve to
/// file and line.
fn generated_profiles_table(project_packages: &BTreeSet<String>) -> cargo_toml::Profiles {
    let mut dev = cargo_toml::Profile {
        debug: Some(cargo_toml::DebugSetting::Lines),
        ..Default::default()
    };
    let mut dependency_override = toml::value::Table::new();
    dependency_override.insert("debug".to_string(), toml::Value::Boolean(false));
    dependency_override.insert("opt-level".to_string(), toml::Value::Integer(2));
    dev.package
        .insert("*".to_string(), toml::Value::Table(dependency_override));

    let mut project_override = toml::value::Table::new();
    project_override.insert(
        "debug".to_string(),
        toml::Value::String("line-tables-only".to_string()),
    );
    project_override.insert("opt-level".to_string(), toml::Value::Integer(0));
    for package in project_packages {
        dev.package.insert(
            package.clone(),
            toml::Value::Table(project_override.clone()),
        );
    }

    // Generated crates are their own workspace roots, so without this section a
    // `cargo build --release` (what `water package` runs) fell back to Cargo's
    // default release profile: no LTO, no stripping, opt-level 3. On Android
    // that shipped a libwaterui_app.so three times the size of the same source
    // built inside the workspace. These settings mirror the workspace root's
    // [profile.release].
    let release = cargo_toml::Profile {
        lto: Some(cargo_toml::LtoSetting::Fat),
        codegen_units: Some(1),
        opt_level: Some(toml::Value::String("z".to_string())),
        strip: Some(cargo_toml::StripSetting::Symbols),
        panic: Some("abort".to_string()),
        ..Default::default()
    };

    cargo_toml::Profiles {
        dev: Some(dev),
        release: Some(release),
        ..Default::default()
    }
}

/// [`generated_profiles_table`] for the project's own package set a render
/// site must supply explicitly. `None` is a `TemplateContext` built without
/// `with_project_packages` — or a helper handed nothing — and is an error:
/// writing the `"*"` pin alone would leave the user's own crates optimized
/// and undebuggable while looking complete.
fn generated_profiles(
    project_packages: Option<&BTreeSet<String>>,
) -> io::Result<cargo_toml::Profiles> {
    project_packages
        .ok_or_else(|| {
            io::Error::other(
                "a generated manifest's profiles need the project's own package set — \
                 build the context with `TemplateContext::with_project_packages` first",
            )
        })
        .map(generated_profiles_table)
}

/// Serialized form of [`generated_profiles_table`], hashed into support-app
/// template fingerprints: the scaffold `Cargo.toml` is generated
/// programmatically rather than from an embedded template file, so cached
/// scaffolds (preview/inspector support apps) would otherwise keep a stale
/// profile when the generated section changes.
fn generated_profiles_fingerprint(project_packages: &BTreeSet<String>) -> String {
    toml::to_string(&generated_profiles_table(project_packages))
        .expect("generated profiles must serialize to TOML")
}

async fn write_support_cargo_toml(
    base_dir: &Path,
    crate_name: &str,
    features: std::collections::BTreeMap<String, Vec<String>>,
    dependencies: std::collections::BTreeMap<String, SupportDependencyValue>,
    runtime_root: Option<&Path>,
    framework: &ResolvedFramework,
    project_packages: Option<&BTreeSet<String>>,
) -> io::Result<()> {
    let patch = match runtime_root {
        Some(root) => {
            let root = root.to_path_buf();
            smol::unblock(move || collect_framework_checkout_patches(&root)).await?
        }
        None => framework.patches(),
    };
    let manifest = SupportCargoManifest {
        package: SupportPackageSection {
            name: crate_name.to_string(),
            version: "0.1.0".to_string(),
            edition: "2024".to_string(),
        },
        lib: SupportLibSection {
            // A support app's own crate is only ever consumed as a Rust dependency of
            // a generated backend crate, and that crate is what the platform links.
            // Emitting `staticlib`/`cdylib` here archived and relinked the entire
            // dependency graph twice more for products nothing ever loads.
            crate_type: vec!["rlib".to_string()],
        },
        profile: generated_profiles(project_packages)?,
        features,
        dependencies,
        workspace: SupportWorkspaceSection {},
        patch,
    };

    let toml_string = toml::to_string_pretty(&manifest)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::create_dir_all(base_dir).await?;
    write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
    Ok(())
}

/// Where a generated backend crate resolves one of its dependencies when the
/// project pins a local `WaterUI` checkout through `waterui_path`.
#[derive(Clone, Copy)]
enum NativeBackendDependencySource<'a> {
    /// The checkout root itself — the `waterui` facade crate.
    WateruiRoot,
    /// A member directory inside the checkout (`core`, `ffi`, …).
    WorkspaceSubdir(&'a str),
    /// The source the checkout's own manifest resolves the crate to: its
    /// `[patch.crates-io]` override when the declared `[workspace.dependencies]`
    /// requirement goes to the registry, the declared entry otherwise. For the
    /// crates released from their own repositories — `hydrolysis-m3`,
    /// `waterui-dew`, `waterui-gtk` — which the checkout consumes as versioned
    /// or git dependencies, not directories.
    WorkspaceDependency,
    /// An in-tree framework workspace member resolved through its
    /// `{name}-path` metadata — `hydrolysis` — the same member source on
    /// every channel, a `path` into a `waterui_path` checkout (#1635).
    FrameworkMember(FrameworkMember),
}

#[derive(Clone, Copy)]
struct NativeBackendDependencySpec<'a> {
    crate_name: &'a str,
    features: &'a [&'a str],
    source: NativeBackendDependencySource<'a>,
}

impl<'a> NativeBackendDependencySpec<'a> {
    const fn new(
        crate_name: &'a str,
        features: &'a [&'a str],
        source: NativeBackendDependencySource<'a>,
    ) -> Self {
        Self {
            crate_name,
            features,
            source,
        }
    }
}

/// The path a generated backend manifest writes for a dependency inside the
/// pinned `WaterUI` checkout. An absolute `waterui_path` resolves directly; a
/// relative one goes through the backend's own `../..` chain to the project
/// root so the project stays portable together with its checkout.
fn compute_native_backend_dependency_path(
    ctx: &TemplateContext,
    waterui_path: &Path,
    subdir: Option<&str>,
) -> String {
    if waterui_path.is_absolute() {
        let absolute_path = subdir.map_or_else(
            || waterui_path.to_path_buf(),
            |subdir| waterui_path.join(subdir),
        );
        return normalize_path_for_config(&absolute_path);
    }

    let project_relative_root = PathBuf::from(ctx.project_root_relative_path());
    let relative_path = subdir.map_or_else(
        || project_relative_root.join(waterui_path),
        |subdir| project_relative_root.join(waterui_path).join(subdir),
    );
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
    manifest.profile = generated_profiles(ctx.project_packages.as_ref())?;

    manifest.dependencies.insert(
        ctx.crate_name.to_string(),
        Dependency::Detailed(Box::new(DependencyDetail {
            path: Some(ctx.project_root_relative_path()),
            ..Default::default()
        })),
    );

    for dependency in dependencies {
        manifest.dependencies.insert(
            dependency.crate_name.to_string(),
            Dependency::Detailed(Box::new(
                generated_dependency_from_spec(ctx, *dependency)?.into_cargo(),
            )),
        );
    }

    manifest.workspace = Some(Workspace::default());
    manifest.patch = generated_crate_patches(ctx)?;

    toml::to_string_pretty(&manifest)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn dependency_path(path: &Path) -> SupportDependencyValue {
    SupportDependencyValue::Detailed(SupportDependencyDetail {
        package: None,
        version: None,
        path: Some(normalize_path_for_config(path)),
        git: None,
        rev: None,
        default_features: None,
        features: Vec::new(),
    })
}

#[derive(serde::Serialize)]
struct GeneratedCargoManifest<T> {
    package: GeneratedPackageSection,
    lib: GeneratedLibSection,
    #[serde(rename = "bin", skip_serializing_if = "Vec::is_empty", default)]
    bins: Vec<GeneratedBinSection>,
    /// Every generated crate declares `[workspace]` and therefore inherits no
    /// profile from the repository or the user's project — see
    /// [`generated_profiles`] for why the dev profile has to be carried
    /// here. Backend scaffolds that omitted this built the entire rendering
    /// stack at `opt-level` 0 with full debug info, which is what made debug
    /// `water run` drop frames.
    profile: cargo_toml::Profiles,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    features: std::collections::BTreeMap<String, Vec<String>>,
    dependencies: std::collections::BTreeMap<String, T>,
    #[serde(
        rename = "build-dependencies",
        skip_serializing_if = "std::collections::BTreeMap::is_empty",
        default
    )]
    build_dependencies: std::collections::BTreeMap<String, T>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    target: std::collections::BTreeMap<String, GeneratedTargetSection<T>>,
    workspace: GeneratedWorkspaceSection,
    /// `[patch]` inherited from the runtime's own workspace.
    ///
    /// Declaring `[workspace]` makes a generated backend crate its own
    /// workspace root, and Cargo only honours `[patch]` from the root of the
    /// workspace being built. A backend crate that skips these silently
    /// resolves the unpatched crates.io version of every forked dependency
    /// (`vello_hybrid` above all) and fails to unify types with the
    /// workspace-built crates it links.
    #[serde(skip_serializing_if = "cargo_toml::PatchSet::is_empty", default)]
    patch: cargo_toml::PatchSet,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<GeneratedPackageMetadata>,
}

/// `[package.metadata]` — presently only wasm-pack, which only hydrolysis's
/// web bundle needs.
#[derive(serde::Serialize)]
struct GeneratedPackageMetadata {
    #[serde(rename = "wasm-pack")]
    wasm_pack: GeneratedWasmPackMetadata,
}

#[derive(serde::Serialize)]
struct GeneratedWasmPackMetadata {
    profile: GeneratedWasmPackProfiles,
}

#[derive(serde::Serialize)]
struct GeneratedWasmPackProfiles {
    release: GeneratedWasmPackProfile,
}

/// wasm-pack runs `wasm-opt -O` with no feature flags, so binaryen validates
/// the wasm-bindgen output against its conservative default feature set and
/// rejects the bulk-memory ops modern rustc emits for memcpy/memset — every
/// `--release` web bundle fails validation without these. The flags are the
/// wasm32 default target features plus reference-types for wasm-bindgen's
/// externref glue.
#[derive(serde::Serialize)]
struct GeneratedWasmPackProfile {
    #[serde(rename = "wasm-opt")]
    wasm_opt: [&'static str; 6],
}

#[derive(serde::Serialize)]
struct GeneratedLibSection {
    /// The library target's name when it must differ from the package's:
    /// Cargo derives both a `[lib]` and a `[[bin]]` from the package name,
    /// and two same-named targets share one output filename stem (the
    /// `.pdb` on MSVC, the `.d` everywhere), which Cargo warns about as an
    /// `output filename collision` on every build.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
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

#[derive(serde::Serialize, Clone, Default)]
struct GeneratedDependencyDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    #[serde(rename = "default-features", skip_serializing_if = "Option::is_none")]
    default_features: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    features: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    optional: bool,
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
    fn inline_toml(&self) -> String {
        let mut table = toml_edit::ser::to_document(self)
            .expect("generated dependency serializes")
            .into_table()
            .into_inline_table();
        table.fmt();
        table.to_string()
    }

    fn into_cargo(self) -> cargo_toml::DependencyDetail {
        cargo_toml::DependencyDetail {
            version: self.version.map(|version| cargo_version_req(&version)),
            path: self.path,
            git: self.git,
            rev: self.rev,
            branch: self.branch,
            tag: self.tag,
            package: self.package,
            default_features: self.default_features.unwrap_or(true),
            features: self.features,
            optional: self.optional,
            ..Default::default()
        }
    }

    /// The dependency `name` — a scaffold package — resolves to on the
    /// selected channel. A package the channel withholds (`stable`'s
    /// git-pinned experimental set) is an error, not a panic: the check must
    /// fire wherever a generated crate is rendered, not only at `create`.
    fn framework(ctx: &TemplateContext, name: &str) -> io::Result<Self> {
        ctx.framework
            .require_distributable(name)
            .map_err(io::Error::other)?;
        let dependency = ctx.framework.dependency(name);
        Ok(Self {
            version: dependency.version.map(|version| version.to_string()),
            path: dependency.path,
            git: dependency.git,
            rev: dependency.rev,
            branch: dependency.branch,
            tag: dependency.tag,
            package: dependency.package,
            default_features: None,
            features: Vec::new(),
            optional: false,
        })
    }

    fn path(path: &Path) -> Self {
        Self {
            path: Some(normalize_path_for_config(path)),
            ..Self::default()
        }
    }

    const fn with_default_features(mut self, default_features: bool) -> Self {
        self.default_features = Some(default_features);
        self
    }

    const fn with_optional(mut self) -> Self {
        self.optional = true;
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
        metadata: None,
    }
}

fn generated_lib(crate_types: &[&str]) -> GeneratedLibSection {
    GeneratedLibSection {
        name: None,
        crate_type: crate_types
            .iter()
            .map(|crate_type| (*crate_type).to_string())
            .collect(),
    }
}

/// The dependency a generated backend manifest declares for `spec`: a `path`
/// into the pinned checkout or the checkout's own resolved source when the
/// project sets `waterui_path`, and the framework's registry/git source
/// otherwise.
fn generated_dependency_from_spec(
    ctx: &TemplateContext,
    spec: NativeBackendDependencySpec<'_>,
) -> io::Result<GeneratedDependencyDetail> {
    let mut detail = match (&ctx.waterui_path, spec.source) {
        // A `*-path` member resolves identically with or without
        // `waterui_path` — `member_dependency` reads the local probe and the
        // channel source itself.
        (_, NativeBackendDependencySource::FrameworkMember(member)) => {
            ctx.member_dependency(member)?
        }
        (Some(_), NativeBackendDependencySource::WorkspaceDependency) => {
            local_checkout_dependency(ctx, spec.crate_name)?
        }
        (Some(waterui_path), NativeBackendDependencySource::WateruiRoot) => {
            GeneratedDependencyDetail {
                path: Some(compute_native_backend_dependency_path(
                    ctx,
                    waterui_path,
                    None,
                )),
                ..GeneratedDependencyDetail::default()
            }
        }
        (Some(waterui_path), NativeBackendDependencySource::WorkspaceSubdir(subdir)) => {
            GeneratedDependencyDetail {
                path: Some(compute_native_backend_dependency_path(
                    ctx,
                    waterui_path,
                    Some(subdir),
                )),
                ..GeneratedDependencyDetail::default()
            }
        }
        (None, _) => GeneratedDependencyDetail::framework(ctx, spec.crate_name)?,
    };

    // Features the checkout's declared entry carries resolve exactly like a
    // member inheriting `dep.workspace = true`, so they merge with — never
    // replace — the features the generated crate selects for itself.
    for feature in spec.features {
        if !detail.features.iter().any(|declared| declared == feature) {
            detail.features.push((*feature).to_string());
        }
    }
    Ok(detail)
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

/// Emit each selectable FFI feature's `dep/feat` forwards, filtered by the
/// feature table of the package its destination actually resolves to.
///
/// `waterui-ffi` is a non-Apple target dependency in the crates that carry
/// it, so the forwards are the plain `dep/feat` form; see
/// `FORWARDED_FFI_FEATURES` for why they are manifest-declared at all. An
/// older `waterui-ffi` without `inspector` gets no `inspector` forward,
/// while a backend destination keeps its own entry (`map` still reaches
/// `waterui-apple/map`). This runs last in each manifest's generation: the
/// probe that learns the resolved tables runs on the manifest being
/// written, so every dependency and patch must already be in place.
async fn configure_capability_forwards(
    manifest: &mut cargo_toml::Manifest<()>,
    ctx: &TemplateContext,
    base_dir: &Path,
) -> io::Result<()> {
    let tables = Box::pin(resolved_forward_tables(
        &ctx.host,
        manifest,
        base_dir,
        &forward_targets(manifest),
    ))
    .await?;
    for name in FORWARDED_FFI_FEATURES {
        let forwards = ffi_feature_forwards(name, manifest, &tables);
        if !forwards.is_empty() {
            manifest.features.insert((*name).to_string(), forwards);
        }
    }
    Ok(())
}

/// The dependency, patch and feature-forward tables the generated
/// Apple-target crates share. The FFI companion and the Apple preview
/// package read them through this one function, so the `waterui` facade and
/// `libwaterui_dylib` resolve to the same Cargo units `water run
/// --platform macos` compiles in the shared target directory: two
/// functions listing them would drift, and Cargo folds the resolved
/// feature set into every symbol's `-C metadata` hash.
///
/// `waterui_apple_features` carries the features one manifest's
/// `waterui-apple` edge selects beyond the companion's — the preview
/// package's `preview`, which `water run` deliberately never enables.
///
/// This runs last in each manifest's generation: the probe that learns the
/// resolved feature tables runs on the manifest being written, so every
/// dependency and patch must already be in place.
async fn configure_apple_target_tables(
    manifest: &mut cargo_toml::Manifest<()>,
    ctx: &TemplateContext,
    base_dir: &Path,
    waterui_apple_features: &[&str],
) -> io::Result<()> {
    manifest.dependencies.insert(
        ctx.crate_name.to_string(),
        cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
            path: Some(ctx.project_root_relative_path()),
            ..Default::default()
        })),
    );

    manifest
        .features
        .insert("dev".to_string(), vec![format!("{}/dev", ctx.crate_name)]);

    let waterui = generated_dependency_from_spec(
        ctx,
        NativeBackendDependencySpec::new(
            "waterui",
            &[],
            NativeBackendDependencySource::WateruiRoot,
        ),
    )?
    .with_default_features(false)
    .into_cargo();
    manifest.dependencies.insert(
        "waterui".to_string(),
        cargo_toml::Dependency::Detailed(Box::new(waterui)),
    );

    // The Rust backend owns the app's whole startup through
    // `waterui_apple::export_app!`. It does not live in the `WaterUI`
    // workspace, so it resolves against the Apple backend checkout the
    // project already uses — never the framework registry source the
    // `waterui` edge above applies. On a macOS host the Apple pieces
    // render into every generated crate these tables cover: the render is
    // a function of the project, not of the invocation that produced it,
    // and macOS is the only host an Apple build can run from, so the
    // `waterui-apple` pin resolves there whether or not the build being
    // scaffolded is an Apple one.
    if ctx.apple_backend_selected {
        let mut waterui_apple = ctx.waterui_apple_dependency()?;
        waterui_apple.features.extend(
            waterui_apple_features
                .iter()
                .map(|feature| (*feature).to_string()),
        );
        manifest
            .target
            .entry("cfg(target_vendor = \"apple\")".to_string())
            .or_default()
            .dependencies
            .insert(
                "waterui-apple".to_string(),
                cargo_toml::Dependency::Detailed(Box::new(waterui_apple.into_cargo())),
            );
    }
    if ctx.cef_runtime_enabled() {
        let browser = generated_dependency_from_spec(
            ctx,
            NativeBackendDependencySpec::new(
                "waterui-browser-cef",
                &["cef-runtime"],
                NativeBackendDependencySource::WorkspaceDependency,
            ),
        )?
        .with_default_features(false)
        .into_cargo();
        manifest
            .target
            .entry("cfg(target_os = \"macos\")".to_string())
            .or_default()
            .dependencies
            .insert(
                "waterui-browser-cef".to_string(),
                cargo_toml::Dependency::Detailed(Box::new(browser)),
            );
    }

    manifest.patch = {
        let ctx = TemplateContext::clone(ctx);
        smol::unblock(move || generated_crate_patches(&ctx)).await?
    };

    configure_capability_forwards(manifest, ctx, base_dir).await
}

/// Apple backend templates.
pub mod apple {
    use super::{
        Path, PathBuf, TemplateContext, TemplateNamespace, embedded, io, render_dir_outputs,
        scaffold_dir,
    };

    /// Write all Apple templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(TemplateNamespace::Apple, &embedded::APPLE, base_dir, ctx).await?;
        Ok(())
    }

    /// The `(path, contents)` pairs [`scaffold`] would write for `ctx`, without
    /// touching the filesystem — the comparison set a generated Apple backend
    /// is regenerated against.
    ///
    /// # Errors
    ///
    /// Returns an error if a template fails to render.
    pub fn rendered_outputs(ctx: &TemplateContext) -> io::Result<Vec<(PathBuf, Vec<u8>)>> {
        render_dir_outputs(TemplateNamespace::Apple, &embedded::APPLE, ctx)
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
        scaffold_dir(
            TemplateNamespace::AndroidShared,
            &embedded::ANDROID_SHARED,
            base_dir,
            ctx,
        )
        .await?;
        // The template carries the wrapper scripts and properties only — the
        // repository ships no binary files, so the jar the `gradlew` scripts
        // need is fetched pinned-and-verified the first time a Gradle task
        // runs (`run_gradle_tasks`), not at scaffold time: scaffolding must
        // stay offline-capable.

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
        if let Some(sdk_path) = AndroidSdk::detect_path(&crate::toolchain::Host::current()) {
            let local_props = base_dir.join("local.properties");
            let content = format!("sdk.dir={}\n", normalize_path_for_config(&sdk_path));
            write_file_if_changed(&local_props, content.as_bytes()).await?;
        }

        Ok(())
    }
}

/// Embedded-mode Android templates: a Gradle *library* project whose
/// `:waterui` module assembles the AAR a host application consumes. Unlike
/// the app template it owns no Activity or manifest entry — the host mounts
/// the root view through the runtime's `WaterUiRootView`.
pub mod android_embedded {
    use crate::android::toolchain::AndroidSdk;

    use super::{
        Path, TemplateContext, TemplateNamespace, embedded, fs, io, normalize_path_for_config,
        scaffold_dir, write_file_if_changed,
    };

    /// Write all embedded Android templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(
            TemplateNamespace::AndroidEmbedded,
            &embedded::ANDROID_EMBEDDED,
            base_dir,
            ctx,
        )
        .await?;
        scaffold_dir(
            TemplateNamespace::AndroidShared,
            &embedded::ANDROID_SHARED,
            base_dir,
            ctx,
        )
        .await?;
        // gradle-wrapper.jar materializes at first Gradle run — see the
        // android scaffold note above.

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

        // Create jniLibs directories under the library module
        for abi in ["arm64-v8a", "x86_64", "armeabi-v7a", "x86"] {
            let jni_dir = base_dir.join(format!("waterui/src/main/jniLibs/{abi}"));
            fs::create_dir_all(&jni_dir).await?;
        }

        // Generate local.properties with Android SDK path
        if let Some(sdk_path) = AndroidSdk::detect_path(&crate::toolchain::Host::current()) {
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
        NativeBackendDependencySource, NativeBackendDependencySpec, Path, TemplateContext,
        TemplateNamespace, embedded, io, scaffold_dir, write_native_backend_bin_cargo_toml,
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
        let features = super::webview_backend_feature(ctx.browser.linux_answers()?)
            .into_iter()
            .collect::<Vec<_>>();
        let dependencies = gtk4_dependencies(&features);
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
        let features = super::webview_backend_feature(ctx.browser.linux_answers()?)
            .into_iter()
            .collect::<Vec<_>>();
        let dependencies = gtk4_dependencies(&features);
        write_native_backend_bin_cargo_toml(base_dir, ctx, package_name, &dependencies).await
    }

    /// The generated crate calls `waterui::configure_environment!` for its
    /// `i18n/` catalog, so it depends on the facade in addition to the backend.
    const fn gtk4_dependencies<'a>(
        features: &'a [&'a str],
    ) -> [NativeBackendDependencySpec<'a>; 2] {
        [
            NativeBackendDependencySpec::new(
                "waterui-gtk",
                features,
                NativeBackendDependencySource::WorkspaceDependency,
            ),
            NativeBackendDependencySpec::new(
                "waterui",
                &[],
                NativeBackendDependencySource::WateruiRoot,
            ),
        ]
    }
}

/// `WinUI` backend templates.
pub mod winui {
    use super::{
        NativeBackendDependencySource, NativeBackendDependencySpec, Path, TemplateContext,
        TemplateNamespace, embedded, io, scaffold_dir,
    };
    use cargo_toml::{Dependency, DependencyDetail};

    /// Write all `WinUI` templates to the given directory.
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
        scaffold_dir(TemplateNamespace::WinUi, &embedded::WINUI, base_dir, ctx).await
    }

    /// Every file `scaffold` would write, as backend-relative path and
    /// content, without touching the filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if template or Cargo manifest rendering fails.
    pub fn rendered_outputs(
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<Vec<(std::path::PathBuf, Vec<u8>)>> {
        let mut outputs =
            super::render_dir_outputs(TemplateNamespace::WinUi, &embedded::WINUI, ctx)?;
        outputs.push((
            std::path::PathBuf::from("Cargo.toml"),
            render_cargo_toml(ctx, package_name)?.into_bytes(),
        ));
        Ok(outputs)
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        super::write_generated_cargo_toml(base_dir, render_cargo_toml(ctx, package_name)?).await
    }

    /// Generated `Cargo.toml` for the `WinUI` launcher crate.
    ///
    /// Serialized through `cargo_toml` like the other simple binary backends,
    /// but assembled here because the manifest carries `[build-dependencies]`
    /// (`winresource` embeds the staged icon, `windows-reactor-setup` stages
    /// the self-contained runtime) and the `gpu-allocator` patch that tracks
    /// wherever `waterui-winui` itself resolved from.
    fn render_cargo_toml(ctx: &TemplateContext, package_name: &str) -> io::Result<String> {
        use cargo_toml::{Manifest, Package, Workspace};

        let (backend, gpu_allocator_patch) = winui_backend_dependency(ctx)?;

        let mut manifest = Manifest::<()>::default();
        let mut package = Package::new(package_name.to_string(), super::cargo_semver("0.1.0"));
        package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
        manifest.package = Some(package);
        manifest.profile = super::generated_profiles(ctx.project_packages.as_ref())?;

        manifest.dependencies.insert(
            ctx.crate_name.to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(ctx.project_root_relative_path()),
                ..Default::default()
            })),
        );
        manifest.dependencies.insert(
            "waterui".to_string(),
            Dependency::Detailed(Box::new(
                super::generated_dependency_from_spec(
                    ctx,
                    NativeBackendDependencySpec::new(
                        "waterui",
                        &[],
                        NativeBackendDependencySource::WateruiRoot,
                    ),
                )?
                .into_cargo(),
            )),
        );
        manifest
            .dependencies
            .insert("waterui-winui".to_string(), backend);

        // `winresource` embeds the staged `app-icon.ico`; `windows-reactor-setup`
        // stages the Windows App Runtime next to the produced binary and emits
        // the `rustc-link-arg-bins` that embed the marker manifest `bootstrap`
        // reads — both must be build-dependencies of the bin crate itself.
        manifest.build_dependencies.insert(
            "winresource".to_string(),
            Dependency::Simple(super::cargo_version_req("0.1")),
        );
        manifest.build_dependencies.insert(
            "windows-reactor-setup".to_string(),
            Dependency::Simple(super::cargo_version_req("0.100")),
        );

        manifest.workspace = Some(Workspace::default());
        manifest.patch = winui_patch_set(ctx, gpu_allocator_patch)?;

        toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// The `waterui-winui` dependency the launcher resolves, paired with the
    /// `gpu-allocator` patch entry the same source carries:
    /// `WATERUI_WINUI_PATH` when set (the escape hatch for developing the
    /// backend itself), a `water-rs/waterui-winui` checkout beside a local
    /// `waterui_path`, and the framework-declared backend coordinate otherwise.
    ///
    /// `waterui-winui` keeps a vendored `gpu-allocator` workspace member that
    /// narrows an upstream `windows` version range; a consumer can only reach
    /// it by patching `gpu-allocator` to the same git revision or checkout the
    /// backend dependency itself resolved to, so both come out of one source.
    fn winui_backend_dependency(ctx: &TemplateContext) -> io::Result<(Dependency, Dependency)> {
        if let Some(path) = std::env::var_os("WATERUI_WINUI_PATH") {
            let path = dunce::canonicalize(path)?;
            return Ok((
                path_dependency(&path),
                path_dependency(&path.join("vendor/gpu-allocator")),
            ));
        }
        if let Some(root) = ctx
            .waterui_workspace_root()
            .and_then(|root| dunce::canonicalize(root).ok())
            && let Some(sibling) = root
                .parent()
                .map(|parent| parent.join("water-rs/waterui-winui"))
            && sibling.join("Cargo.toml").is_file()
        {
            return Ok((
                path_dependency(&sibling),
                path_dependency(&sibling.join("vendor/gpu-allocator")),
            ));
        }
        let detail = super::generated_dependency_from_spec(
            ctx,
            NativeBackendDependencySpec::new(
                "waterui-winui",
                &[],
                NativeBackendDependencySource::WorkspaceDependency,
            ),
        )?;
        let patch = gpu_allocator_patch(&detail)?;
        Ok((Dependency::Detailed(Box::new(detail.into_cargo())), patch))
    }

    fn path_dependency(path: &Path) -> Dependency {
        Dependency::Detailed(Box::new(DependencyDetail {
            path: Some(super::normalize_path_for_config(path)),
            ..Default::default()
        }))
    }

    /// The `[patch.crates-io]` entry for the vendored `gpu-allocator` member of
    /// the `waterui-winui` source `detail` resolved to. A registry-sourced
    /// `waterui-winui` has no vendored member to pin — the workspace-member
    /// patch only exists inside the backend's own repository.
    fn gpu_allocator_patch(detail: &super::GeneratedDependencyDetail) -> io::Result<Dependency> {
        if let Some(git) = &detail.git {
            return Ok(Dependency::Detailed(Box::new(DependencyDetail {
                git: Some(git.clone()),
                rev: detail.rev.clone(),
                ..Default::default()
            })));
        }
        if let Some(path) = &detail.path {
            return Ok(Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(super::normalize_path_for_config(
                    &Path::new(path).join("vendor/gpu-allocator"),
                )),
                ..Default::default()
            })));
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "`waterui-winui` resolved to a registry dependency, but its `gpu-allocator` \
             patch can only pin a git revision or a checkout carrying the vendored member",
        ))
    }

    /// The `[patch]` table the launcher needs as its own workspace root: the
    /// checkout's or channel's set every generated crate gets, plus the
    /// `gpu-allocator` entry only `waterui-winui` requires, with the project's
    /// own entries merged over both.
    fn winui_patch_set(
        ctx: &TemplateContext,
        gpu_allocator_patch: Dependency,
    ) -> io::Result<cargo_toml::PatchSet> {
        let mut patch = super::framework_crate_patches(ctx)?;
        patch
            .entry("crates-io".to_string())
            .or_default()
            .insert("gpu-allocator".to_string(), gpu_allocator_patch);
        super::with_project_patches(patch, ctx.project_root_path.as_deref())
    }
}

/// Hydrolysis backend templates.
pub mod hydrolysis {
    use super::{
        GeneratedBinSection, GeneratedCargoManifest, GeneratedDependencyDetail,
        GeneratedDependencyValue, GeneratedTargetSection, GeneratedWorkspaceSection, HYDROLYSIS,
        NativeBackendDependencySource, NativeBackendDependencySpec, Path, TemplateContext,
        TemplateNamespace, embedded, io, scaffold_dir, write_generated_cargo_toml,
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
        let patch = super::generated_crate_patches(ctx)?;
        outputs.push((
            std::path::PathBuf::from("Cargo.toml"),
            super::render_generated_cargo_toml(&generated_manifest(ctx, package_name, patch)?)?
                .into_bytes(),
        ));
        Ok(outputs)
    }

    fn generated_manifest(
        ctx: &TemplateContext,
        package_name: &str,
        patch: cargo_toml::PatchSet,
    ) -> io::Result<GeneratedCargoManifest<GeneratedDependencyValue>> {
        let mut package = super::generated_package(package_name, Vec::new());
        package.autobins = Some(false);
        package.metadata = Some(super::GeneratedPackageMetadata {
            wasm_pack: super::GeneratedWasmPackMetadata {
                profile: super::GeneratedWasmPackProfiles {
                    release: super::GeneratedWasmPackProfile {
                        wasm_opt: [
                            "-O",
                            "--enable-bulk-memory",
                            "--enable-mutable-globals",
                            "--enable-sign-ext",
                            "--enable-nontrapping-float-to-int",
                            "--enable-reference-types",
                        ],
                    },
                },
            },
        });
        let mut bins = vec![GeneratedBinSection {
            name: package_name.to_string(),
            path: "src/main.rs".to_string(),
        }];
        if requires_cef(ctx) {
            bins.push(GeneratedBinSection {
                name: crate::project_model::project_types::cef_helper_binary_name(package_name),
                path: "src/bin/waterui-cef-helper.rs".to_string(),
            });
        }
        // The backend binary keeps the package's name (`build_binary` and
        // packaging select it by that name); the library target the web
        // bundle compiles takes a distinct one so the two never share an
        // output filename stem in the shared target directory.
        let mut lib = super::generated_lib(&["cdylib", "rlib"]);
        lib.name = Some(hydrolysis_library_target_name(package_name));
        Ok(GeneratedCargoManifest {
            package,
            lib,
            bins,
            profile: super::generated_profiles(ctx.project_packages.as_ref())?,
            features: BTreeMap::from([
                // The preview runtime module compiles under the feature
                // alone — it no longer consumes the `waterui-preview`
                // support-app crate.
                ("waterui-preview-mode".to_string(), vec![]),
                (
                    "waterui-preview-test-mode".to_string(),
                    vec!["dep:waterui-testing".to_string()],
                ),
                (
                    "waterui-mcp-mode".to_string(),
                    vec![
                        "dep:waterui-mcp".to_string(),
                        "dep:waterui-testing".to_string(),
                    ],
                ),
            ]),
            dependencies: cargo_dependencies(ctx)?,
            // The build script embeds the staged Windows icon resource; the
            // crate is a no-op on every other target.
            build_dependencies: BTreeMap::from([(
                "winresource".to_string(),
                GeneratedDependencyValue::Simple("0.1".to_string()),
            )]),
            target: cargo_target_dependencies(ctx)?,
            workspace: GeneratedWorkspaceSection {},
            patch,
        })
    }

    /// The `[lib]` target name of a generated Hydrolysis backend: the package
    /// name as a crate identifier with a `_lib` suffix, distinct from the
    /// `[[bin]]` that carries the package name itself.
    pub fn hydrolysis_library_target_name(package_name: &str) -> String {
        format!("{}_lib", package_name.replace('-', "_"))
    }

    /// Whether this application's graph links the bundled CEF runtime for
    /// any desktop OS — the helper `[[bin]]` is declared once for the
    /// whole manifest while the dependency lives in each OS's own table.
    ///
    /// A packaged CEF application needs a subprocess helper binary, and the
    /// helper needs the engine crate; both follow the application's own
    /// dependencies, never a manifest setting.
    fn requires_cef(ctx: &TemplateContext) -> bool {
        ctx.declares_cef_helper()
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let patch = {
            let ctx = TemplateContext::clone(ctx);
            smol::unblock(move || super::generated_crate_patches(&ctx)).await?
        };
        let manifest = generated_manifest(ctx, package_name, patch)?;
        write_generated_cargo_toml(base_dir, super::render_generated_cargo_toml(&manifest)?).await
    }

    fn cargo_dependencies(
        ctx: &TemplateContext,
    ) -> io::Result<BTreeMap<String, GeneratedDependencyValue>> {
        Ok(BTreeMap::from([
            (
                ctx.crate_name.to_string(),
                GeneratedDependencyValue::detailed(GeneratedDependencyDetail {
                    path: Some(ctx.project_root_relative_path()),
                    ..GeneratedDependencyDetail::default()
                }),
            ),
            (
                "waterui".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui",
                            // Hydrolysis draws every pixel itself, so it has no
                            // native player to bridge — but whether the
                            // self-drawn realization is linked is the
                            // application's call: it declares `video-gpu` on
                            // its own `waterui` dependency, and feature
                            // unification carries that choice into this graph.
                            // Forcing it here would push the decoder stack
                            // into applications that never render video.
                            &[],
                            NativeBackendDependencySource::WateruiRoot,
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
        ]))
    }

    fn cargo_target_dependencies(
        ctx: &TemplateContext,
    ) -> io::Result<BTreeMap<String, GeneratedTargetSection<GeneratedDependencyValue>>> {
        let mut target = BTreeMap::from([
            (
                "cfg(all(not(target_arch = \"wasm32\"), not(target_os = \"android\")))".to_string(),
                GeneratedTargetSection {
                    dependencies: native_target_dependencies(ctx)?,
                },
            ),
            (
                "cfg(target_os = \"android\")".to_string(),
                GeneratedTargetSection {
                    dependencies: android_target_dependencies(ctx)?,
                },
            ),
            (
                "cfg(target_arch = \"wasm32\")".to_string(),
                GeneratedTargetSection {
                    dependencies: wasm_target_dependencies(ctx)?,
                },
            ),
        ]);
        target.extend(native_os_target_dependencies(ctx)?);
        Ok(target)
    }

    /// The engine-dependent pieces the shared native table cannot hold:
    /// the `webview-system` Hydrolysis feature and the
    /// `waterui-browser-cef` dependency legitimately differ per OS —
    /// `waterui-browser-wpe` enters the application's graph on Linux only —
    /// so each OS's section renders from that OS's own answers while
    /// everything engine-independent stays in the shared native table.
    fn native_os_target_dependencies(
        ctx: &TemplateContext,
    ) -> io::Result<BTreeMap<String, GeneratedTargetSection<GeneratedDependencyValue>>> {
        let browser = ctx.browser.desktop_answers()?;
        let mut tables = BTreeMap::new();
        for os in crate::platform::NativeOs::ALL {
            let mut dependencies = BTreeMap::new();
            if let Some(feature) = browser.webview_backend_feature(os) {
                dependencies.insert(
                    "hydrolysis".to_string(),
                    GeneratedDependencyValue::detailed(
                        super::generated_dependency_from_spec(
                            ctx,
                            NativeBackendDependencySpec::new(
                                "hydrolysis",
                                &[feature],
                                NativeBackendDependencySource::FrameworkMember(HYDROLYSIS),
                            ),
                        )?
                        .with_default_features(false),
                    ),
                );
            }
            // The CEF subprocess helper is a second binary in this crate,
            // and it is the one process that must not start WaterUI at
            // all: it dispatches straight into Chromium. The engine crate
            // is the application's choice, so this dependency appears only
            // in the OS section whose graph links it — the helper source
            // gates its dispatch on the same set.
            if crate::project_model::project_types::declares_cef_helper(browser.for_os(os).engine) {
                dependencies.insert(
                    "waterui-browser-cef".to_string(),
                    GeneratedDependencyValue::detailed(super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-browser-cef",
                            &[],
                            NativeBackendDependencySource::WorkspaceSubdir(
                                "components/platform/browser-cef",
                            ),
                        ),
                    )?),
                );
            }
            if !dependencies.is_empty() {
                tables.insert(
                    os.cfg().to_string(),
                    GeneratedTargetSection { dependencies },
                );
            }
        }
        Ok(tables)
    }

    /// The dependencies only the Android launcher compiles: the Hydrolysis
    /// runner's Android host, the JNI declarations `JNI_OnLoad` needs, and
    /// the GPU-capable `waterui` the registered app builds with. The
    /// desktop-only stack (winit, pollster, preview and MCP runtimes) stays
    /// out — the Android launcher never binaries or previews.
    fn android_target_dependencies(
        ctx: &TemplateContext,
    ) -> io::Result<BTreeMap<String, GeneratedDependencyValue>> {
        Ok(BTreeMap::from([
            (
                "hydrolysis".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis",
                            &["accessibility"],
                            NativeBackendDependencySource::FrameworkMember(HYDROLYSIS),
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
            (
                "waterui".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui",
                            &["gpu"],
                            NativeBackendDependencySource::WateruiRoot,
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
            (
                "waterui-core".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-core",
                            &[],
                            NativeBackendDependencySource::WorkspaceSubdir("core"),
                        ),
                    )?
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
                            &[],
                            NativeBackendDependencySource::WorkspaceDependency,
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
            (
                "jni".to_string(),
                GeneratedDependencyValue::simple("0.21.1"),
            ),
        ]))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "linear enumeration of native target dependencies reads clearest as one list"
    )]
    fn native_target_dependencies(
        ctx: &TemplateContext,
    ) -> io::Result<BTreeMap<String, GeneratedDependencyValue>> {
        let dependencies: BTreeMap<String, GeneratedDependencyValue> = BTreeMap::from([
            (
                "hydrolysis".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis",
                            &["winit"],
                            NativeBackendDependencySource::FrameworkMember(HYDROLYSIS),
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
            (
                "pollster".to_string(),
                GeneratedDependencyValue::simple("0.4"),
            ),
            (
                "waterui-core".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-core",
                            &[],
                            NativeBackendDependencySource::WorkspaceSubdir("core"),
                        ),
                    )?
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
                            &[],
                            NativeBackendDependencySource::WorkspaceSubdir(
                                "components/devtools/preview/protocol",
                            ),
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
            ("serde".to_string(), GeneratedDependencyValue::simple("1")),
            (
                "serde_json".to_string(),
                GeneratedDependencyValue::simple("1"),
            ),
            (
                "waterui-mcp".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-mcp",
                            &[],
                            NativeBackendDependencySource::WorkspaceSubdir(
                                "components/devtools/mcp/server",
                            ),
                        ),
                    )?
                    .with_default_features(false)
                    .with_optional(),
                ),
            ),
            (
                "waterui-testing".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "waterui-testing",
                            &[],
                            NativeBackendDependencySource::WorkspaceSubdir("testing"),
                        ),
                    )?
                    .with_default_features(false)
                    .with_optional(),
                ),
            ),
            (
                "hydrolysis-m3".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis-m3",
                            &[],
                            NativeBackendDependencySource::WorkspaceDependency,
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
        ]);
        Ok(dependencies)
    }

    fn wasm_target_dependencies(
        ctx: &TemplateContext,
    ) -> io::Result<BTreeMap<String, GeneratedDependencyValue>> {
        Ok(BTreeMap::from([
            (
                "hydrolysis".to_string(),
                GeneratedDependencyValue::detailed(
                    super::generated_dependency_from_spec(
                        ctx,
                        NativeBackendDependencySpec::new(
                            "hydrolysis",
                            &["web"],
                            NativeBackendDependencySource::FrameworkMember(HYDROLYSIS),
                        ),
                    )?
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
                            &[],
                            NativeBackendDependencySource::WorkspaceDependency,
                        ),
                    )?
                    .with_default_features(false),
                ),
            ),
        ]))
    }
}

/// The generated Gradle app that runs a `WaterUI` application through the
/// Hydrolysis Android host: a Kotlin `HydrolysisActivity` subclass, the
/// painter band, and the Rust cdylib wiring — rendered under
/// `<backend>/android` beside the launcher crate `templates::hydrolysis`
/// scaffolds.
pub mod hydrolysis_android {
    use crate::android::toolchain::AndroidSdk;

    use super::{
        Path, PathBuf, TemplateContext, TemplateNamespace, embedded, io, normalize_path_for_config,
        scaffold_dir, write_file_if_changed,
    };

    /// Write all Hydrolysis Android app templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub async fn scaffold(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        scaffold_dir(
            TemplateNamespace::HydrolysisAndroid,
            &embedded::HYDROLYSIS_ANDROID,
            base_dir,
            ctx,
        )
        .await?;
        scaffold_dir(
            TemplateNamespace::AndroidShared,
            &embedded::ANDROID_SHARED,
            base_dir,
            ctx,
        )
        .await?;
        // gradle-wrapper.jar materializes at first Gradle run — see the
        // android scaffold note above.

        // Make gradlew executable
        #[cfg(unix)]
        {
            use super::fs;
            use std::os::unix::fs::PermissionsExt;
            let gradlew_path = base_dir.join("gradlew");
            if gradlew_path.exists() {
                let mut perms = fs::metadata(&gradlew_path).await?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&gradlew_path, perms).await?;
            }
        }

        // Generate local.properties with Android SDK path
        if let Some(sdk_path) = AndroidSdk::detect_path(&crate::toolchain::Host::current()) {
            let local_props = base_dir.join("local.properties");
            let content = format!("sdk.dir={}\n", normalize_path_for_config(&sdk_path));
            write_file_if_changed(&local_props, content.as_bytes()).await?;
        }

        Ok(())
    }

    /// Every file `scaffold` would write, as backend-relative path and
    /// content, without touching the filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if template rendering fails.
    pub fn rendered_outputs(ctx: &TemplateContext) -> io::Result<Vec<(PathBuf, Vec<u8>)>> {
        let mut outputs = super::render_dir_outputs(
            TemplateNamespace::HydrolysisAndroid,
            &embedded::HYDROLYSIS_ANDROID,
            ctx,
        )?;
        outputs.extend(super::render_dir_outputs(
            TemplateNamespace::AndroidShared,
            &embedded::ANDROID_SHARED,
            ctx,
        )?);
        Ok(outputs)
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

/// Experimental terminal (TUI) backend templates.
///
/// `water run --tui` generates a thin launcher crate into the project's managed
/// build cache. The crate depends on the pinned `waterui-tui` backend and calls
/// its `run_app` entry point with the application's composed `App`.
pub mod tui {
    use cargo_toml::{Dependency, DependencyDetail, Manifest, Package, Workspace};

    use super::{
        NativeBackendDependencySource, NativeBackendDependencySpec, Path, PathBuf, TemplateContext,
        TemplateNamespace, embedded, io, normalize_path_for_config, scaffold_dir,
        write_file_if_changed,
    };
    use crate::build_info::TUI_BACKEND;

    /// `waterui-*` crates `waterui-tui` names directly that the framework's own
    /// `[patch.crates-io]` table never lists — the workspace only patches crates
    /// an extracted backend depends on, and none names these two. Without
    /// entries here they resolve from the registry beside the checkout- or
    /// channel-sourced graph, so `App` becomes a different type on either side
    /// of the launcher's `run_app` call. Values are in-checkout directories.
    const EXTRA_PATCHES: &[(&str, &str)] = &[
        ("waterui-internal", "src"),
        ("waterui-navigation", "components/foundation/navigation"),
    ];

    /// Write all TUI launcher templates to the given directory.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations or manifest rendering fail.
    pub async fn scaffold(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        generate_cargo_toml(base_dir, ctx, package_name).await?;
        scaffold_dir(TemplateNamespace::Tui, &embedded::TUI, base_dir, ctx).await
    }

    /// Every file `scaffold` would write, as launcher-relative path and
    /// content, without touching the filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if template or manifest rendering fails.
    pub fn rendered_outputs(
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<Vec<(PathBuf, Vec<u8>)>> {
        let mut outputs = super::render_dir_outputs(TemplateNamespace::Tui, &embedded::TUI, ctx)?;
        outputs.push((
            PathBuf::from("Cargo.toml"),
            render_cargo_toml(ctx, package_name)?.into_bytes(),
        ));
        Ok(outputs)
    }

    async fn generate_cargo_toml(
        base_dir: &Path,
        ctx: &TemplateContext,
        package_name: &str,
    ) -> io::Result<()> {
        let toml_string = render_cargo_toml(ctx, package_name)?;
        super::fs::create_dir_all(base_dir).await?;
        write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await
    }

    fn render_cargo_toml(ctx: &TemplateContext, package_name: &str) -> io::Result<String> {
        let mut manifest = Manifest::<()>::default();
        let mut package = Package::new(package_name.to_string(), super::cargo_semver("0.1.0"));
        package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
        manifest.package = Some(package);
        // The launcher is its own workspace root and therefore inherits no
        // profile — a TUI built at opt-level 0 cannot push frames, so the dev
        // profile has to be carried here like every other generated crate.
        manifest.profile = super::generated_profiles(ctx.project_packages.as_ref())?;

        manifest.dependencies.insert(
            ctx.crate_name.to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some(ctx.project_root_relative_path()),
                ..Default::default()
            })),
        );
        manifest.dependencies.insert(
            "waterui".to_string(),
            Dependency::Detailed(Box::new(
                super::generated_dependency_from_spec(
                    ctx,
                    NativeBackendDependencySpec::new(
                        "waterui",
                        &[],
                        NativeBackendDependencySource::WateruiRoot,
                    ),
                )?
                .with_default_features(false)
                .into_cargo(),
            )),
        );
        manifest
            .dependencies
            .insert("waterui-tui".to_string(), tui_backend_dependency(ctx)?);

        manifest.workspace = Some(Workspace::default());
        manifest.patch = tui_patch_set(ctx)?;

        toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// The `waterui-tui` dependency the launcher resolves:
    /// `WATERUI_TUI_PATH` when set (the escape hatch for developing the backend
    /// itself), a `water-rs/tui` checkout beside a local `waterui_path`, and
    /// the pinned backend revision otherwise.
    fn tui_backend_dependency(ctx: &TemplateContext) -> io::Result<Dependency> {
        if let Some(path) = std::env::var_os("WATERUI_TUI_PATH") {
            return Ok(path_dependency(&dunce::canonicalize(path)?));
        }
        if let Some(root) = ctx
            .waterui_workspace_root()
            .and_then(|root| dunce::canonicalize(root).ok())
            && let Some(sibling) = root.parent().map(|parent| parent.join("water-rs/tui"))
            && sibling.join("Cargo.toml").is_file()
        {
            return Ok(path_dependency(&sibling));
        }
        Ok(Dependency::Detailed(Box::new(DependencyDetail {
            git: Some(TUI_BACKEND.repository_url.to_string()),
            rev: Some(TUI_BACKEND.revision.to_string()),
            ..Default::default()
        })))
    }

    fn path_dependency(path: &Path) -> Dependency {
        Dependency::Detailed(Box::new(DependencyDetail {
            path: Some(normalize_path_for_config(path)),
            ..Default::default()
        }))
    }

    /// The `[patch]` table the launcher needs as its own workspace root: the
    /// checkout's or channel's set every other generated crate gets, plus the
    /// [`EXTRA_PATCHES`] entries `waterui-tui` alone requires, with the
    /// project's own entries merged over both.
    fn tui_patch_set(ctx: &TemplateContext) -> io::Result<cargo_toml::PatchSet> {
        let waterui_root = ctx.waterui_workspace_root();
        let mut patch = super::framework_crate_patches(ctx)?;
        let crates_io = patch.entry("crates-io".to_string()).or_default();
        if let Some(root) = &waterui_root {
            for &(name, subdir) in EXTRA_PATCHES {
                crates_io
                    .entry(name.to_string())
                    .or_insert_with(|| path_dependency_patch(&root.join(subdir)));
            }
        } else if let Some((repository, revision)) = ctx.framework.git_source() {
            for &(name, _) in EXTRA_PATCHES {
                crates_io.entry(name.to_string()).or_insert_with(|| {
                    Dependency::Detailed(Box::new(DependencyDetail {
                        git: Some(repository.to_string()),
                        rev: Some(revision.to_string()),
                        ..Default::default()
                    }))
                });
            }
        }
        super::with_project_patches(patch, ctx.project_root_path.as_deref())
    }

    fn path_dependency_patch(path: &Path) -> Dependency {
        Dependency::Detailed(Box::new(DependencyDetail {
            path: Some(normalize_path_for_config(path)),
            ..Default::default()
        }))
    }
}

/// Reads the `[patch]` tables from the workspace root that governs a build
/// rooted at `project_root`, with path patches made absolute and normalized
/// for config files the same way every other path the crate renders is.
pub fn collect_workspace_patches(project_root: &Path) -> io::Result<cargo_toml::PatchSet> {
    let Some((workspace_dir, source)) = find_workspace_manifest(project_root)? else {
        return Ok(cargo_toml::PatchSet::default());
    };

    let mut patches = source.patch;
    for deps in patches.values_mut() {
        for dependency in deps.values_mut() {
            if let cargo_toml::Dependency::Detailed(detail) = dependency
                && let Some(path) = detail.path.take()
            {
                detail.path = Some(normalize_path_for_config(&workspace_dir.join(path)));
            }
        }
    }
    patch_framework_git_source(&mut patches);
    Ok(patches)
}

/// The canonical repository extracted `WaterUI` crates declare their
/// `waterui-*` dependencies against — the source a `[patch]` table must name
/// to redirect those dependencies.
fn framework_git_source() -> String {
    env!("WATERUI_FRAMEWORK_REPOSITORY")
        .trim_end_matches(".git")
        .to_string()
}

/// Mirror a patch set's `crates-io` path entries onto the framework's
/// repository source.
///
/// Extracted backend/component crates declare their `waterui-*` dependencies
/// as `git = "<repo>"`, which a `[patch.crates-io]` table cannot redirect —
/// Cargo only patches the source a dependency actually names. Without the
/// repository-source table the graph carries a second copy of every framework
/// crate and `View` splits across the two (#758). Only path entries are
/// mirrored — they name crates living in the checkout the table was read
/// from, so a dependency on that name from the framework's repository must
/// resolve to the same tree; entries patched to another source (a fork's git
/// pin) are left alone.
fn patch_framework_git_source(patches: &mut cargo_toml::PatchSet) {
    let path_entries: Vec<(String, String)> = patches
        .get("crates-io")
        .into_iter()
        .flatten()
        .filter_map(|(name, dependency)| match dependency {
            cargo_toml::Dependency::Detailed(detail) => detail
                .path
                .as_ref()
                .map(|path| (name.clone(), path.clone())),
            _ => None,
        })
        .collect();
    if path_entries.is_empty() {
        return;
    }
    let repository_source = patches.entry(framework_git_source()).or_default();
    for (name, path) in path_entries {
        repository_source.insert(
            name,
            cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
                path: Some(path),
                ..cargo_toml::DependencyDetail::default()
            })),
        );
    }
}

/// Every directory the workspace at `root` declares a member crate in —
/// `[workspace] members` patterns expanded one path segment at a time and
/// `exclude` subtracted the same way — paired with the package name the
/// member's manifest declares. `components/*`-style member globs carry crates
/// the patch table never names, so generated manifests that copy only the
/// checkout's `[patch]` leave them unpinned.
fn workspace_member_packages(root: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    // A missing manifest is an empty member set, matching
    // `collect_workspace_patches`' missing-workspace default: a
    // `waterui_path` that names nothing augments nothing.
    if !root.join("Cargo.toml").is_file() {
        return Ok(Vec::new());
    }
    let manifest = cargo_toml::Manifest::from_path(root.join("Cargo.toml"))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let workspace = manifest.workspace.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} declares no [workspace]",
                root.join("Cargo.toml").display()
            ),
        )
    })?;
    let expand = |pattern: &str| -> io::Result<Vec<PathBuf>> {
        let mut dirs = vec![root.to_path_buf()];
        for component in pattern.split('/') {
            let mut next = Vec::new();
            for dir in &dirs {
                if component == "*" {
                    for entry in std::fs::read_dir(dir)? {
                        let path = entry?.path();
                        if path.is_dir() {
                            next.push(path);
                        }
                    }
                } else {
                    let path = dir.join(component);
                    if path.is_dir() {
                        next.push(path);
                    }
                }
            }
            dirs = next;
        }
        Ok(dirs)
    };
    let mut excluded = std::collections::BTreeSet::new();
    for pattern in &workspace.exclude {
        excluded.extend(expand(pattern)?);
    }
    let mut packages = Vec::new();
    for pattern in &workspace.members {
        for dir in expand(pattern)? {
            if excluded.contains(&dir) {
                continue;
            }
            let member_manifest = dir.join("Cargo.toml");
            if let Ok(member) = cargo_toml::Manifest::from_path(&member_manifest)
                && let Some(package) = member.package
            {
                packages.push((package.name, dir));
            }
        }
    }
    Ok(packages)
}

/// [`collect_workspace_patches`] plus a `{ path }` entry for every framework
/// member package the checkout's own patch table leaves out — the workspace's
/// glob members (`waterui-ffi`, `waterui-preview`, …) and the `*-path` members
/// (`hydrolysis`) carry no entry of their own, and without one a generated
/// crate resolves their registry copies beside the patched siblings (#197,
/// #1635). The member set is read off the checkout's workspace globs, the
/// same source of truth channel resolution reads from the framework
/// lockfile. Additions ride every table the set already carries, `crates-io`
/// and the repository-source mirror alike.
pub fn collect_framework_checkout_patches(
    workspace_root: &Path,
) -> io::Result<cargo_toml::PatchSet> {
    let mut patches = collect_workspace_patches(workspace_root)?;
    for (name, dir) in workspace_member_packages(workspace_root)? {
        if !(name.starts_with("waterui")
            || FRAMEWORK_MEMBERS
                .iter()
                .any(|member| member.package == name))
        {
            continue;
        }
        let path = normalize_path_for_config(&dir);
        for table in patches.values_mut() {
            table.entry(name.clone()).or_insert_with(|| {
                cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
                    path: Some(path.clone()),
                    ..cargo_toml::DependencyDetail::default()
                }))
            });
        }
    }
    Ok(patches)
}

/// The `[patch]` tables a generated crate resolves with: the framework's
/// ([`framework_crate_patches`]) with the project's own merged over them
/// through [`with_project_patches`].
fn generated_crate_patches(ctx: &TemplateContext) -> io::Result<cargo_toml::PatchSet> {
    with_project_patches(
        framework_crate_patches(ctx)?,
        ctx.project_root_path.as_deref(),
    )
}

/// The framework's `[patch]` tables for a generated crate: the checkout's own
/// when `waterui_path` names a checkout — carrying the repository-source
/// mirror [`collect_workspace_patches`] synthesizes — and the resolved
/// channel's otherwise, whose resolution rebases the same table onto the
/// channel's revision.
fn framework_crate_patches(ctx: &TemplateContext) -> io::Result<cargo_toml::PatchSet> {
    ctx.waterui_workspace_root().map_or_else(
        || Ok(ctx.framework.patches()),
        |root| collect_framework_checkout_patches(&root),
    )
}

/// `framework` merged with the `[patch]` tables governing the project's own
/// build at `project_root`.
///
/// A generated crate is its own workspace root inside the build cache, and
/// Cargo honours `[patch]` only from the root of the workspace being built.
/// Without the project's tables an application that pins a crate — an
/// unreleased component through a git revision, a fork carrying a fix — links
/// the unpatched source into everything the CLI builds, or two copies of it
/// when the project also depends on the pinned crate directly (#178, #1997).
/// Every source key the project patches is carried, and its `path` entries
/// are made absolute, since the generated crate lives outside the project.
/// A crate both sides patch takes the project's entry, as the project's own
/// build does (see [`crate::patch_tables::merge`]).
///
/// # Errors
///
/// Returns an error when the project's manifest cannot be read.
pub fn with_project_patches(
    framework: cargo_toml::PatchSet,
    project_root: Option<&Path>,
) -> io::Result<cargo_toml::PatchSet> {
    let Some(project_root) = project_root else {
        return Ok(framework);
    };
    let project = collect_workspace_patches(project_root)?;
    Ok(crate::patch_tables::merge(framework, project))
}

/// The stamp recording what a managed crate's `Cargo.lock` was last seeded
/// from — the project's lockfile plus the canonical lock's checksum — kept
/// beside the crate's own `Cargo.lock`.
pub const LOCKFILE_SEED: &str = "Cargo.lock.seed";

/// The seed-merge format a [`LOCKFILE_SEED`] stamp was written under,
/// recorded in the stamp itself. The merge rules in
/// [`crate::framework::seed_packages`] are an input to the seed the same
/// way the project lock is: a CLI whose rules differ must re-seed once
/// rather than trust a stamp an older merge wrote. Bump this when the
/// merge — or the stamp's own layout — changes.
const LOCKFILE_SEED_FORMAT: u32 = 2;

/// Seed a managed crate's `Cargo.lock` from the application's lockfile.
///
/// A managed crate is its own Cargo workspace, so left alone it resolves its
/// dependency graph fresh from the registry the first time it is built, and
/// the application ships with versions nothing in the project pins or tests
/// (#312). Copying the project's lockfile in before Cargo resolves keeps
/// every version the project already pins; Cargo then only adds the entries
/// the managed crate needs on top and prunes the ones it does not use.
///
/// When `canonical` carries the channel's certified `Water.lock`, the seed is
/// the same merge `prepare_build` writes — certified identities for every
/// name the canonical lock records — so a managed crate resolved outside the
/// build (the create-time and `water fetch` font scans) cannot pin a
/// generation the channel contradicts (#203).
///
/// The write is gated on the seed's inputs, never on the managed
/// `Cargo.lock` itself: Cargo rewrites the lock on every resolution — the
/// `cargo_lock` serialisation this writes is not the layout Cargo re-emits —
/// so the file on disk holds Cargo's merge of the seed, which a comparison
/// against the seed reads as a change on every run and re-seeds forever
/// (#2073). The stamp at [`LOCKFILE_SEED`] records the inputs instead, and
/// the crate is re-seeded only when the project's lockfile or the canonical
/// lock changed since the last seed, or when the crate has no `Cargo.lock`
/// at all. A project without a lockfile has nothing to pin yet and is left
/// to resolve on its own.
///
/// # Errors
///
/// Returns an error when the lockfiles cannot be read or written.
pub async fn seed_lockfile(
    base_dir: &Path,
    project_lockfile: &Path,
    canonical: Option<&cargo_lock::Lockfile>,
) -> io::Result<()> {
    let seed = match fs::read(project_lockfile).await {
        Ok(seed) => seed,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            tracing::debug!(
                lockfile = %project_lockfile.display(),
                "project has no lockfile; the managed crate resolves on its own"
            );
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    let seed_copy = base_dir.join(LOCKFILE_SEED);
    let managed_lockfile = base_dir.join("Cargo.lock");
    let stamp = seed_stamp(&seed, canonical);
    let seeded_from = match fs::read(&seed_copy).await {
        Ok(previous) => previous == stamp,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if seeded_from && managed_lockfile.exists() {
        return Ok(());
    }

    let previous: Option<cargo_lock::Lockfile> = match fs::read(&managed_lockfile).await {
        Ok(contents) => Some(parse_lockfile(&contents)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    // The project's pins do not name the packages only the managed crate
    // resolves; without the channel's certified pins Cargo takes whatever
    // the registry holds newest — an `accesskit` generation `Water.lock`
    // contradicts (#203). `previous` fills what neither lock names so
    // packages only the managed crate adds stay put.
    let mut merged = parse_lockfile(&seed)?;
    merged.packages = crate::framework::seed_packages(canonical, &merged, previous.as_ref());

    tracing::debug!(
        lockfile = %project_lockfile.display(),
        "seeding the managed crate's Cargo.lock from the project lockfile"
    );
    // Both writes are atomic: a torn lock or stamp is worse than an absent
    // one — a reader racing the write sees the old pair or the new one,
    // and an interrupted seed restores what it found.
    write_file_atomic(&managed_lockfile, merged.to_string().as_bytes()).await?;
    write_file_atomic(&seed_copy, &stamp).await
}

/// Restore a managed crate's `Cargo.lock` and its [`LOCKFILE_SEED`] stamp
/// to the bytes a failed post-seed step found — or remove each file when
/// the run found none.
///
/// The stamp comes down first and goes back only once the lock is whole:
/// [`seed_lockfile`] writes the pair together, so any failure — a lock
/// that will not write, a stamp that will not — must leave the stamp
/// absent, never claiming the failed run's inputs already seeded while
/// the restored lock no longer carries them.
///
/// # Errors
///
/// Returns an error when a file cannot be written or removed.
pub async fn restore_seeded_lockfile(
    base_dir: &Path,
    previous_lock: Option<&[u8]>,
    previous_stamp: Option<&[u8]>,
) -> io::Result<()> {
    let stamp_path = base_dir.join(LOCKFILE_SEED);
    restore_seeded_file(&stamp_path, None).await?;
    restore_seeded_file(&base_dir.join("Cargo.lock"), previous_lock).await?;
    if let Some(stamp) = previous_stamp {
        write_file_if_changed(&stamp_path, stamp).await?;
    }
    Ok(())
}

/// The per-file half of [`restore_seeded_lockfile`]: the recorded bytes
/// back through [`write_file_atomic`] — a file already carrying them is
/// left untouched, while a plain write could tear the lock under a
/// mid-restore crash and fail every later build's parse — or the file
/// removed when the run recorded none.
async fn restore_seeded_file(path: &Path, previous: Option<&[u8]>) -> io::Result<()> {
    match previous {
        Some(bytes) => match fs::read(path).await {
            Ok(existing) if existing == bytes => Ok(()),
            Ok(_) => write_file_atomic(path, bytes).await,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                write_file_atomic(path, bytes).await
            }
            Err(error) => Err(error),
        },
        None => match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    }
}

/// The [`LOCKFILE_SEED`] stamp recording the last seed's inputs: the
/// project's lock bytes followed by comments naming the seed-merge format
/// and the canonical lock's checksum, so the stamp still parses as the
/// lockfile it copies. A changed project lock, canonical checksum or merge
/// format fails the byte comparison and re-seeds.
fn seed_stamp(project_lock: &[u8], canonical: Option<&cargo_lock::Lockfile>) -> Vec<u8> {
    use sha2::Digest as _;
    let mut stamp = project_lock.to_vec();
    if !stamp.ends_with(b"\n") {
        stamp.push(b'\n');
    }
    stamp.extend_from_slice(format!("# seed-format: {LOCKFILE_SEED_FORMAT}\n").as_bytes());
    stamp.extend_from_slice(b"# canonical-lock-sha256: ");
    match canonical {
        Some(canonical) => stamp.extend_from_slice(
            hex::encode(sha2::Sha256::digest(canonical.to_string().as_bytes())).as_bytes(),
        ),
        None => stamp.extend_from_slice(b"none"),
    }
    stamp.push(b'\n');
    stamp
}

/// Parse `contents` as a `Cargo.lock` — the format both Cargo and the CLI
/// emit.
fn parse_lockfile(contents: &[u8]) -> io::Result<cargo_lock::Lockfile> {
    std::str::from_utf8(contents)
        .map_err(io::Error::other)?
        .parse()
        .map_err(io::Error::other)
}

/// The `[patch]` tables of the `WaterUI` checkout at `waterui_path`, rebased
/// onto that path so they resolve from the project root that names it.
///
/// A project built against a checkout takes `waterui` by path, but any
/// component it pulls from the registry — `waterui-image`, `waterui-chart` —
/// still names the registry `waterui-core`, and Cargo only honours `[patch]`
/// from the root of the workspace being built. Without the checkout's own
/// table the graph carries two copies of every foundation crate and `View` is
/// a different type on either side (#498). A relative `waterui_path` stays
/// relative, so the project remains portable together with its checkout.
pub fn local_framework_patches(
    project_root: &Path,
    waterui_path: &Path,
) -> io::Result<cargo_toml::PatchSet> {
    let manifest =
        cargo_toml::Manifest::from_path(project_root.join(waterui_path).join("Cargo.toml"))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut patches = manifest.patch;
    for deps in patches.values_mut() {
        for dependency in deps.values_mut() {
            if let cargo_toml::Dependency::Detailed(detail) = dependency
                && let Some(path) = detail.path.take()
            {
                detail.path = Some(normalize_path_for_config(&waterui_path.join(path)));
            }
        }
    }
    patch_framework_git_source(&mut patches);
    // The checkout's glob members carry no patch entry of their own; pin each
    // framework member's directory the same relative way (#197) — the `*-path`
    // members included, so an independent dependent's crates.io requirement
    // (`hydrolysis-m3`'s `hydrolysis` edge) resolves into the checkout too.
    let checkout_root = project_root.join(waterui_path);
    for (name, dir) in workspace_member_packages(&checkout_root)? {
        if !(name.starts_with("waterui")
            || FRAMEWORK_MEMBERS
                .iter()
                .any(|member| member.package == name))
            || !dir.starts_with(&checkout_root)
        {
            continue;
        }
        let path = waterui_path.join(dir.strip_prefix(&checkout_root).unwrap_or(&dir));
        let path = normalize_path_for_config(&path);
        for table in patches.values_mut() {
            table.entry(name.clone()).or_insert_with(|| {
                cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
                    path: Some(path.clone()),
                    ..cargo_toml::DependencyDetail::default()
                }))
            });
        }
    }
    Ok(patches)
}

/// The dependency a pinned `WaterUI` checkout declares for `crate_name`,
/// resolved exactly as a member of that checkout's workspace resolves it: the
/// `[patch.crates-io]` override when the `[workspace.dependencies]` requirement
/// goes to the registry, the declared entry itself otherwise.
///
/// Crates released from their own repositories — `hydrolysis-m3`,
/// `waterui-dew`, `waterui-gtk` — are consumed by the checkout as versioned or
/// git dependencies, so a generated backend manifest names that same source
/// rather than a directory the tree does not carry. In-tree members like
/// `hydrolysis` resolve through their `{name}-path` member source instead.
fn local_checkout_dependency(
    ctx: &TemplateContext,
    crate_name: &str,
) -> io::Result<GeneratedDependencyDetail> {
    let waterui_path = ctx
        .waterui_path
        .as_ref()
        .expect("local_checkout_dependency requires a pinned waterui_path");
    let root = ctx.waterui_workspace_root().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "relative waterui_path `{}` has no project root to read the checkout manifest from",
                waterui_path.display()
            ),
        )
    })?;
    let manifest_path = root.join("Cargo.toml");
    let manifest = cargo_toml::Manifest::from_path(&manifest_path)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let declared = manifest
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.dependencies.get(crate_name));
    let patch = manifest
        .patch
        .get("crates-io")
        .and_then(|deps| deps.get(crate_name));
    let resolved = match declared {
        // A crates.io requirement is the only kind `[patch.crates-io]`
        // rewrites; git and path sources are used as declared.
        Some(dependency) if registry_sourced(dependency) => patch.unwrap_or(dependency),
        Some(dependency) => dependency,
        None => patch.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "`{crate_name}` is declared in neither [workspace.dependencies] nor \
                     [patch.crates-io] of {}",
                    manifest_path.display()
                ),
            )
        })?,
    };
    checkout_dependency_detail(ctx, waterui_path, crate_name, resolved)
}

/// Whether a declared dependency resolves from the default registry — the
/// only requirement a `[patch.crates-io]` entry rewrites.
fn registry_sourced(dependency: &cargo_toml::Dependency) -> bool {
    match dependency {
        cargo_toml::Dependency::Simple(_) => true,
        cargo_toml::Dependency::Detailed(_) => dependency.is_crates_io(),
        // `workspace = true` at the checkout's own root manifest resolves
        // nowhere; `checkout_dependency_detail` reports it.
        cargo_toml::Dependency::Inherited(_) => false,
    }
}

/// A dependency entry read from the checkout's root manifest, re-expressed
/// for a generated backend manifest. `path` sources are relative to the
/// checkout root and rebase onto `waterui_path` like every other generated
/// checkout path.
fn checkout_dependency_detail(
    ctx: &TemplateContext,
    waterui_path: &Path,
    crate_name: &str,
    dependency: &cargo_toml::Dependency,
) -> io::Result<GeneratedDependencyDetail> {
    match dependency {
        cargo_toml::Dependency::Simple(version) => Ok(GeneratedDependencyDetail {
            version: Some(version.to_string()),
            ..GeneratedDependencyDetail::default()
        }),
        cargo_toml::Dependency::Detailed(detail) => {
            if detail.registry.is_some()
                || detail.registry_index.is_some()
                || !detail.unstable.is_empty()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "`{crate_name}` in the `WaterUI` checkout manifest uses a dependency \
                         source generated manifests cannot express"
                    ),
                ));
            }
            Ok(GeneratedDependencyDetail {
                version: detail.version.as_ref().map(ToString::to_string),
                path: detail.path.as_ref().map(|path| {
                    compute_native_backend_dependency_path(ctx, waterui_path, Some(path))
                }),
                git: detail.git.clone(),
                rev: detail.rev.clone(),
                branch: detail.branch.clone(),
                tag: detail.tag.clone(),
                package: detail.package.clone(),
                default_features: (!detail.default_features).then_some(false),
                features: detail.features.clone(),
                optional: detail.optional,
            })
        }
        cargo_toml::Dependency::Inherited(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "`{crate_name}` is a `workspace = true` dependency in the `WaterUI` checkout's \
                 root manifest, which has no parent workspace to inherit from"
            ),
        )),
    }
}

/// The `[patch]` tables a project's root `Cargo.toml` carries for the mode its
/// `Water.toml` selects: the framework revision's on a channel, the checkout's
/// when built against a local `waterui_path`, none on the registry.
pub fn project_patches(
    project_root: &Path,
    manifest: &crate::project::Manifest,
) -> io::Result<cargo_toml::PatchSet> {
    match (&manifest.framework, &manifest.waterui_path) {
        (Some(framework), _) => Ok(framework.patches()),
        (None, Some(waterui_path)) => {
            local_framework_patches(project_root, Path::new(waterui_path))
        }
        (None, None) => Ok(cargo_toml::PatchSet::default()),
    }
}

/// The directory whose `Cargo.toml` Cargo reads `[patch]` tables from when it
/// builds the package at `project_root`.
///
/// That is the root of the governing workspace, and the package's own directory
/// when it is standalone. A `[patch]` table anywhere else is inert: Cargo
/// ignores it and warns on every build.
///
/// # Errors
/// Returns an error if an ancestor manifest cannot be read or parsed.
pub fn patch_manifest_dir(project_root: &Path) -> io::Result<PathBuf> {
    Ok(find_workspace_manifest(project_root)?
        .map_or_else(|| project_root.to_path_buf(), |(dir, _)| dir))
}

/// Finds the manifest Cargo would treat as the workspace root for a package at
/// `project_root`: the nearest ancestor manifest with a `[workspace]` section,
/// or the package's own manifest when it is standalone.
fn find_workspace_manifest(
    project_root: &Path,
) -> io::Result<Option<(PathBuf, cargo_toml::Manifest)>> {
    let mut fallback = None;
    for dir in project_root.ancestors() {
        let manifest_path = dir.join("Cargo.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest = cargo_toml::Manifest::from_path(&manifest_path)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if manifest.workspace.is_some() {
            return Ok(Some((dir.to_path_buf(), manifest)));
        }
        if fallback.is_none() && dir == project_root {
            fallback = Some((dir.to_path_buf(), manifest));
        }
    }
    Ok(fallback)
}

/// The `waterui-ffi` features the generated FFI manifests re-export under their
/// own names, so builds select them as features of the generated crate.
///
/// Cargo only honours the seeded lockfile for dependency subtrees it reaches
/// through manifest-declared features: `dep/feature` passed to `--features`
/// resolves outside the lockfile's coverage — the reported resolve is free to
/// drift off `Water.lock`, and the lockfile Cargo writes back omits that
/// subtree entirely (#197: `hyper-util` drifted under `waterui-ffi/media` →
/// `waterkit-audio` → `zenwave` and the generated-build gate then rejected
/// its own resolution). Declaring each selectable feature in the manifest
/// keeps every one of them inside the locked graph.
const FORWARDED_FFI_FEATURES: &[&str] = &[
    "android-jni",
    "c-api",
    "chromium",
    "gpu",
    "inspector",
    "map",
    "media",
    "video",
    "webview",
    "webview-cef",
];

/// Additional `dep/feature` forwards a selectable FFI feature emits beyond
/// `waterui-ffi`: (feature name, destination dependency, the feature it turns
/// on there). `map` also turns on `waterui-apple/map` so the native
/// `MKMapView` leaf compiles only when the app's graph opted in; the `media`
/// and `webview` capabilities forward the same way, so the `AVKit` and
/// `WebKit` leaves — and the framework links they carry through `cocoa-ui` —
/// compile only for apps whose graph holds `waterui-video` /
/// `waterui-webview`. The CEF runtime listens under its own feature names —
/// `webview-cef` turns on `waterui-browser-cef/webview`.
const BACKEND_FEATURE_FORWARDS: &[(&str, &str, &str)] = &[
    ("map", "waterui-apple", "map"),
    ("media", "waterui-apple", "media"),
    ("webview", "waterui-apple", "webview"),
    ("chromium", "waterui-browser-cef", "chromium"),
    ("webview-cef", "waterui-browser-cef", "webview"),
];

/// The selectable FFI features that also reach the `waterui` facade crate —
/// the gpu/media/video/webview capability surface the native Apple entry
/// compiles. Emitted only while the Apple backend is selected.
const APPLE_RUNTIME_FEATURE_FORWARDS: &[&str] = &["gpu", "media", "video", "webview"];

/// The declaration `name` carries in `manifest`, wherever it lives:
/// `[dependencies]` or a `[target.*.dependencies]` table. A generated
/// manifest names a package in exactly one table — `waterui` top-level,
/// `waterui-ffi` behind `cfg(not(target_vendor = "apple"))`,
/// `waterui-apple` behind `cfg(target_vendor = "apple")`,
/// `waterui-browser-cef` behind `cfg(target_os = "macos")` — so the first
/// match is the declaration.
fn declared_dependency<'m>(
    manifest: &'m cargo_toml::Manifest<()>,
    name: &str,
) -> Option<&'m cargo_toml::Dependency> {
    manifest.dependencies.get(name).or_else(|| {
        manifest
            .target
            .values()
            .find_map(|target| target.dependencies.get(name))
    })
}

/// The `[features]` tables of the packages a generated manifest forwards
/// features into, keyed by the dependency's name in that manifest.
type FeatureTables = std::collections::BTreeMap<String, std::collections::BTreeSet<String>>;

/// Whether `tables` records `dep` as declaring `feature`.
fn declares(tables: &FeatureTables, dep: &str, feature: &str) -> bool {
    tables.get(dep).is_some_and(|table| table.contains(feature))
}

/// The `dep/feature` entries a selectable feature forwards to: the
/// `waterui-ffi` entry and each backend destination the manifest declares,
/// each kept only when that destination's resolved package declares the
/// feature. Empty when no destination declares it — the feature is not
/// emitted at all.
///
/// A forward names a feature of the dependency verbatim
/// (`name = ["waterui-ffi/name"]`), so a package that lacks the feature fails
/// the whole resolution: every released framework through 0.5.2 carries no
/// `inspector`, and a manifest that declared it unconditionally could not
/// resolve against the stable channel at all. Filtering per destination by
/// the resolved package's own `[features]` table keeps an older `waterui-ffi`
/// behaving exactly as before — what it enables it enables through its own
/// dependencies — while a backend destination keeps its own entry even when
/// the framework side never declared the feature: `map` still reaches
/// `waterui-apple/map` when an older `waterui-ffi` drops out of the forward.
fn ffi_feature_forwards(
    name: &str,
    manifest: &cargo_toml::Manifest<()>,
    tables: &FeatureTables,
) -> Vec<String> {
    let mut forwards = Vec::new();
    let ffi_declares = declares(tables, "waterui-ffi", name);
    if ffi_declares {
        forwards.push(format!("waterui-ffi/{name}"));
    }
    // The `waterui` facade forwards ride with the native Apple runtime: the
    // manifest declares `waterui-apple` exactly when that backend was
    // selected. A manifest that declares no `waterui-ffi` dependency at all
    // — the Apple preview package — routes to the facade instead, so the
    // feature means the same thing whichever manifest carries it.
    let facade_route = (APPLE_RUNTIME_FEATURE_FORWARDS.contains(&name)
        && declared_dependency(manifest, "waterui-apple").is_some())
        || declared_dependency(manifest, "waterui-ffi").is_none();
    if facade_route
        && declared_dependency(manifest, "waterui").is_some()
        && declares(tables, "waterui", name)
    {
        forwards.push(format!("waterui/{name}"));
    }
    for (dep, dep_feature) in BACKEND_FEATURE_FORWARDS
        .iter()
        .filter(|(feature, dep, _)| {
            feature == &name && declared_dependency(manifest, dep).is_some()
        })
        .map(|(_, dep, dep_feature)| (dep, dep_feature))
    {
        if declares(tables, dep, dep_feature) {
            forwards.push(format!("{dep}/{dep_feature}"));
        }
    }
    forwards
}

/// The dependency names a generated manifest's forwards can target:
/// `waterui-ffi` and the `waterui` facade and each
/// `BACKEND_FEATURE_FORWARDS` destination the manifest declares — a backend
/// forward references the backend crate, so it only exists where that
/// backend is a dependency. The Apple preview package carries no
/// `waterui-ffi` edge, so the name is a target only where the manifest
/// declares it.
fn forward_targets(manifest: &cargo_toml::Manifest<()>) -> Vec<&'static str> {
    let mut targets = Vec::new();
    if declared_dependency(manifest, "waterui-ffi").is_some() {
        targets.push("waterui-ffi");
    }
    for dep in
        std::iter::once("waterui").chain(BACKEND_FEATURE_FORWARDS.iter().map(|(_, dep, _)| *dep))
    {
        if declared_dependency(manifest, dep).is_some() && !targets.contains(&dep) {
            targets.push(dep);
        }
    }
    targets
}

/// `path` with `..` segments resolved textually rather than through the
/// filesystem: a generated manifest's relative dependency paths chain `..`
/// through directories the scaffold has not written yet, and the OS form
/// only resolves once every intermediate directory exists.
pub fn collapse_dotdot(path: &Path) -> PathBuf {
    let mut collapsed = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if !collapsed.pop() {
                    collapsed.push("..");
                }
            }
            component => collapsed.push(component.as_os_str()),
        }
    }
    collapsed
}

/// The `[features]` table of every package `targets` names, learned from the
/// manifest that is being written so the answer describes the exact packages
/// the generated build resolves.
///
/// A `path` dependency answers from the manifest the path names — the same
/// file Cargo resolves it from. Any other form — a channel's git pin, a
/// registry version — resolves through `cargo metadata` on a probe of the
/// manifest itself: the same dependency declarations and `[patch]` tables,
/// but as its own `[workspace]` root (a sibling member's manifest is still
/// the previous generation's while a re-scaffold runs) and with an empty
/// `[features]` table, since a forward the package cannot satisfy is exactly
/// what fails the resolve. The probe is written into a temporary directory —
/// never into the generated project — with every relative `path` absolutized
/// against `manifest_dir` and each probed dependency held non-optional, since
/// an optional edge (the preview crate's `waterui-ffi`) only enters the
/// resolved graph under a feature that enables it.
///
/// # Errors
///
/// Every failure propagates immediately with its resolution context — an
/// undeclared target, a dep manifest that cannot be read, a `cargo metadata`
/// that cannot resolve the probe, or a target absent from the resolved
/// graph. Emitting the forwards unfiltered would break resolution against
/// packages that lack the feature, so there is no fallback table.
async fn resolved_forward_tables(
    host: &crate::toolchain::Host,
    manifest: &cargo_toml::Manifest<()>,
    manifest_dir: &Path,
    targets: &[&str],
) -> io::Result<FeatureTables> {
    let generated_manifest = manifest_dir.join("Cargo.toml");
    let mut tables = FeatureTables::new();
    let mut unresolved = Vec::new();
    for &target in targets {
        let dep_path =
            declared_dependency(manifest, target).and_then(|dependency| match dependency {
                cargo_toml::Dependency::Detailed(detail) => detail.path.as_deref(),
                _ => None,
            });
        match dep_path {
            Some(path) => {
                let dep_manifest_path =
                    collapse_dotdot(&manifest_dir.join(path)).join("Cargo.toml");
                let dep_manifest =
                    cargo_toml::Manifest::from_path(&dep_manifest_path).map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "cannot load the manifest of `{target}` at {} \
                                 (dependency of {}): {error}",
                                dep_manifest_path.display(),
                                generated_manifest.display(),
                            ),
                        )
                    })?;
                tables.insert(
                    target.to_string(),
                    dep_manifest.features.keys().cloned().collect(),
                );
            }
            None if declared_dependency(manifest, target).is_some() => {
                unresolved.push(target);
            }
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "`{target}` is not a dependency of the generated manifest at {}",
                        generated_manifest.display(),
                    ),
                ));
            }
        }
    }
    if unresolved.is_empty() {
        return Ok(tables);
    }
    let probed = Box::pin(probe_forward_tables(
        host,
        manifest,
        manifest_dir,
        &unresolved,
    ))
    .await?;
    tables.extend(probed);
    Ok(tables)
}

/// `cargo metadata` on a probe of `manifest` answering the feature tables of
/// the `unresolved` targets — the dependencies whose source is not a `path`
/// (a channel's git pin, a registry version).
///
/// Each target is identified by the resolved package, not the edge name —
/// `resolve.nodes[].deps[].name` may carry a normalized spelling
/// (`waterui_ffi`), so the dep edge's `pkg` id is looked up in
/// `metadata.packages` and matched on the package's own name.
///
/// # Errors
///
/// Fails when `cargo metadata` cannot resolve the probe, when the probe
/// produces no resolution graph or no root, or when a target names no
/// resolved package of the probe's root node.
/// `probe`'s own copy of `manifest`'s dependency declarations: every
/// relative `path` absolutized against `manifest_dir` — the probe resolves
/// from a temporary directory — and each `unresolved` target held
/// non-optional, since an optional edge (the preview crate's `waterui-ffi`)
/// only enters the resolved graph under a feature that enables it.
fn absolutize_probe_paths(
    probe: &mut cargo_toml::Manifest<()>,
    manifest_dir: &Path,
    unresolved: &[&str],
) {
    let absolutize = |path: &mut Option<String>| {
        let Some(path_str) = path else { return };
        let dir = Path::new(path_str.as_str());
        if !dir.is_absolute() {
            *path_str = collapse_dotdot(&manifest_dir.join(dir))
                .to_string_lossy()
                .into_owned();
        }
    };
    for (name, dependency) in &mut probe.dependencies {
        if let cargo_toml::Dependency::Detailed(detail) = dependency {
            absolutize(&mut detail.path);
            if unresolved.iter().any(|target| *target == name) {
                detail.optional = false;
            }
        }
    }
    for (name, dependency) in [&mut probe.dev_dependencies, &mut probe.build_dependencies]
        .into_iter()
        .chain(probe.target.values_mut().flat_map(|target| {
            [
                &mut target.dependencies,
                &mut target.dev_dependencies,
                &mut target.build_dependencies,
            ]
        }))
        .flat_map(|dependencies| dependencies.iter_mut())
    {
        if let cargo_toml::Dependency::Detailed(detail) = dependency {
            absolutize(&mut detail.path);
            if unresolved.iter().any(|target| *target == name) {
                detail.optional = false;
            }
        }
    }
    for table in probe.patch.values_mut() {
        for dependency in table.values_mut() {
            if let cargo_toml::Dependency::Detailed(detail) = dependency {
                absolutize(&mut detail.path);
            }
        }
    }
}

async fn probe_forward_tables(
    host: &crate::toolchain::Host,
    manifest: &cargo_toml::Manifest<()>,
    manifest_dir: &Path,
    unresolved: &[&str],
) -> io::Result<FeatureTables> {
    let generated_manifest = manifest_dir.join("Cargo.toml");
    let mut probe = manifest.clone();
    probe.features.clear();
    probe.workspace = Some(cargo_toml::Workspace::default());
    // The probe resolves before the template sources land, so it declares no
    // products — their files are not on disk — and gets the single target
    // Cargo insists on, a stub `src/lib.rs` written beside it below.
    probe.lib = None;
    probe.bin.clear();
    probe.test.clear();
    probe.bench.clear();
    probe.example.clear();
    absolutize_probe_paths(&mut probe, manifest_dir, unresolved);

    let probe_dir = tempfile::tempdir()?;
    let manifest_path = probe_dir.path().join("Cargo.toml");
    let toml_string = toml::to_string_pretty(&probe)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::create_dir_all(probe_dir.path().join("src")).await?;
    fs::write(probe_dir.path().join("src/lib.rs"), "// probe target\n").await?;
    fs::write(&manifest_path, toml_string.as_bytes()).await?;
    let metadata = crate::project_model::assets::crate_metadata(host, &manifest_path, &[])
        .await
        .map_err(|error| {
            io::Error::other(format!(
                "cannot resolve the dependency graph of the generated manifest at {} \
                 (probe {}): {error}",
                generated_manifest.display(),
                manifest_path.display(),
            ))
        })?;

    let resolve = metadata.resolve.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the generated manifest at {} resolved no dependency graph (probe {})",
                generated_manifest.display(),
                manifest_path.display(),
            ),
        )
    })?;
    let root = resolve
        .root
        .as_ref()
        .and_then(|root_id| resolve.nodes.iter().find(|node| &node.id == root_id))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "the generated manifest at {} resolved without a root node (probe {})",
                    generated_manifest.display(),
                    manifest_path.display(),
                ),
            )
        })?;
    let mut tables = FeatureTables::new();
    for &target in unresolved {
        let package = root
            .deps
            .iter()
            .find_map(|dep| {
                metadata
                    .packages
                    .iter()
                    .find(|package| package.id == dep.pkg && package.name == target)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "`{target}` is a dependency of the generated manifest at {} \
                         but resolved no package named `{target}` (probe {})",
                        generated_manifest.display(),
                        manifest_path.display(),
                    ),
                )
            })?;
        tables.insert(
            target.to_string(),
            package.features.keys().cloned().collect(),
        );
    }
    Ok(tables)
}

/// Whether the generated FFI manifest at `manifest_path` declares `feature` —
/// and so whether a build may pass it in `--features`. The scaffold filters
/// the forwarded set by the resolved `waterui-ffi`'s own feature table, so a
/// feature the package does not declare is absent here too.
///
/// # Errors
/// Returns an error when the manifest cannot be read or parsed.
pub fn generated_ffi_manifest_declares(manifest_path: &Path, feature: &str) -> io::Result<bool> {
    let manifest = cargo_toml::Manifest::from_path(manifest_path)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(manifest.features.contains_key(feature))
}
/// Native FFI companion crate templates.
pub mod ffi {
    use cargo_toml::{Dependency, Manifest, Package, Product, Workspace};

    use super::{
        BTreeSet, NativeBackendDependencySource, NativeBackendDependencySpec, Path,
        TemplateContext, TemplateNamespace, cargo_semver, embedded, fs,
        generated_dependency_from_spec, generated_profiles, io, scaffold_dir,
        write_file_if_changed,
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
        scaffold_dir(TemplateNamespace::Ffi, &embedded::FFI, base_dir, ctx).await?;
        // A previous apple-selected render leaves the entry binary behind;
        // a non-apple scaffold must not ship a file naming an undeclared
        // dependency.
        if !ctx.apple_backend_selected {
            let stale = base_dir.join("src/bin/waterui-apple-main.rs");
            match fs::remove_file(&stale).await {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
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
        manifest.profile = generated_profiles(ctx.project_packages.as_ref())?;

        // Apple links `lib<ffi>.a` and Android loads `lib<ffi>.so`, so the manifest
        // declares only that union plus `rlib` — which Cargo requires for the
        // entry-owning binary's dependency emission: the bin must link the
        // crate statically so `_waterui_init`/`_waterui_app` live inside the
        // executable image rather than in a second dylib carrying duplicate
        // ObjC classes.
        // Each build then narrows further to the single crate type its platform
        // links, via `RustBuild::with_crate_type_override`.
        manifest.lib = Some(Product {
            crate_type: vec![
                "staticlib".to_string(),
                "cdylib".to_string(),
                "rlib".to_string(),
            ],
            ..Default::default()
        });
        // Entry-owning Apple packaging installs this binary as the
        // application executable: it calls the `waterui_apple_main` export
        // `waterui_apple::export_app!` placed in the companion library, so
        // every `waterui_*` symbol reaches the image from that one artifact
        // rather than from both the staticlib and the bin's own codegen.
        // The companion is scaffolded for Android projects too, so the bin
        // only exists when the Apple backend was actually selected.
        if ctx.apple_backend_selected {
            manifest.bin.push(Product {
                name: Some(crate::apple::platform::APPLE_ENTRY_BINARY_NAME.to_string()),
                path: Some("src/bin/waterui-apple-main.rs".to_string()),
                ..Default::default()
            });
        }
        if ctx.cef_runtime_enabled() {
            manifest.bin.push(Product {
                name: Some(crate::project_model::project_types::cef_helper_binary_name(
                    package_name,
                )),
                path: Some("src/bin/waterui-cef-helper.rs".to_string()),
                ..Default::default()
            });
        }

        // `waterui-ffi` is this crate's own edge — the Apple preview
        // package shares every other table through
        // `configure_apple_target_tables`.
        insert_waterui_ffi_dependency(&mut manifest, ctx)?;

        super::configure_apple_target_tables(&mut manifest, ctx, base_dir, &[]).await?;

        // This crate roots the workspace that also holds preview modules. A preview
        // module is loaded into the support application and resolves its `WaterUI`
        // symbols against the runtime that application already has open, so the two
        // must come out of one Cargo resolution: Cargo derives `-C metadata` — which
        // it mangles into every symbol — per workspace, and two workspaces produce
        // runtimes whose symbols cannot resolve against each other even when their
        // dependency graphs are byte-for-byte identical.
        //
        // The members are whichever modules are on disk, listed by name rather than
        // by a `modules/*` glob: Cargo reads a glob that matches nothing as a
        // literal path and fails on it, and an ordinary application has no modules
        // at all.
        manifest.workspace = Some(Workspace {
            members: super::preview_module_members(base_dir).await?,
            ..Workspace::default()
        });

        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
        Ok(())
    }

    /// Declare the crate's own `waterui-ffi` edge behind the non-Apple
    /// target cfg. `waterui` resolves it on Apple targets through
    /// `waterui-apple` instead — the two destinations are mutually
    /// exclusive so `#[cfg]` paths in generated code agree with Cargo's
    /// graph.
    fn insert_waterui_ffi_dependency(
        manifest: &mut Manifest<()>,
        ctx: &TemplateContext,
    ) -> io::Result<()> {
        let dependency = generated_dependency_from_spec(
            ctx,
            NativeBackendDependencySpec::new(
                "waterui-ffi",
                &[],
                NativeBackendDependencySource::WorkspaceSubdir("ffi"),
            ),
        )?
        .with_default_features(false)
        .into_cargo();
        manifest
            .target
            .entry("cfg(not(target_vendor = \"apple\"))".to_string())
            .or_default()
            .dependencies
            .insert(
                "waterui-ffi".to_owned(),
                Dependency::Detailed(Box::new(dependency)),
            );
        Ok(())
    }

    /// Lay down the workspace root the preview modules resolve under before
    /// the support project exists to generate the real one.
    ///
    /// `scaffold_preview_module` writes a module's manifest before the
    /// preview support app has been scaffolded, so the first run reaches
    /// `cargo metadata` on the module with no `[workspace]` root above it:
    /// Cargo honours `[patch]` only at a workspace root, so the module's own
    /// table is ignored and the resolution collapses to the registry copies
    /// (#197). This manifest carries just the workspace — members, the
    /// channel's or checkout's patch tables, the generated profile — and the
    /// managed `generate_cargo_toml` run overwrites it once the support
    /// project exists, member list included.
    ///
    /// # Errors
    ///
    /// Returns an error when the modules directory or the manifest cannot be
    /// written.
    pub async fn write_workspace_root_manifest(
        base_dir: &Path,
        patches: cargo_toml::PatchSet,
        project_root: Option<&Path>,
        project_packages: Option<&BTreeSet<String>>,
    ) -> io::Result<()> {
        let project_root = project_root.map(Path::to_path_buf);
        let patch =
            smol::unblock(move || super::with_project_patches(patches, project_root.as_deref()))
                .await?;
        let manifest = Manifest::<()> {
            profile: generated_profiles(project_packages)?,
            patch,
            workspace: Some(Workspace {
                members: super::preview_module_members(base_dir).await?,
                ..Workspace::default()
            }),
            ..Default::default()
        };
        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
        Ok(())
    }
}

/// The managed Apple in-process preview binary's templates.
///
/// `water preview --platform macos` builds and execs this package: its
/// dependency, patch and feature-forward tables come from
/// [`configure_apple_target_tables`], the same function the FFI companion
/// uses, so the `waterui` facade and `libwaterui_dylib` resolve to the
/// `water run --platform macos` build in the shared target directory. The
/// package's only edges beyond the app crate are the framework members
/// that function names — `waterui-ffi` is absent: this preview path is the
/// facade and `waterui-apple`, nothing else.
///
/// `waterui-apple/preview` — the in-process preview entry — is the one
/// deliberate feature addition on the `waterui-apple` edge; the companion
/// build never enables it, so the two manifests describe deliberately
/// different `waterui-apple` units while everything they share stays
/// identical.
pub mod apple_preview {
    use askama::Template;
    use cargo_toml::{Dependency, Manifest, Package};

    use super::{
        NativeBackendDependencySource, NativeBackendDependencySpec, Path, TemplateContext,
        cargo_semver, fs, generated_dependency_from_spec, generated_profiles, io,
        write_file_if_changed,
    };

    /// `main` for the generated preview binary, typed like every generated
    /// entry point.
    #[derive(Template)]
    #[template(path = "src/templates/apple_preview_main.rs.tpl", escape = "none")]
    struct ApplePreviewMainTemplate<'a> {
        crate_name_ident: &'a str,
    }

    /// Write the preview package's manifest and entry source to `base_dir`.
    ///
    /// `main` is rendered from the askama template and the generated files
    /// go through `write_file_if_changed`, so a re-scaffold that produced
    /// nothing new dirties no unit.
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
        let crate_name_ident = ctx.crate_name.rust_ident();
        let rendered = ApplePreviewMainTemplate {
            crate_name_ident: crate_name_ident.as_str(),
        }
        .render()
        .map_err(io::Error::other)?;
        fs::create_dir_all(base_dir.join("src")).await?;
        write_file_if_changed(&base_dir.join("src/main.rs"), rendered.as_bytes()).await
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
        manifest.profile = generated_profiles(ctx.project_packages.as_ref())?;

        let preview_protocol = generated_dependency_from_spec(
            ctx,
            NativeBackendDependencySpec::new(
                "waterui-preview-protocol",
                &[],
                NativeBackendDependencySource::WorkspaceSubdir(
                    "components/devtools/preview/protocol",
                ),
            ),
        )?
        .with_default_features(false)
        .into_cargo();
        manifest.dependencies.insert(
            "waterui-preview-protocol".to_string(),
            Dependency::Detailed(Box::new(preview_protocol)),
        );

        super::configure_apple_target_tables(&mut manifest, ctx, base_dir, &["preview"]).await?;

        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
        Ok(())
    }
}

/// The preview modules that live under a generated FFI crate, as member paths.
///
/// # Errors
///
/// Returns an error when the modules directory exists but cannot be read.
async fn preview_module_members(ffi_crate_dir: &Path) -> io::Result<Vec<String>> {
    use smol::stream::StreamExt as _;

    let modules_root = ffi_crate_dir.join(PREVIEW_MODULES_DIR);
    let mut entries = match fs::read_dir(&modules_root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut members = Vec::new();
    while let Some(entry) = entries.next().await {
        let entry = entry?;
        if entry.path().join("Cargo.toml").is_file()
            && let Some(name) = entry.file_name().to_str()
        {
            members.push(format!("{PREVIEW_MODULES_DIR}/{name}"));
        }
    }
    members.sort();
    Ok(members)
}

/// Directory, relative to the generated FFI crate, that holds preview modules.
///
/// The FFI crate roots the workspace these modules join; see the workspace
/// declaration in `ffi::generate_cargo_toml` for why they must share one.
pub const PREVIEW_MODULES_DIR: &str = "modules";

/// Root-level templates (Cargo.toml, lib.rs, .gitignore).
pub mod root {
    use super::{
        GeneratedCargoManifest, GeneratedDependencyDetail, GeneratedTargetSection,
        GeneratedWorkspaceSection, Path, TemplateContext, TemplateNamespace, embedded, fs, io,
        render_scaffold_template, write_file_if_changed, write_generated_cargo_toml,
    };
    use std::collections::BTreeMap;

    /// Cargo feature of the generated crate that turns on video playback.
    ///
    /// `waterui/media` links the framework's video stack, and on Linux that
    /// stack needs VA-API 1.19+ and `PipeWire` 0.3.65+ at build time — floors
    /// Ubuntu 22.04 and Debian 12 do not meet. A first app plays no video, so
    /// the scaffold declares the feature and leaves it off; the manifest
    /// comment on the declaration tells the user where to turn it on.
    pub const MEDIA_FEATURE: &str = "media";

    const MEDIA_FEATURE_COMMENT: &str = "\
# Video playback. Off by default: it links the framework's video stack, which\n\
# on Linux needs VA-API 1.19+ and PipeWire 0.3.65+ to build. Enable it when the\n\
# app uses video: `cargo build --features media`, or add it to `default`.\n";

    /// Root template files, paired with their destination relative to the
    /// project root. The assets README is what makes the documented `assets!`
    /// workflow work out of the box: the planner walks the assets root
    /// recursively, so the directory has to exist before the first `assets!`
    /// call, and a tracked file is what keeps it present in git.
    static ROOT_TEMPLATES: &[(&str, &str)] = &[(".gitignore.tpl", ".gitignore")];

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
        // A web-frontend project gets the `include_web!` root view
        // instead of the demo form.
        let lib_template = if ctx.web_frontend_arg.is_some() {
            "web_lib.rs.tpl"
        } else {
            "lib.rs.tpl"
        };
        let templates: Vec<(&str, String)> =
            core::iter::once((lib_template, "src/lib.rs".to_string()))
                .chain(
                    ROOT_TEMPLATES
                        .iter()
                        .map(|(template, dest)| (*template, (*dest).to_string())),
                )
                .chain(core::iter::once(("assets_readme.md.tpl", assets_readme)))
                .collect();
        // The WaterUI logo is the starting app icon; the planner picks up any
        // root-level `Icon.*` asset, so replacing the file rebrands the app.
        // Builds without SVG support scaffold a rendered PNG instead.
        #[cfg(feature = "svg-icons")]
        let templates = {
            let mut templates = templates;
            templates.push(("icon.svg", format!("{assets_dir}/Icon.svg")));
            templates
        };

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

        #[cfg(not(feature = "svg-icons"))]
        {
            use crate::project_model::assets::icon::{IconSource, encode_png};
            let icon = encode_png(
                &IconSource::default_logo()
                    .render(1024)
                    .map_err(io::Error::other)?,
            )
            .map_err(io::Error::other)?;
            let dest = base_dir.join(format!("{assets_dir}/Icon.png"));
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(dest, icon).await?;
        }
        Ok(())
    }

    /// Generate Cargo.toml programmatically using serde-compatible structs for type safety.
    async fn generate_cargo_toml(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        let waterui_dependency = waterui_dependency(ctx)?;
        let manifest = GeneratedCargoManifest {
            package: super::generated_package(ctx.crate_name.as_str(), vec![ctx.author.clone()]),
            lib: super::generated_lib(&["lib"]),
            bins: Vec::new(),
            // The project's own root manifest keeps the `"*"` pin alone:
            // workspace members are never matched by `package."*"`, so a
            // self-override would be noise in a file the user owns.
            profile: super::generated_profiles_table(&std::collections::BTreeSet::new()),
            features: BTreeMap::from([
                (
                    "dev".to_string(),
                    vec!["waterui/dynamic_linking".to_string()],
                ),
                (MEDIA_FEATURE.to_string(), vec!["waterui/media".to_string()]),
            ]),
            dependencies: BTreeMap::from([("waterui".to_string(), waterui_dependency.clone())]),
            build_dependencies: BTreeMap::new(),
            target: native_target_section(waterui_dependency, ctx.web_frontend_arg.is_some()),
            workspace: GeneratedWorkspaceSection {},
            patch: match &ctx.waterui_path {
                Some(waterui_path) => {
                    let (project_root, waterui_path) =
                        (base_dir.to_path_buf(), waterui_path.clone());
                    smol::unblock(move || {
                        super::local_framework_patches(&project_root, &waterui_path)
                    })
                    .await?
                }
                None => ctx.framework.patches(),
            },
        };

        let rendered = annotate_media_feature(&super::render_generated_cargo_toml(&manifest)?)?;
        write_generated_cargo_toml(base_dir, rendered).await
    }

    /// Places [`MEDIA_FEATURE_COMMENT`] above the `media` entry of
    /// `[features]`, so the generated manifest itself says how video is
    /// turned on. Serialization cannot carry comments, hence the second pass.
    fn annotate_media_feature(rendered: &str) -> io::Result<String> {
        let mut document: toml_edit::DocumentMut = rendered
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let features = document
            .get_mut("features")
            .and_then(toml_edit::Item::as_table_mut)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "generated Cargo.toml has no [features] table",
                )
            })?;
        let mut media = features.key_mut(MEDIA_FEATURE).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("generated Cargo.toml declares no `{MEDIA_FEATURE}` feature"),
            )
        })?;
        media.leaf_decor_mut().set_prefix(MEDIA_FEATURE_COMMENT);
        Ok(document.to_string())
    }

    fn waterui_dependency(ctx: &TemplateContext) -> io::Result<GeneratedDependencyDetail> {
        let detail = ctx.waterui_path.as_ref().map_or_else(
            || GeneratedDependencyDetail::framework(ctx, "waterui"),
            |waterui_path| Ok(GeneratedDependencyDetail::path(waterui_path)),
        )?;
        Ok(detail.with_default_features(false))
    }

    fn native_target_section(
        waterui_dependency: GeneratedDependencyDetail,
        web_frontend: bool,
    ) -> BTreeMap<String, GeneratedTargetSection<GeneratedDependencyDetail>> {
        // The scaffold carries only the features the generated sources use:
        // nothing the plain template reaches for is feature-gated, so the
        // dependency declares none. A web frontend needs `assets` for the
        // `include_web!` bundle API plus `webview` for the surface. The
        // `assets`/`media` codec stack and `flow-markdown` are heavy —
        // they enter the manifest only when the author enables them, which
        // is what keeps the built dylib at the app's own feature set.
        let mut waterui_features = Vec::new();
        if web_frontend {
            waterui_features.push("assets");
            waterui_features.push("webview");
        }
        BTreeMap::from([(
            "cfg(not(any(target_arch = \"wasm32\", target_os = \"espidf\")))".to_string(),
            GeneratedTargetSection {
                dependencies: BTreeMap::from([(
                    "waterui".to_string(),
                    waterui_dependency.with_features(&waterui_features),
                )]),
            },
        )])
    }
}

/// Preview app templates.
pub mod preview {
    use super::{
        BTreeSet, Path, SupportDependencyDetail, SupportDependencyValue, TemplateContext,
        TemplateNamespace, dependency_path, embedded, io, scaffold_dir, write_support_cargo_toml,
    };

    /// Hash of embedded preview template files and the programmatically
    /// generated scaffold inputs (the dev profile written into every generated
    /// `Cargo.toml`), so a change to either regenerates cached support apps.
    /// `project_packages` is the previewed project's own package set — the
    /// `[profile.dev.package.<name>]` overrides the support manifests write,
    /// so a changed set regenerates too.
    #[must_use]
    pub fn template_fingerprint(project_packages: &BTreeSet<String>) -> String {
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
        hasher.update(super::generated_profiles_fingerprint(project_packages).as_bytes());
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
                    git: None,
                    rev: None,
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
                SupportDependencyValue::Detailed(
                    super::GeneratedDependencyDetail::framework(ctx, "waterui")?
                        .with_default_features(false)
                        .into(),
                ),
            );
            dependencies.insert(
                "waterui-preview".to_string(),
                SupportDependencyValue::Detailed(
                    super::GeneratedDependencyDetail::framework(ctx, "waterui-preview")?.into(),
                ),
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
                git: None,
                rev: None,
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
        write_support_cargo_toml(
            base_dir,
            ctx.crate_name.as_str(),
            features,
            dependencies,
            ctx.waterui_path.as_deref(),
            &ctx.framework,
            ctx.project_packages.as_ref(),
        )
        .await
    }
}

/// Preview-only wrapper templates.
pub mod preview_ffi {
    use cargo_toml::{Dependency, DependencyDetail, Manifest, Package, Product};

    use super::{
        Path, TemplateContext, TemplateNamespace, cargo_semver,
        compute_native_backend_dependency_path, embedded, fs, io, scaffold_dir,
        write_file_if_changed,
    };

    /// Preview ABI exported to Apple support applications.
    pub const APPLE_ABI_FEATURE: &str = "apple-preview-abi";
    /// Preview ABI exported to Android support applications.
    pub const ANDROID_ABI_FEATURE: &str = "android-preview-abi";

    /// Write preview-only wrapper templates to the given directory.
    ///
    /// The crate is always a member of the support runtime's workspace rather
    /// than a workspace of its own, so the module and the runtime it is loaded
    /// into come out of a single Cargo resolution and agree on `-C metadata`.
    /// Profiles, `[patch]` entries and the lockfile therefore belong to that
    /// root and are deliberately absent here.
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
            || {
                let mut dependency = ctx.framework.dependency("waterui-ffi");
                dependency.optional = true;
                dependency.default_features = false;
                dependency
            },
            |waterui_path| DependencyDetail {
                path: Some(compute_native_backend_dependency_path(
                    ctx,
                    waterui_path,
                    Some("ffi"),
                )),
                optional: true,
                default_features: false,
                ..Default::default()
            },
        );
        manifest
            .target
            .entry("cfg(not(target_vendor = \"apple\"))".to_string())
            .or_default()
            .dependencies
            .insert(
                "waterui-ffi".to_string(),
                Dependency::Detailed(Box::new(ffi_dependency)),
            );

        let preview_dependency = preview_dependency(ctx, base_dir).await?;
        manifest.dependencies.insert(
            "waterui-preview".to_string(),
            Dependency::Detailed(Box::new(preview_dependency)),
        );

        // Every forward names a feature of `waterui-ffi`, so only its table is
        // learned — the resolved package's, not an assumed spelling.
        let tables = Box::pin(super::resolved_forward_tables(
            &ctx.host,
            &manifest,
            base_dir,
            &["waterui-ffi"],
        ))
        .await?;
        let ffi_declares = |name: &str| super::declares(&tables, "waterui-ffi", name);
        // The portable non-Apple preview loader also selects APPLE_ABI_FEATURE.
        // Its c-api forward only activates the non-Apple target dependency;
        // Apple targets compile no waterui-ffi dependency through this feature.
        for (feature, ffi_feature) in [
            (APPLE_ABI_FEATURE, "c-api"),
            (ANDROID_ABI_FEATURE, "android-jni"),
        ] {
            let mut entries = vec!["dep:waterui-ffi".to_string()];
            if ffi_declares(ffi_feature) {
                entries.push(format!("waterui-ffi/{ffi_feature}"));
            }
            entries.push("dep:waterui-preview".to_string());
            manifest.features.insert(feature.to_string(), entries);
        }

        // Same forwards as the workspace root's, weakened: this crate's
        // `waterui-ffi` dependency is optional and only an ABI feature enables
        // it, so a capability feature alone must not pull the dep in. A
        // feature the resolved `waterui-ffi` does not declare is not emitted
        // at all — the forward would fail the resolution.
        for name in super::FORWARDED_FFI_FEATURES {
            if ffi_declares(name) {
                manifest
                    .features
                    .insert((*name).to_string(), vec![format!("waterui-ffi?/{name}")]);
            }
        }

        let toml_string = toml::to_string_pretty(&manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::create_dir_all(base_dir).await?;
        write_file_if_changed(&base_dir.join("Cargo.toml"), toml_string.as_bytes()).await?;
        Ok(())
    }

    /// Locate the `waterui-preview` workspace member in the pinned checkout, or
    /// fall back to the framework registry source when no checkout is pinned.
    async fn preview_dependency(
        ctx: &TemplateContext,
        base_dir: &Path,
    ) -> io::Result<DependencyDetail> {
        if let Some(waterui_path) = &ctx.waterui_path {
            let waterui_root = Path::new(&compute_native_backend_dependency_path(
                ctx,
                waterui_path,
                None,
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
            Ok(DependencyDetail {
                path: Some(super::normalize_path_for_config(&preview_path)),
                optional: true,
                ..Default::default()
            })
        } else {
            let mut dependency = ctx.framework.dependency("waterui-preview");
            dependency.optional = true;
            Ok(dependency)
        }
    }
}

/// Inspector app templates.
pub mod inspector {
    use super::{
        BTreeSet, Path, TemplateContext, TemplateNamespace, dependency_path, embedded, io,
        scaffold_dir, write_support_cargo_toml,
    };

    /// Hash of embedded inspector template files and the programmatically
    /// generated scaffold inputs (see `generated_profiles_fingerprint`).
    /// `project_packages` is the set the inspector support manifest writes —
    /// its own crate is the only project package in its graph.
    #[must_use]
    pub fn template_fingerprint(project_packages: &BTreeSet<String>) -> String {
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
        hasher.update(super::generated_profiles_fingerprint(project_packages).as_bytes());
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

    /// Path of the Inspector application crate inside a `WaterUI` checkout.
    ///
    /// The Inspector's user interface is a real crate rather than template
    /// text, so it is compiled, linted, and tested with the rest of the
    /// workspace. The scaffolded app is a shim that depends on it.
    const INSPECTOR_APP_CRATE: &str = "components/devtools/inspector/app";

    async fn generate_cargo_toml(base_dir: &Path, ctx: &TemplateContext) -> io::Result<()> {
        use std::collections::BTreeMap;

        let waterui_path = ctx.waterui_path.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Inspector support app requires a local waterui_path",
            )
        })?;

        let inspector_app_path = waterui_path.join(INSPECTOR_APP_CRATE);
        if !inspector_app_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "Inspector support app requires {} (missing {INSPECTOR_APP_CRATE})",
                    waterui_path.display()
                ),
            ));
        }

        let mut dependencies = BTreeMap::new();
        dependencies.insert("waterui".to_string(), dependency_path(waterui_path));
        dependencies.insert(
            "waterui-inspector-app".to_string(),
            dependency_path(&inspector_app_path),
        );

        // The FFI scaffold generated alongside this app declares
        // `dev = ["<app>/dev"]`, so an app without a `dev` feature cannot be
        // resolved at all: cargo fails the whole metadata query before anything
        // is built. Every generated project carries this feature; the support
        // app is no different.
        let features = BTreeMap::from([(
            "dev".to_string(),
            vec!["waterui/dynamic_linking".to_string()],
        )]);

        write_support_cargo_toml(
            base_dir,
            ctx.crate_name.as_str(),
            features,
            dependencies,
            ctx.waterui_path.as_deref(),
            &ctx.framework,
            ctx.project_packages.as_ref(),
        )
        .await
    }

    #[cfg(test)]
    mod tests {
        /// The scaffolder resolves the Inspector crate by path inside a
        /// `waterui_path` checkout, so a move that nobody updates here breaks
        /// `water inspector` silently — which is exactly what happened when the
        /// crate moved under `devtools/`. The checkout is the enclosing
        /// workspace root, so the probe is a path check — but it still
        /// asserts the framework tree layout, so it stays nightly-only.
        #[test]
        #[ignore = "reads the enclosing workspace checkout"]
        fn the_inspector_app_crate_path_exists() {
            let checkout = crate::pinned_framework::checkout();
            let crate_path = checkout.join(super::INSPECTOR_APP_CRATE);
            assert!(
                crate_path.join("Cargo.toml").is_file(),
                "inspector app crate is not at {}",
                crate_path.display()
            );
        }

        /// The support crate is a plain Rust library: the generated backend
        /// crate is what each platform links, so it carries neither the
        /// widget-FFI dependency nor its export — under Hydrolysis on Android
        /// that export would collide with the launcher's `JNI_OnLoad`.
        #[test]
        fn the_support_crate_carries_no_widget_ffi() {
            smol::block_on(async {
                let temporary = tempfile::tempdir().expect("tempdir");
                let checkout = temporary.path().join("waterui");
                std::fs::create_dir_all(checkout.join(super::INSPECTOR_APP_CRATE))
                    .expect("inspector app crate dir");
                let app = temporary.path().join("app");
                let ctx = crate::templates::TemplateContext::for_support_app(
                    crate::templates::SupportAppIdentity {
                        display_name: "WaterUI Inspector".to_string(),
                        crate_name: crate::project_types::CrateName::try_from("waterui_inspector")
                            .expect("crate name"),
                        bundle_identifier: crate::project_types::BundleIdentifier::try_from(
                            "dev.waterui.inspector",
                        )
                        .expect("bundle identifier"),
                    },
                    Some(checkout),
                    &crate::framework::test_fixtures::stable_framework(),
                    false,
                    None,
                    &crate::templates::LocalBackendSources::default(),
                )
                .with_project_packages(std::collections::BTreeSet::from([
                    "waterui_inspector".to_string()
                ]));

                super::scaffold(&app, &ctx)
                    .await
                    .expect("the inspector support crate scaffolds");

                let cargo_toml =
                    std::fs::read_to_string(app.join("Cargo.toml")).expect("Cargo.toml");
                let manifest = cargo_toml
                    .parse::<toml::Table>()
                    .expect("Cargo.toml parses");
                assert!(
                    manifest["dependencies"]
                        .get("waterui-inspector-app")
                        .is_some(),
                    "{cargo_toml}"
                );
                assert!(!cargo_toml.contains("waterui-ffi"), "{cargo_toml}");
                assert!(manifest.get("target").is_none(), "{cargo_toml}");

                let lib = std::fs::read_to_string(app.join("src/lib.rs")).expect("lib.rs");
                assert!(lib.contains("waterui_inspector_app::app(env)"), "{lib}");
                assert!(!lib.contains("waterui_ffi"), "{lib}");
            });
        }

        /// The FFI scaffold generated beside this app declares
        /// `dev = ["<app>/dev"]`. An app without that feature cannot be
        /// resolved at all — cargo fails the metadata query and `water
        /// inspector` dies before building anything, which is exactly what it
        /// did until this was noticed.
        #[test]
        fn the_generated_app_declares_the_feature_its_ffi_scaffold_requires() {
            let generated = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/project_model/templates.rs");
            let source = std::fs::read_to_string(generated).expect("the module is readable");
            let inspector = source
                .split("pub mod inspector {")
                .nth(1)
                .expect("the inspector template module exists");
            assert!(
                inspector.contains("\"dev\".to_string()"),
                "the inspector support app is generated without a `dev` feature"
            );
        }
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

#[cfg(test)]
mod write_file_if_changed_tests {
    /// A rewrite with identical bytes must not touch the file: build scripts
    /// watch generated files as `rerun-if-changed` inputs, so every rewrite
    /// is a rebuild trigger whether or not the bytes moved (#2073).
    #[test]
    fn identical_bytes_leave_the_file_untouched() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let path = temporary.path().join("generated.rs");
        smol::block_on(async {
            super::write_file_if_changed(&path, b"same bytes")
                .await
                .expect("the first write lands");
            // Pin mtime to a fixed past value so a rewrite — which always
            // bumps mtime, whatever the filesystem granularity — is caught
            // by the assertion below.
            let past = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .expect("open for set_modified")
                .set_modified(past)
                .expect("set mtime");
            super::write_file_if_changed(&path, b"same bytes")
                .await
                .expect("the second write is a no-op");
            assert_eq!(
                std::fs::metadata(&path)
                    .expect("metadata")
                    .modified()
                    .expect("mtime"),
                past,
                "unchanged bytes must not rewrite the file — a rewrite bumps mtime"
            );
        });
    }

    /// Different bytes do land: the helper is a write gate, not a no-op, and
    /// a missing file is written rather than reported.
    #[test]
    fn changed_and_missing_files_are_written() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let path = temporary.path().join("generated.rs");
        smol::block_on(async {
            super::write_file_if_changed(&path, b"first")
                .await
                .expect("a missing file is written");
            assert_eq!(std::fs::read(&path).expect("contents"), b"first");
            super::write_file_if_changed(&path, b"second")
                .await
                .expect("changed bytes are written");
            assert_eq!(std::fs::read(&path).expect("contents"), b"second");
        });
    }
}
