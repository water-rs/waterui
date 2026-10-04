//! Toolchain diagnostics for the `water doctor` command.
//!
//! [`doctor`] runs every check against an explicit [`Host`], so the report is
//! fully determined by that host's environment, PATH, and filesystem — never
//! by ambient process state. Each [`DoctorItem`] carries a stable
//! machine-readable `id` (`DoctorItem::id`) for `--json` output and tests,
//! the [`DoctorGroup`] it is reported under, and whether the backend it
//! belongs to is in scope for the surrounding project (or, without one, for
//! the host).

use std::borrow::Cow;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use semver::Version;

use crate::platform::TargetBackend;
use crate::{
    android::{
        device::AndroidDevice,
        platform::AndroidPlatform,
        toolchain::{
            AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
            AndroidSdkPlatforms, Java, Kotlin,
        },
    },
    apple::{
        device::AppleSimulator,
        toolchain::{AppleSdk, Xcode},
    },
    device::Device,
    esp32::{chip::Esp32Chip, toolchain::Esp32Toolchain},
    framework::manifest_rust_version,
    gtk4::toolchain::Gtk4Toolchain,
    platform::TargetPlatform,
    project::Manifest,
    toolchain::{
        Host, Installation, Toolchain, ToolchainError, UnfixableToolchain,
        cargo_helpers::CargoHelpers,
        cmake::Cmake,
        dxc::Dxc,
        git::Git,
        linux::{CToolchain, LinuxSystemToolchain},
        msvc::MsvcBuildTools,
        rust::{CLI_MINIMUM_RUST_VERSION, RustToolchain},
        sccache::Sccache,
        spirv_tools::SpirvTools,
        web::{PackageManagerToolchain, WasmPack, wasm32_target},
        windows_arm64_llvm::WindowsArm64LlvmToolchain,
    },
    utils::parse_semver_version,
    winui::toolchain::WinUiToolchain,
};
use futures_util::join;
use serde::{Deserialize, Serialize};

/// Status of a toolchain check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// Toolchain is available and working.
    Ok,
    /// Toolchain is missing or misconfigured.
    Missing,
    /// Toolchain check was skipped (e.g., not applicable on this platform).
    Skipped,
}

impl CheckStatus {
    /// The stable `snake_case` label emitted in `--json` records.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::Skipped => "skipped",
        }
    }
}

/// A boxed async function that performs an installation.
pub type BoxedInstallFn =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send>> + Send>;

/// The heading an item is reported under.
///
/// The Rust toolchain, the host-level tools every workflow needs, one
/// backend, or the build helpers. Backends a host builds for by default come
/// before the optional ones in [`DoctorGroup::ORDER`], the order a new user
/// needs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DoctorGroup {
    /// The rustup-managed Rust toolchain every backend builds with.
    Rust,
    /// Tools the host itself must supply (`git`, a C toolchain) independent
    /// of any backend.
    Host,
    /// Xcode, Apple SDKs, simulators, and the Apple rustup targets.
    Apple,
    /// The Hydrolysis renderer: native desktop prerequisites and the web pieces.
    Hydrolysis,
    /// `WinUI` build prerequisites.
    WinUi,
    /// GTK4 development libraries.
    Gtk4,
    /// The Android SDK chain and its Java/Kotlin/CMake helpers.
    Android,
    /// The Espressif toolchain the Dew backend flashes with.
    Esp32,
    /// Build accelerators and cargo helper binaries.
    Helpers,
}

impl DoctorGroup {
    /// Every group in report order: Rust, host tools, the backends
    /// (host-capable ones first), then helpers.
    pub const ORDER: &[Self] = &[
        Self::Rust,
        Self::Host,
        Self::Apple,
        Self::Hydrolysis,
        Self::WinUi,
        Self::Gtk4,
        Self::Android,
        Self::Esp32,
        Self::Helpers,
    ];

    /// The stable `snake_case` label emitted in `--json` records.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Host => "host",
            Self::Apple => "apple",
            Self::Hydrolysis => "hydrolysis",
            Self::WinUi => "winui",
            Self::Gtk4 => "gtk4",
            Self::Android => "android",
            Self::Esp32 => "esp32",
            Self::Helpers => "helpers",
        }
    }

    /// The heading the terminal prints for the group.
    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self {
            Self::Rust => "Rust toolchain",
            Self::Host => "Host tools",
            Self::Apple => "Apple (iOS, macOS)",
            Self::Hydrolysis => "Hydrolysis (desktop, web)",
            Self::WinUi => "WinUI",
            Self::Gtk4 => "GTK4",
            Self::Android => "Android",
            Self::Esp32 => "ESP32 (Dew)",
            Self::Helpers => "Build helpers",
        }
    }

    /// The backend the group checks for; `None` for the Rust toolchain,
    /// host tools, and the helpers, which are always in scope.
    #[must_use]
    pub const fn backend(&self) -> Option<TargetBackend> {
        match self {
            Self::Rust | Self::Host | Self::Helpers => None,
            Self::Apple => Some(TargetBackend::Apple),
            Self::Hydrolysis => Some(TargetBackend::Hydrolysis),
            Self::WinUi => Some(TargetBackend::WinUi),
            Self::Gtk4 => Some(TargetBackend::Gtk4),
            Self::Android => Some(TargetBackend::Android),
            Self::Esp32 => Some(TargetBackend::Dew),
        }
    }

    /// The group an item id is reported under.
    ///
    /// # Panics
    /// Panics on an id outside [`ids::ALL`]; every item constructor passes a
    /// constant from that module.
    #[must_use]
    pub fn of(id: &str) -> Self {
        match id {
            ids::RUST => Self::Rust,
            ids::GIT | ids::C_TOOLCHAIN => Self::Host,
            ids::XCODE
            | ids::IOS_SDK
            | ids::IOS_SIMULATOR_SDK
            | ids::IOS_SIMULATORS
            | ids::MACOS_SDK
            | ids::APPLE_RUST_TARGETS => Self::Apple,
            ids::LINUX_SYSTEM_PACKAGES
            | ids::MSVC_BUILD_TOOLS
            | ids::DXC
            | ids::SPIRV_OPT
            | ids::WINDOWS_ARM64_LLVM
            | ids::WASM32_TARGET
            | ids::WASM_PACK
            | ids::WEB_PACKAGE_MANAGER => Self::Hydrolysis,
            ids::WINUI => Self::WinUi,
            ids::GTK4 => Self::Gtk4,
            ids::ANDROID_SDK
            | ids::ANDROID_PLATFORM_TOOLS
            | ids::ANDROID_SDK_PLATFORMS
            | ids::ANDROID_BUILD_TOOLS
            | ids::ANDROID_NDK
            | ids::ANDROID_RUST_TARGETS
            | ids::ANDROID_RUN_TARGETS
            | ids::CMAKE
            | ids::JAVA
            | ids::KOTLIN => Self::Android,
            ids::ESP32_TOOLCHAIN => Self::Esp32,
            ids::SCCACHE | ids::CARGO_HELPERS => Self::Helpers,
            other => unreachable!("doctor item id `{other}` is not in `ids::ALL`"),
        }
    }
}

/// One heading of the rendered report: a group, whether it is optional for
/// the current project or host, and its items in emission order.
#[derive(Debug)]
pub struct DoctorSection {
    /// The group the items belong to.
    pub group: DoctorGroup,
    /// `true` when the group's backend is out of scope, so its missing items
    /// are hints rather than failures.
    pub optional: bool,
    /// The group's items, in [`ids::ALL`] order.
    pub items: Vec<DoctorItem>,
}

/// Group `items` by [`DoctorGroup`] in report order: in-scope groups in
/// [`DoctorGroup::ORDER`], then optional groups in the same order, with
/// helpers last. Groups with no items are omitted.
#[must_use]
pub fn sections(items: Vec<DoctorItem>) -> Vec<DoctorSection> {
    let mut sections: Vec<DoctorSection> = Vec::new();
    for item in items {
        match sections
            .iter_mut()
            .find(|section| section.group == item.group)
        {
            Some(section) => section.items.push(item),
            None => sections.push(DoctorSection {
                group: item.group,
                optional: item.optional,
                items: vec![item],
            }),
        }
    }
    let position = |group: DoctorGroup| {
        DoctorGroup::ORDER
            .iter()
            .position(|candidate| *candidate == group)
            .unwrap_or_else(|| unreachable!("`DoctorGroup::ORDER` lists every group"))
    };
    sections.sort_by_key(|section| {
        (
            section.group == DoctorGroup::Helpers,
            section.optional,
            position(section.group),
        )
    });
    sections
}

/// A single item in the doctor report.
pub struct DoctorItem {
    /// Stable machine-readable identifier (e.g. `android-sdk`).
    pub id: &'static str,
    /// Human-readable name of the toolchain or component.
    pub name: &'static str,
    /// The heading the item is reported under.
    pub group: DoctorGroup,
    /// `true` when the item's backend is out of scope for the current project
    /// or host: it is reported with its hint but neither fails the run nor
    /// counts toward `--fix`.
    pub optional: bool,
    /// Status of the check.
    pub status: CheckStatus,
    /// Optional message with details or suggestions.
    pub message: Option<String>,
    /// `true` when running the fix modifies the machine outside `~/.water`
    /// (a system installer, `msiexec`, ...): `water doctor --fix` asks before
    /// it runs unless `--yes` was passed or the shell is non-interactive
    /// without consent semantics. Set from [`Installation::modifies_system`].
    pub system_wide: bool,
    /// Optional installation function if the issue can be fixed automatically.
    pub install_fn: Option<BoxedInstallFn>,
}

impl std::fmt::Debug for DoctorItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoctorItem")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("group", &self.group)
            .field("optional", &self.optional)
            .field("status", &self.status)
            .field("message", &self.message)
            .field("system_wide", &self.system_wide)
            .field("install_fn", &self.install_fn.as_ref().map(|_| "..."))
            .finish()
    }
}

/// The JSON record emitted for each [`DoctorItem`] by `water doctor --json`.
///
/// Lives in the library (not the shell) so integration tests deserialize the
/// binary's stdout with the same schema the command serializes. Fields are
/// `Cow` so serialization borrows the static strings while deserialization
/// (the `--json` smoke test) produces owned values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorItemRecord {
    /// Record discriminator, like the shell's other typed records.
    #[serde(rename = "type")]
    pub ty: Cow<'static, str>,
    /// Stable machine-readable item identifier.
    pub id: Cow<'static, str>,
    /// Human-readable item name.
    pub name: Cow<'static, str>,
    /// The [`DoctorGroup`] label the item is reported under.
    pub group: Cow<'static, str>,
    /// Whether the item's backend is out of scope for the project or host.
    pub optional: bool,
    /// `ok`, `missing`, or `skipped`.
    pub status: Cow<'static, str>,
    /// Whether `--fix` can remediate the item automatically.
    pub fixable: bool,
    /// Detail or suggestion shown to the user, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl From<&DoctorItem> for DoctorItemRecord {
    fn from(item: &DoctorItem) -> Self {
        Self {
            ty: Cow::Borrowed("doctor-item"),
            id: Cow::Borrowed(item.id),
            name: Cow::Borrowed(item.name),
            group: Cow::Borrowed(item.group.as_str()),
            optional: item.optional,
            status: Cow::Borrowed(item.status.as_str()),
            fixable: item.is_fixable(),
            message: item.message.clone(),
        }
    }
}

impl DoctorItem {
    fn ok(id: &'static str, name: &'static str) -> Self {
        Self {
            id,
            name,
            group: DoctorGroup::of(id),
            optional: false,
            status: CheckStatus::Ok,
            message: None,
            system_wide: false,
            install_fn: None,
        }
    }

    fn missing(id: &'static str, name: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            name,
            group: DoctorGroup::of(id),
            optional: false,
            status: CheckStatus::Missing,
            message: Some(message.into()),
            system_wide: false,
            install_fn: None,
        }
    }

    fn fixable<I: Installation + Send + 'static>(
        id: &'static str,
        name: &'static str,
        message: impl Into<String>,
        installation: I,
        host: &Host,
    ) -> Self {
        let host = host.clone();
        let system_wide = installation.modifies_system();
        Self {
            id,
            name,
            group: DoctorGroup::of(id),
            optional: false,
            status: CheckStatus::Missing,
            message: Some(message.into()),
            system_wide,
            install_fn: Some(Box::new(move || {
                Box::pin(async move { installation.install(&host).await.map_err(Into::into) })
            })),
        }
    }

    fn skipped(id: &'static str, name: &'static str) -> Self {
        Self {
            id,
            name,
            group: DoctorGroup::of(id),
            optional: false,
            status: CheckStatus::Skipped,
            message: None,
            system_wide: false,
            install_fn: None,
        }
    }

    fn skipped_with_message(
        id: &'static str,
        name: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id,
            name,
            group: DoctorGroup::of(id),
            optional: false,
            status: CheckStatus::Skipped,
            message: Some(message.into()),
            system_wide: false,
            install_fn: None,
        }
    }

    /// Returns `true` if the issue can be fixed automatically.
    #[must_use]
    pub const fn is_fixable(&self) -> bool {
        self.install_fn.is_some()
    }
}

/// Stable identifiers for every item in the doctor report.
///
/// These are the contract asserted by `water doctor --json` consumers and the
/// integration test; renaming one is a breaking change to that stream.
pub mod ids {
    /// `xcodebuild`/`xcode-select` presence.
    pub const XCODE: &str = "xcode";
    /// iOS device SDK via `xcrun --sdk iphoneos`.
    pub const IOS_SDK: &str = "ios-sdk";
    /// iOS simulator SDK via `xcrun --sdk iphonesimulator`.
    pub const IOS_SIMULATOR_SDK: &str = "ios-simulator-sdk";
    /// At least one iOS simulator runtime/device.
    pub const IOS_SIMULATORS: &str = "ios-simulators";
    /// macOS SDK via `xcrun --sdk macosx`.
    pub const MACOS_SDK: &str = "macos-sdk";
    /// rustup-managed Rust toolchain, version floor, and host target.
    pub const RUST: &str = "rust";
    /// `git`, required by `water create` to initialize the project repository.
    pub const GIT: &str = "git";
    /// The C compiler driver (`cc`) and linker (`ld`) native builds invoke.
    pub const C_TOOLCHAIN: &str = "c-toolchain";
    /// iOS device and simulator rustup targets on the selected toolchain.
    pub const APPLE_RUST_TARGETS: &str = "apple-rust-targets";
    /// Android SDK root + `sdkmanager`.
    pub const ANDROID_SDK: &str = "android-sdk";
    /// `platform-tools` (`adb`).
    pub const ANDROID_PLATFORM_TOOLS: &str = "android-platform-tools";
    /// `platforms;android-*` packages (`android.jar`).
    pub const ANDROID_SDK_PLATFORMS: &str = "android-sdk-platforms";
    /// `build-tools;*` packages (`d8`).
    pub const ANDROID_BUILD_TOOLS: &str = "android-build-tools";
    /// Android NDK + host clang.
    pub const ANDROID_NDK: &str = "android-ndk";
    /// rustup Android targets for the configured ABIs.
    pub const ANDROID_RUST_TARGETS: &str = "android-rust-targets";
    /// A connected device or an emulator AVD to run on.
    pub const ANDROID_RUN_TARGETS: &str = "android-run-targets";
    /// Host `cmake`.
    pub const CMAKE: &str = "cmake";
    /// LLVM `clang-cl`/`llvm-lib` for Windows ARM64 assembly deps.
    pub const WINDOWS_ARM64_LLVM: &str = "windows-arm64-llvm";
    /// Java runtime for Gradle.
    pub const JAVA: &str = "java";
    /// `kotlinc` compiler.
    pub const KOTLIN: &str = "kotlin";
    /// `wasm32-unknown-unknown` rustup target.
    pub const WASM32_TARGET: &str = "wasm32-target";
    /// `wasm-pack` binary.
    pub const WASM_PACK: &str = "wasm-pack";
    /// The Espressif `esp` toolchain, its clang/GCC/`rust-src` pieces, and the
    /// `espflash`/`ldproxy` helpers an ESP32 build drives.
    pub const ESP32_TOOLCHAIN: &str = "esp32-toolchain";
    /// Cargo-installed helper binaries the CLI's workflows invoke
    /// (`cargo-nextest` for `water bench`).
    pub const CARGO_HELPERS: &str = "cargo-helpers";
    /// Distribution packages the Linux backends build against.
    pub const LINUX_SYSTEM_PACKAGES: &str = "linux-system-packages";
    /// MSVC C++ build tools (`link.exe`) required to link Windows binaries.
    pub const MSVC_BUILD_TOOLS: &str = "msvc-build-tools";
    /// `dxc` (DirectX Shader Compiler) for Hydrolysis shader builds.
    pub const DXC: &str = "dxc";
    /// `spirv-opt` (SPIRV-Tools) for `cherenkov-gpu` SPIR-V builds on
    /// non-Apple native targets.
    pub const SPIRV_OPT: &str = "spirv-opt";
    /// GTK4/pango pkg-config probes.
    pub const GTK4: &str = "gtk4";
    /// `WinUI` build prerequisites on Windows hosts.
    pub const WINUI: &str = "winui";
    /// `sccache` compile cache.
    pub const SCCACHE: &str = "sccache";
    /// The `[web] package_manager` the current project's `Water.toml` declares.
    pub const WEB_PACKAGE_MANAGER: &str = "web-package-manager";

    /// Every doctor item id in emission order — grouped by
    /// [`super::DoctorGroup`] in [`super::DoctorGroup::ORDER`].
    ///
    /// This is the single source of truth for the report's identity set:
    /// [`crate::toolchain::doctor::doctor`], the lib-level ordering test, and
    /// the `water doctor --json` integration test all assert against it.
    pub const ALL: &[&str] = &[
        RUST,
        GIT,
        C_TOOLCHAIN,
        XCODE,
        IOS_SDK,
        IOS_SIMULATOR_SDK,
        IOS_SIMULATORS,
        MACOS_SDK,
        APPLE_RUST_TARGETS,
        LINUX_SYSTEM_PACKAGES,
        MSVC_BUILD_TOOLS,
        DXC,
        SPIRV_OPT,
        WINDOWS_ARM64_LLVM,
        WASM32_TARGET,
        WASM_PACK,
        WEB_PACKAGE_MANAGER,
        WINUI,
        GTK4,
        ANDROID_SDK,
        ANDROID_PLATFORM_TOOLS,
        ANDROID_SDK_PLATFORMS,
        ANDROID_BUILD_TOOLS,
        ANDROID_NDK,
        ANDROID_RUST_TARGETS,
        ANDROID_RUN_TARGETS,
        CMAKE,
        JAVA,
        KOTLIN,
        ESP32_TOOLCHAIN,
        SCCACHE,
        CARGO_HELPERS,
    ];
}

fn unfixable_message(error: &UnfixableToolchain) -> String {
    format!(
        "Cannot auto-fix: {}. Next step: {}",
        error.message(),
        error.suggestion()
    )
}

/// Why a backend is (or is not) checked in the current run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendScope {
    /// The run is inside a project, which can build with every backend.
    Selected,
    /// No project surrounds the run and the host can build for the backend.
    HostDefault,
    /// Neither: the backend is reported with its hints but does not fail
    /// the run.
    Optional,
}

impl BackendScope {
    const fn is_in_scope(self) -> bool {
        matches!(self, Self::Selected | Self::HostDefault)
    }
}

/// What `host.cwd()` tells doctor about the surrounding project: the
/// `Water.toml` manifest when the working directory is a project, and the
/// Rust floor the `rust` item enforces — the maximum of the CLI's own
/// `rust-version`, the project's `Cargo.toml` `rust-version`, and the
/// selected framework's.
struct ProjectContext {
    manifest: Option<Manifest>,
    rust_floor: Version,
}

impl ProjectContext {
    /// Whether `backend` is checked in this run and why.
    ///
    /// Inside a project every backend is [`BackendScope::Selected`]: the CLI
    /// generates whichever backend a command selects. Outside a project the host decides: the backends the
    /// machine can build for — Apple on macOS, `WinUI` on Windows, GTK4 on
    /// Linux, Hydrolysis on every desktop host — are
    /// [`BackendScope::HostDefault`]; Android and Dew need a project to
    /// select them and are optional everywhere.
    fn scope(&self, backend: TargetBackend) -> BackendScope {
        self.manifest.as_ref().map_or_else(
            || {
                let host_builds = match backend {
                    TargetBackend::Apple => cfg!(target_os = "macos"),
                    TargetBackend::WinUi => cfg!(target_os = "windows"),
                    TargetBackend::Gtk4 => cfg!(target_os = "linux"),
                    TargetBackend::Hydrolysis => true,
                    TargetBackend::Android | TargetBackend::Dew => false,
                };
                if host_builds {
                    BackendScope::HostDefault
                } else {
                    BackendScope::Optional
                }
            },
            |_| BackendScope::Selected,
        )
    }

    fn in_scope(&self, backend: TargetBackend) -> bool {
        self.scope(backend).is_in_scope()
    }

    /// The `skipped` message for a project-gated item whose backend is out
    /// of scope — only ever outside a project, since a project selects every
    /// backend.
    fn out_of_scope_message(backend: &str) -> String {
        format!(
            "Optional: checked inside a WaterUI project, which can build with the {backend} backend."
        )
    }

    /// The chips the project's ESP32 (Dew) backend can target: the chip
    /// `[esp32]` declares, or every supported chip. `None` when no project
    /// is present.
    fn esp32_chips(&self) -> Option<eyre::Result<Vec<Esp32Chip>>> {
        let manifest = self.manifest.as_ref()?;
        if let Some(config) = &manifest.esp32 {
            return Some(config.resolved_chip().map(|chip| vec![chip]));
        }
        Some(Ok(vec![
            Esp32Chip::Esp32S3,
            Esp32Chip::Esp32C3,
            Esp32Chip::Esp32P4,
        ]))
    }
}

/// The `rust-version` a `Cargo.toml` root manifest declares, when it parses.
async fn cargo_manifest_rust_version(path: &Path) -> Option<Version> {
    let manifest: toml::Value = toml::from_str(&smol::fs::read_to_string(path).await.ok()?).ok()?;
    manifest_rust_version(&manifest).ok().flatten()
}

async fn project_context(host: &Host) -> ProjectContext {
    let manifest = Manifest::open(host.cwd().join("Water.toml")).await.ok();
    let mut rust_floor = parse_semver_version(CLI_MINIMUM_RUST_VERSION)
        .unwrap_or_else(|_| unreachable!("CARGO_PKG_RUST_VERSION is valid semver"));
    if let Some(manifest) = &manifest {
        // The project's own `rust-version` and the selected framework's both
        // raise the floor; a `waterui_path` checkout's root manifest carries
        // the framework's.
        if let Some(floor) = cargo_manifest_rust_version(&host.cwd().join("Cargo.toml")).await {
            rust_floor = rust_floor.max(floor);
        }
        let framework_floor = match (&manifest.framework, &manifest.waterui_path) {
            (Some(framework), _) => framework.rust_version().cloned(),
            (None, Some(waterui_path)) => {
                let path = Path::new(waterui_path);
                let root = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    host.cwd().join(path)
                };
                cargo_manifest_rust_version(&root.join("Cargo.toml")).await
            }
            (None, None) => None,
        };
        if let Some(floor) = framework_floor {
            rust_floor = rust_floor.max(floor);
        }
    }
    ProjectContext {
        manifest,
        rust_floor,
    }
}

async fn toolchain_check<T>(
    host: &Host,
    id: &'static str,
    name: &'static str,
    fixable_message: &'static str,
    toolchain: T,
) -> DoctorItem
where
    T: Toolchain,
    T::Installation: Send + 'static,
{
    toolchain_check_with_unfixable(
        host,
        id,
        name,
        fixable_message,
        toolchain,
        unfixable_message,
    )
    .await
}

async fn toolchain_check_with_unfixable<T, F>(
    host: &Host,
    id: &'static str,
    name: &'static str,
    fixable_message: &'static str,
    toolchain: T,
    unfixable_message_fn: F,
) -> DoctorItem
where
    T: Toolchain,
    T::Installation: Send + 'static,
    F: FnOnce(&UnfixableToolchain) -> String,
{
    match toolchain.check(host).await {
        Ok(()) => DoctorItem::ok(id, name),
        Err(ToolchainError::Fixable(installation)) => {
            DoctorItem::fixable(id, name, fixable_message, installation, host)
        }
        Err(ToolchainError::Unfixable(error)) => {
            DoctorItem::missing(id, name, unfixable_message_fn(&error))
        }
    }
}

/// Xcode, the Apple SDKs, the iOS simulators, and the Apple rustup targets.
/// The `xcrun` probes are independent and run concurrently.
async fn apple_checks(host: &Host, project: &ProjectContext) -> Vec<DoctorItem> {
    if !cfg!(target_os = "macos") {
        return vec![
            DoctorItem::skipped(ids::XCODE, "Xcode"),
            DoctorItem::skipped(ids::IOS_SDK, "iOS SDK"),
            DoctorItem::skipped(ids::IOS_SIMULATOR_SDK, "iOS Simulator SDK"),
            DoctorItem::skipped(ids::IOS_SIMULATORS, "iOS Simulators"),
            DoctorItem::skipped(ids::MACOS_SDK, "macOS SDK"),
            DoctorItem::skipped_with_message(
                ids::APPLE_RUST_TARGETS,
                "Apple Rust targets",
                "Apple platforms can only be built on macOS.",
            ),
        ];
    }

    let (xcode, ios_sdk, ios_simulator_sdk, ios_simulators, macos_sdk, rust_targets) = join!(
        Xcode.check(host),
        AppleSdk::Ios.check(host),
        AppleSdk::IosSimulator.check(host),
        ios_simulator_check(host),
        AppleSdk::Macos.check(host),
        apple_rust_targets_check(host, project),
    );
    vec![
        simple_check(ids::XCODE, "Xcode", xcode),
        simple_check(ids::IOS_SDK, "iOS SDK", ios_sdk),
        simple_check(
            ids::IOS_SIMULATOR_SDK,
            "iOS Simulator SDK",
            ios_simulator_sdk,
        ),
        ios_simulators,
        simple_check(ids::MACOS_SDK, "macOS SDK", macos_sdk),
        rust_targets,
    ]
}

fn simple_check(
    id: &'static str,
    name: &'static str,
    result: Result<(), impl std::fmt::Display>,
) -> DoctorItem {
    match result {
        Ok(()) => DoctorItem::ok(id, name),
        Err(error) => DoctorItem::missing(id, name, error.to_string()),
    }
}

async fn ios_simulator_check(host: &Host) -> DoctorItem {
    const NAME: &str = "iOS Simulators";
    match AppleSimulator::scan_ios(host).await {
        Ok(simulators) if simulators.is_empty() => DoctorItem::missing(
            ids::IOS_SIMULATORS,
            NAME,
            "No iOS simulators available. Install a simulator runtime in Xcode Settings > Platforms.",
        ),
        Ok(_) => DoctorItem::ok(ids::IOS_SIMULATORS, NAME),
        Err(error) => DoctorItem::missing(
            ids::IOS_SIMULATORS,
            NAME,
            format!("Failed to list iOS simulators: {error}"),
        ),
    }
}

/// The iOS device and simulator rustup targets an Apple-backend project
/// needs on its selected toolchain. (The macOS target is the host triple the
/// `rust` item already requires.)
async fn apple_rust_targets_check(host: &Host, project: &ProjectContext) -> DoctorItem {
    const NAME: &str = "Apple Rust targets";
    if !project.in_scope(TargetBackend::Apple) {
        return DoctorItem::skipped_with_message(
            ids::APPLE_RUST_TARGETS,
            NAME,
            ProjectContext::out_of_scope_message("Apple"),
        );
    }
    toolchain_check(
        host,
        ids::APPLE_RUST_TARGETS,
        NAME,
        "Required iOS targets are missing on the selected Rust toolchain",
        crate::toolchain::rust::SelectedToolchainTargets::new(vec![
            TargetPlatform::IOS.triple().to_string(),
            TargetPlatform::IOSSimulator.triple().to_string(),
        ]),
    )
    .await
}

/// The Android SDK chain. The component probes need `sdkmanager`, so they
/// follow the SDK probe; the rustup targets, run targets, and the `CMake` /
/// Java / Kotlin helpers are independent of it and run alongside.
async fn android_checks(host: &Host, project: &ProjectContext) -> Vec<DoctorItem> {
    let components = async {
        let sdk = toolchain_check(
            host,
            ids::ANDROID_SDK,
            "Android SDK",
            "Android SDK is missing (automatic install is supported on this host)",
            AndroidSdk,
        )
        .await;
        let mut items = vec![sdk];
        if AndroidSdk::sdkmanager_path(host).await.is_some() {
            let components = join!(
                toolchain_check(
                    host,
                    ids::ANDROID_PLATFORM_TOOLS,
                    "Android Platform-Tools (adb)",
                    "Required for `water run --platform android`",
                    AndroidPlatformTools,
                ),
                toolchain_check(
                    host,
                    ids::ANDROID_SDK_PLATFORMS,
                    "Android SDK Platforms",
                    "Required for Android build/package workflows",
                    AndroidSdkPlatforms,
                ),
                toolchain_check(
                    host,
                    ids::ANDROID_BUILD_TOOLS,
                    "Android SDK Build-Tools (d8)",
                    "Required for Android build/package workflows",
                    AndroidBuildTools,
                ),
                toolchain_check(
                    host,
                    ids::ANDROID_NDK,
                    "Android NDK",
                    "Required for Android build/package workflows",
                    AndroidNdk,
                ),
            );
            items.extend(<[DoctorItem; 4]>::from(components));
        } else {
            items.extend(blocked_android_component_checks());
        }
        items
    };
    let rust_targets = async {
        // The rustup targets only need rustup — they are probed regardless
        // of SDK state, and only when Android is in scope.
        if project.in_scope(TargetBackend::Android) {
            toolchain_check(
                host,
                ids::ANDROID_RUST_TARGETS,
                "Android Rust Targets",
                "Required for Android Rust cross-compilation",
                AndroidRustTargets::default(),
            )
            .await
        } else {
            DoctorItem::skipped_with_message(
                ids::ANDROID_RUST_TARGETS,
                "Android Rust Targets",
                ProjectContext::out_of_scope_message("Android"),
            )
        }
    };
    let (mut items, rust_targets, run_targets, cmake, java, kotlin) = join!(
        components,
        rust_targets,
        android_run_target_check(host),
        toolchain_check(
            host,
            ids::CMAKE,
            "Host CMake",
            "Required for native Rust dependencies in Android builds",
            Cmake::default(),
        ),
        toolchain_check(
            host,
            ids::JAVA,
            "Java",
            "Required for Android Gradle builds",
            Java,
        ),
        toolchain_check(
            host,
            ids::KOTLIN,
            "Kotlin",
            "Required for Android Kotlin helper compilation",
            Kotlin,
        ),
    );
    items.extend([rust_targets, run_targets, cmake, java, kotlin]);
    items
}

/// Items emitted when the SDK is missing, in the same order as the probed
/// branch above so `--json` ordering does not depend on the diagnosis path.
/// `android-rust-targets` is deliberately absent: it needs only rustup, so it
/// is probed (or skipped) independently of the SDK.
fn blocked_android_component_checks() -> impl Iterator<Item = DoctorItem> {
    [
        (ids::ANDROID_PLATFORM_TOOLS, "Android Platform-Tools (adb)"),
        (ids::ANDROID_SDK_PLATFORMS, "Android SDK Platforms"),
        (ids::ANDROID_BUILD_TOOLS, "Android SDK Build-Tools (d8)"),
        (ids::ANDROID_NDK, "Android NDK"),
    ]
    .into_iter()
    .map(|(id, name)| {
        DoctorItem::missing(
            id,
            name,
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        )
    })
}

async fn android_run_target_check(host: &Host) -> DoctorItem {
    const NAME: &str = "Android Run Targets";
    if AndroidSdk::adb_path(host).is_none() {
        return DoctorItem::missing(
            ids::ANDROID_RUN_TARGETS,
            NAME,
            "Blocked: Android Platform-Tools (`adb`) is not ready yet.",
        );
    }

    match AndroidDevice::scan(host).await {
        Ok(devices) if !devices.is_empty() => DoctorItem::ok(ids::ANDROID_RUN_TARGETS, NAME),
        Ok(_) => match AndroidPlatform::list_avds(host).await {
            Ok(avds) if !avds.is_empty() => DoctorItem::ok(ids::ANDROID_RUN_TARGETS, NAME),
            Ok(_) => DoctorItem::missing(
                ids::ANDROID_RUN_TARGETS,
                NAME,
                "No connected Android devices and no emulator AVDs were found. Connect a device or create an AVD.",
            ),
            Err(error) => DoctorItem::missing(
                ids::ANDROID_RUN_TARGETS,
                NAME,
                format!(
                    "No connected Android devices and failed to list AVDs: {error}. Install Android emulator components or connect a device."
                ),
            ),
        },
        Err(error) => DoctorItem::missing(
            ids::ANDROID_RUN_TARGETS,
            NAME,
            format!("Failed to query Android devices via adb: {error}"),
        ),
    }
}

/// The Hydrolysis backend beyond the Linux system packages (probed with
/// GTK4 in [`linux_checks`]): the Windows-native prerequisites (`link.exe`,
/// `dxc`, the ARM64 LLVM pieces) and the web target's `wasm32` target,
/// `wasm-pack`, and declared package manager.
async fn hydrolysis_checks(host: &Host, project: &ProjectContext) -> Vec<DoctorItem> {
    let windows_arm64_llvm = async {
        if WindowsArm64LlvmToolchain::required_on_host() {
            toolchain_check(
                host,
                ids::WINDOWS_ARM64_LLVM,
                "Windows ARM64 LLVM toolchain",
                "Required by native assembly dependencies in Windows ARM64 hydrolysis builds",
                WindowsArm64LlvmToolchain,
            )
            .await
        } else {
            DoctorItem::skipped_with_message(
                ids::WINDOWS_ARM64_LLVM,
                "Windows ARM64 LLVM toolchain",
                "Only required on Windows ARM64 hosts for native assembly dependencies.",
            )
        }
    };
    let web = async {
        if project.in_scope(TargetBackend::Hydrolysis) {
            join!(
                toolchain_check_with_unfixable(
                    host,
                    ids::WASM32_TARGET,
                    "Rust wasm32 target",
                    "wasm32-unknown-unknown target not installed",
                    wasm32_target(),
                    ToString::to_string,
                ),
                toolchain_check_with_unfixable(
                    host,
                    ids::WASM_PACK,
                    "wasm-pack",
                    "wasm-pack not found (required for web packaging)",
                    WasmPack,
                    ToString::to_string,
                ),
            )
        } else {
            (
                DoctorItem::skipped_with_message(
                    ids::WASM32_TARGET,
                    "Rust wasm32 target",
                    ProjectContext::out_of_scope_message("hydrolysis (web)"),
                ),
                DoctorItem::skipped_with_message(
                    ids::WASM_PACK,
                    "wasm-pack",
                    ProjectContext::out_of_scope_message("hydrolysis (web)"),
                ),
            )
        }
    };
    let (
        (msvc_build_tools, dxc),
        spirv_opt,
        windows_arm64_llvm,
        (wasm32, wasm_pack),
        web_package_manager,
    ) = join!(
        windows_checks(host),
        spirv_opt_check(host, project),
        windows_arm64_llvm,
        web,
        web_package_manager_check(host, project)
    );
    let mut items = vec![
        msvc_build_tools,
        dxc,
        spirv_opt,
        windows_arm64_llvm,
        wasm32,
        wasm_pack,
    ];
    items.extend(web_package_manager);
    items
}

/// The Windows-only prerequisites of a Hydrolysis build: the MSVC C++ build
/// tools (linker) and `dxc` (shader compiler). Both are skipped items on
/// other hosts.
async fn windows_checks(host: &Host) -> (DoctorItem, DoctorItem) {
    if cfg!(target_os = "windows") {
        join!(
            toolchain_check(
                host,
                ids::MSVC_BUILD_TOOLS,
                "MSVC C++ build tools",
                "MSVC C++ build tools are missing (`link.exe` is required to link Windows binaries). `--fix` downloads the Visual Studio Build Tools installer and adds the 'C++ build tools' workload (~2 GB download, ~6 GB installed, requires administrator rights and modifies the system outside ~/.water).",
                MsvcBuildTools,
            ),
            toolchain_check(
                host,
                ids::DXC,
                "DirectX Shader Compiler (dxc)",
                "dxc is missing (Hydrolysis shader builds invoke it on Windows). `--fix` unpacks a pinned microsoft/DirectXShaderCompiler release into ~/.water/tools.",
                Dxc,
            ),
        )
    } else {
        (
            DoctorItem::skipped_with_message(
                ids::MSVC_BUILD_TOOLS,
                "MSVC C++ build tools",
                "Only required on Windows hosts.",
            ),
            DoctorItem::skipped_with_message(
                ids::DXC,
                "DirectX Shader Compiler (dxc)",
                "Only required on Windows hosts.",
            ),
        )
    }
}

/// Whether this run can produce a target `cherenkov-gpu` emits SPIR-V for.
///
/// The engine's build script invokes `spirv-opt` for every non-Apple native
/// target — Linux, Windows and Android — while Apple targets compile to
/// Metal instead. Outside a project the host decides: a Linux or Windows
/// host always has such a target in scope (GTK4/`WinUI`, and Hydrolysis's
/// own native desktop build); on macOS the only host-default targets are
/// Apple, so `spirv-opt` only matters once a project selects the Android
/// backend Hydrolysis cross-compiles to.
fn spirv_opt_required(project: &ProjectContext) -> bool {
    project.in_scope(TargetBackend::Android)
        || project.in_scope(TargetBackend::Gtk4)
        || project.in_scope(TargetBackend::WinUi)
        || (project.in_scope(TargetBackend::Hydrolysis) && !cfg!(target_os = "macos"))
}

/// `spirv-opt` (SPIRV-Tools), scoped by [`spirv_opt_required`]. The item's
/// `Ok` report carries the binary's own `--version` line.
async fn spirv_opt_check(host: &Host, project: &ProjectContext) -> DoctorItem {
    const NAME: &str = "SPIRV-Tools (spirv-opt)";
    if !spirv_opt_required(project) {
        return DoctorItem::skipped_with_message(
            ids::SPIRV_OPT,
            NAME,
            "Only required for Linux, Windows, or Android targets — Apple targets compile to Metal.",
        );
    }
    match SpirvTools.check(host).await {
        Ok(()) => {
            let mut item = DoctorItem::ok(ids::SPIRV_OPT, NAME);
            item.message = SpirvTools.version(host).await;
            item
        }
        Err(ToolchainError::Fixable(installation)) => DoctorItem::fixable(
            ids::SPIRV_OPT,
            NAME,
            installation.describe(),
            installation,
            host,
        ),
        Err(ToolchainError::Unfixable(error)) => {
            DoctorItem::missing(ids::SPIRV_OPT, NAME, unfixable_message(&error))
        }
    }
}

/// The Espressif-side toolchain — `esp` Rust fork, clang/GCC, `rust-src`,
/// `espflash`/`ldproxy`, QEMU — when the project selects a Dew/ESP32 backend.
async fn esp32_check(host: &Host, project: &ProjectContext) -> DoctorItem {
    const NAME: &str = "ESP32 toolchain";
    let Some(chips) = project.esp32_chips() else {
        return DoctorItem::skipped_with_message(
            ids::ESP32_TOOLCHAIN,
            NAME,
            ProjectContext::out_of_scope_message("ESP32"),
        );
    };
    let chips = match chips {
        Ok(chips) => chips,
        Err(error) => {
            return DoctorItem::missing(
                ids::ESP32_TOOLCHAIN,
                NAME,
                format!("Invalid `[esp32]` configuration: {error}"),
            );
        }
    };
    match Esp32Toolchain::new(chips).check(host).await {
        Ok(()) => DoctorItem::ok(ids::ESP32_TOOLCHAIN, NAME),
        Err(ToolchainError::Fixable(installation)) => DoctorItem::fixable(
            ids::ESP32_TOOLCHAIN,
            NAME,
            installation.describe(),
            installation,
            host,
        ),
        Err(ToolchainError::Unfixable(error)) => {
            DoctorItem::missing(ids::ESP32_TOOLCHAIN, NAME, unfixable_message(&error))
        }
    }
}

/// The cargo-installed helper binaries a project's workflows invoke —
/// `cargo-nextest` for `water bench`. Platform helpers that are also cargo
/// installs (`wasm-pack`, `espflash`/`ldproxy`) are covered by their own
/// platform items.
async fn cargo_helpers_check(host: &Host) -> DoctorItem {
    const NAME: &str = "Cargo helpers";
    match CargoHelpers::new(["cargo-nextest"]).check(host).await {
        Ok(()) => DoctorItem::ok(ids::CARGO_HELPERS, NAME),
        Err(ToolchainError::Fixable(installation)) => DoctorItem::fixable(
            ids::CARGO_HELPERS,
            NAME,
            installation.describe(),
            installation,
            host,
        ),
        Err(ToolchainError::Unfixable(error)) => {
            DoctorItem::missing(ids::CARGO_HELPERS, NAME, unfixable_message(&error))
        }
    }
}

/// Host-level tools every workflow needs regardless of backend: `git`, which
/// `water create` invokes to initialize the scaffolded repository, and the C
/// toolchain rustc links through.
///
/// The C toolchain is Linux-only here — macOS gets `cc`/`ld` from the Xcode
/// Command Line Tools (the `apple` group) and Windows from MSVC/LLVM — but it
/// still reports as a skipped item so every id in [`ids::ALL`] appears.
async fn host_checks(host: &Host) -> Vec<DoctorItem> {
    let git = toolchain_check(
        host,
        ids::GIT,
        "git",
        "git is missing — `water create` needs it to initialize the project repository",
        Git,
    )
    .await;

    let c_toolchain = if cfg!(target_os = "linux") {
        toolchain_check(
            host,
            ids::C_TOOLCHAIN,
            "C toolchain",
            "a C compiler (`cc`) and linker (`ld`) are missing — rustc links through `cc`",
            CToolchain,
        )
        .await
    } else {
        DoctorItem::skipped(ids::C_TOOLCHAIN, "C toolchain")
    };

    vec![git, c_toolchain]
}

/// The Linux system packages and the GTK4 probe, as
/// `(linux_system_packages, gtk4)`: the GTK4 diagnosis names the package
/// install when the packages are what is missing.
async fn linux_checks(host: &Host) -> (DoctorItem, DoctorItem) {
    const PACKAGES: &str = "Linux system packages";
    if !cfg!(target_os = "linux") {
        return (
            DoctorItem::skipped(ids::LINUX_SYSTEM_PACKAGES, PACKAGES),
            DoctorItem::skipped(ids::GTK4, "GTK4"),
        );
    }

    let (packages, gtk4) = join!(LinuxSystemToolchain.check(host), Gtk4Toolchain.check(host));
    let (packages, packages_fixable) = match packages {
        Ok(()) => (DoctorItem::ok(ids::LINUX_SYSTEM_PACKAGES, PACKAGES), false),
        Err(ToolchainError::Fixable(installation)) => {
            let msg = format!(
                "Missing packages for {}: {}. Install command: {}",
                installation.package_manager_name(),
                installation.missing_packages().join(", "),
                installation.install_command_hint(),
            );
            (
                DoctorItem::fixable(
                    ids::LINUX_SYSTEM_PACKAGES,
                    PACKAGES,
                    msg,
                    installation,
                    host,
                ),
                true,
            )
        }
        Err(ToolchainError::Unfixable(error)) => (
            DoctorItem::missing(
                ids::LINUX_SYSTEM_PACKAGES,
                PACKAGES,
                unfixable_message(&error),
            ),
            false,
        ),
    };

    let gtk4 = match gtk4 {
        Ok(()) => DoctorItem::ok(ids::GTK4, "GTK4"),
        Err(ToolchainError::Fixable(installation)) => DoctorItem::fixable(
            ids::GTK4,
            "GTK4",
            "GTK4 dependencies are missing",
            installation,
            host,
        ),
        Err(ToolchainError::Unfixable(error)) => {
            if packages_fixable {
                DoctorItem::missing(
                    ids::GTK4,
                    "GTK4",
                    "GTK4 probe failed because required Linux packages are missing. Run `water doctor --fix` to install Linux system packages, then re-run `water doctor`.",
                )
            } else {
                DoctorItem::missing(ids::GTK4, "GTK4", unfixable_message(&error))
            }
        }
    };
    (packages, gtk4)
}

async fn winui_check(host: &Host) -> DoctorItem {
    if !cfg!(target_os = "windows") {
        return DoctorItem::skipped(ids::WINUI, "WinUI");
    }

    toolchain_check(
        host,
        ids::WINUI,
        "WinUI",
        "WinUI build prerequisites are missing",
        WinUiToolchain,
    )
    .await
}

/// Run diagnostics on all toolchains on `host` and return a report.
///
/// Item order is fixed ([`ids::ALL`]) and platform branching is driven by
/// `cfg!` plus the project context `host.cwd()` resolves, so two runs on
/// equal hosts in equal projects produce identical item sequences — the
/// property the orchestration tests and `--json` consumers rely on. The
/// independent probes run concurrently; the report is returned once all of
/// them have answered.
///
/// Which backends are in scope is [`ProjectContext::scope`]'s decision:
/// inside a project the manifest's, outside it the host's. Items of an
/// out-of-scope backend are marked [`DoctorItem::optional`].
pub async fn doctor(host: &Host) -> Vec<DoctorItem> {
    let project = project_context(host).await;
    let (
        rust,
        host_tools,
        apple,
        (linux_system_packages, gtk4),
        hydrolysis,
        winui,
        android,
        esp32,
        sccache,
        cargo_helpers,
    ) = join!(
        Box::pin(rust_toolchain_check(host, &project)),
        Box::pin(host_checks(host)),
        Box::pin(apple_checks(host, &project)),
        Box::pin(linux_checks(host)),
        Box::pin(hydrolysis_checks(host, &project)),
        Box::pin(winui_check(host)),
        Box::pin(android_checks(host, &project)),
        Box::pin(esp32_check(host, &project)),
        Box::pin(toolchain_check(
            host,
            ids::SCCACHE,
            "sccache",
            "sccache is missing or too old — 0.9.0 or newer is recommended for faster builds",
            Sccache,
        )),
        Box::pin(cargo_helpers_check(host)),
    );

    let mut items = vec![rust];
    items.extend(host_tools);
    items.extend(apple);
    items.push(linux_system_packages);
    items.extend(hydrolysis);
    items.push(winui);
    items.push(gtk4);
    items.extend(android);
    items.push(esp32);
    items.push(sccache);
    items.push(cargo_helpers);
    for item in &mut items {
        item.optional = item
            .group
            .backend()
            .is_some_and(|backend| !project.in_scope(backend));
    }
    items
}

/// Checks the `[web] package_manager` the project's `Water.toml` declares.
/// Only the declared manager is probed — a project on `pnpm` is never
/// reported healthy because `bun` happens to be installed.
async fn web_package_manager_check(host: &Host, project: &ProjectContext) -> Option<DoctorItem> {
    let web = project
        .manifest
        .as_ref()
        .and_then(|manifest| manifest.web.as_ref())?;
    let package_manager = web.package_manager;
    let name: &'static str = match package_manager {
        crate::web::PackageManager::Bun => "bun (web package manager)",
        crate::web::PackageManager::Pnpm => "pnpm (web package manager)",
        crate::web::PackageManager::Npm => "npm (web package manager)",
        crate::web::PackageManager::Yarn => "yarn (web package manager)",
    };
    Some(
        toolchain_check(
            host,
            ids::WEB_PACKAGE_MANAGER,
            name,
            package_manager.install_hint(),
            PackageManagerToolchain(package_manager),
        )
        .await,
    )
}

async fn rust_toolchain_check(host: &Host, project: &ProjectContext) -> DoctorItem {
    match RustToolchain::new(&project.rust_floor).check(host).await {
        Ok(()) => DoctorItem::ok(ids::RUST, "Rust toolchain"),
        Err(ToolchainError::Fixable(installation)) => DoctorItem::fixable(
            ids::RUST,
            "Rust toolchain",
            format!(
                "Rust toolchain is missing, outdated, or incomplete. Planned automatic fixes: {}",
                installation.summary()
            ),
            installation,
            host,
        ),
        Err(ToolchainError::Unfixable(error)) => {
            DoctorItem::missing(ids::RUST, "Rust toolchain", unfixable_message(&error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendScope, CheckStatus, DoctorGroup, ProjectContext, doctor, ids, sections};
    use crate::platform::TargetBackend;
    use crate::toolchain::testing::TestMachine;
    use semver::Version;

    fn project_less() -> ProjectContext {
        ProjectContext {
            manifest: None,
            rust_floor: Version::new(1, 85, 0),
        }
    }

    fn project_with(extra: &str) -> ProjectContext {
        ProjectContext {
            manifest: Some(toml::from_str(&manifest(extra)).expect("fixture manifest must parse")),
            rust_floor: Version::new(1, 85, 0),
        }
    }

    const ANDROID_COMPONENT_IDS: &[&str] = &[
        ids::ANDROID_PLATFORM_TOOLS,
        ids::ANDROID_SDK_PLATFORMS,
        ids::ANDROID_BUILD_TOOLS,
        ids::ANDROID_NDK,
    ];

    /// A minimal `Water.toml` app manifest; `extra` is appended verbatim
    /// (`[esp32]`, `[web]`, ...).
    fn manifest(extra: &str) -> String {
        format!(
            "[package]\nname = \"Fixture\"\nbundle_identifier = \"dev.waterui.fixture\"\n\n{extra}"
        )
    }

    fn ids_of(items: &[super::DoctorItem]) -> Vec<&'static str> {
        items.iter().map(|item| item.id).collect()
    }

    fn item<'a>(items: &'a [super::DoctorItem], id: &str) -> &'a super::DoctorItem {
        items
            .iter()
            .find(|item| item.id == id)
            .unwrap_or_else(|| panic!("doctor report must contain `{id}`"))
    }

    #[test]
    fn doctor_emits_every_item_in_stable_order() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        // `WEB_PACKAGE_MANAGER` only emits when the current directory's
        // `Water.toml` declares a `[web]` section; the test CWD has none.
        let expected: Vec<&'static str> = ids::ALL
            .iter()
            .copied()
            .filter(|id| *id != ids::WEB_PACKAGE_MANAGER)
            .collect();
        assert_eq!(ids_of(&items), expected);
    }

    #[test]
    fn doctor_blocks_android_components_when_sdk_absent() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));

        assert_eq!(item(&items, ids::ANDROID_SDK).status, CheckStatus::Missing);
        for id in ANDROID_COMPONENT_IDS {
            let component = item(&items, id);
            assert_eq!(component.status, CheckStatus::Missing, "{id}");
            assert!(
                component
                    .message
                    .as_deref()
                    .is_some_and(|message| message.contains("Blocked")),
                "{id} must carry the blocked diagnostic: {:?}",
                component.message
            );
            assert!(
                !component.is_fixable(),
                "blocked {id} must not offer an install"
            );
        }

        // Without a manifest the Android rust targets are not required, so
        // the item is skipped rather than blocked or probed.
        assert_eq!(
            item(&items, ids::ANDROID_RUST_TARGETS).status,
            CheckStatus::Skipped
        );

        let run_targets = item(&items, ids::ANDROID_RUN_TARGETS);
        assert_eq!(run_targets.status, CheckStatus::Missing);
        assert!(
            run_targets
                .message
                .as_deref()
                .is_some_and(|message| message.contains("Blocked"))
        );
    }

    #[test]
    fn doctor_probes_android_components_when_sdk_ready() {
        let machine = TestMachine::new();
        machine.file("Water.toml", &manifest(""));
        let sdk = machine.install_android_sdk();
        let host = machine.host([(
            String::from("ANDROID_SDK_ROOT"),
            sdk.as_os_str().to_os_string(),
        )]);
        let items = smol::block_on(doctor(&host));

        assert_eq!(item(&items, ids::ANDROID_SDK).status, CheckStatus::Ok);
        for id in ANDROID_COMPONENT_IDS {
            let component = item(&items, id);
            assert_eq!(component.status, CheckStatus::Missing, "{id}");
            assert!(
                !component
                    .message
                    .as_deref()
                    .is_some_and(|message| message.contains("Blocked")),
                "{id} must be a real diagnosis, not the blocked marker: {:?}",
                component.message
            );
        }

        // adb / platforms / build-tools / NDK are installable via sdkmanager.
        for id in [
            ids::ANDROID_PLATFORM_TOOLS,
            ids::ANDROID_SDK_PLATFORMS,
            ids::ANDROID_BUILD_TOOLS,
            ids::ANDROID_NDK,
        ] {
            assert!(item(&items, id).is_fixable(), "{id} must be fixable");
        }
        // The manifest selects the Android backend, so the rustup targets are
        // probed; with no rustup on the fake PATH they are unfixable.
        let rust_targets = item(&items, ids::ANDROID_RUST_TARGETS);
        assert_eq!(rust_targets.status, CheckStatus::Missing);
        assert!(!rust_targets.is_fixable());
    }

    #[test]
    fn doctor_apple_items_match_platform() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        for id in [
            ids::XCODE,
            ids::IOS_SDK,
            ids::IOS_SIMULATOR_SDK,
            ids::IOS_SIMULATORS,
            ids::MACOS_SDK,
        ] {
            let status = item(&items, id).status;
            if cfg!(target_os = "macos") {
                assert_eq!(
                    status,
                    CheckStatus::Missing,
                    "{id} is probed on macOS and missing on a bare host"
                );
            } else {
                assert_eq!(
                    status,
                    CheckStatus::Skipped,
                    "{id} must be skipped off macOS"
                );
            }
        }
    }

    /// The staged `simctl list devices --json` transcript reports one healthy
    /// iPhone, so `ios-simulators` comes back `Ok` — the fake `xcrun` must
    /// answer the query and the transcript's `dataPath` must exist.
    #[test]
    #[cfg(target_os = "macos")]
    fn doctor_ios_simulators_ok_when_simctl_reports_healthy_device() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        // Retarget the transcript's `/fake/...` paths into the scratch root
        // so `data_path.exists()` holds on the declared host.
        machine.dir(
            "Library/Developer/CoreSimulator/Devices/3E8B0C4F-0000-4000-8000-000000000001/data",
        );
        let transcript = include_str!("testdata/simctl_devices.json")
            .replace("/fake/", &format!("{}/", machine.root().display()));
        machine.respond("XCRUN_SIMCTL_DEVICES", &transcript);
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        assert_eq!(
            item(&items, ids::IOS_SIMULATORS).status,
            CheckStatus::Ok,
            "a healthy simctl device must satisfy ios-simulators"
        );
    }

    #[test]
    fn doctor_linux_items_match_platform() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        for id in [ids::LINUX_SYSTEM_PACKAGES, ids::GTK4] {
            let status = item(&items, id).status;
            if cfg!(target_os = "linux") {
                assert_eq!(
                    status,
                    CheckStatus::Missing,
                    "{id} is probed on Linux and missing on a bare host"
                );
            } else {
                assert_eq!(
                    status,
                    CheckStatus::Skipped,
                    "{id} must be skipped off Linux"
                );
            }
        }
    }

    #[test]
    fn doctor_windows_llvm_skipped_where_not_required() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        let status = item(&items, ids::WINDOWS_ARM64_LLVM).status;
        if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
            assert_eq!(status, CheckStatus::Missing);
        } else {
            assert_eq!(status, CheckStatus::Skipped);
        }
    }

    #[test]
    fn doctor_fixable_and_manual_classification() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));

        // No rust tools at all → manual fix required.
        let rust = item(&items, ids::RUST);
        assert_eq!(rust.status, CheckStatus::Missing);
        assert!(!rust.is_fixable());

        // The cargo helpers need `cargo` to install → manual without it.
        let cargo_helpers = item(&items, ids::CARGO_HELPERS);
        assert_eq!(cargo_helpers.status, CheckStatus::Missing);
        assert!(!cargo_helpers.is_fixable());

        // On Linux a bare host still plans an SDK install into ~/Android/Sdk.
        #[cfg(target_os = "linux")]
        assert!(item(&items, ids::ANDROID_SDK).is_fixable());
    }

    /// With a hydrolysis backend selected and `cargo` on PATH, a missing
    /// `wasm-pack` is a `cargo install` away → fixable.
    #[test]
    fn doctor_wasm_pack_fixable_when_hydrolysis_selected() {
        let machine = TestMachine::new();
        machine.file("Water.toml", &manifest(""));
        machine.install("cargo");
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));

        let wasm_pack = item(&items, ids::WASM_PACK);
        assert_eq!(wasm_pack.status, CheckStatus::Missing);
        assert!(wasm_pack.is_fixable());
    }

    /// `spirv-opt` is probed wherever a non-Apple native target is in scope
    /// — every Linux/Windows host and any project — and skipped only on a
    /// project-less macOS host, where just Apple targets apply.
    #[test]
    fn doctor_spirv_opt_matches_the_target_scope() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        let spirv_opt = item(&items, ids::SPIRV_OPT);
        if cfg!(target_os = "macos") {
            assert_eq!(spirv_opt.status, CheckStatus::Skipped);
        } else {
            assert_eq!(spirv_opt.status, CheckStatus::Missing);
            assert!(!spirv_opt.optional);
        }

        let project_machine = TestMachine::new();
        project_machine.file("Water.toml", &manifest(""));
        let host = project_machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        let spirv_opt = item(&items, ids::SPIRV_OPT);
        assert_eq!(
            spirv_opt.status,
            CheckStatus::Missing,
            "a project can always build a SPIR-V target"
        );
    }

    /// With `spirv-opt` staged on PATH the item is `Ok` and reports the
    /// binary's own `--version` line.
    #[test]
    fn doctor_spirv_opt_reports_the_installed_version() {
        let machine = TestMachine::new();
        machine.install("spirv-opt");
        machine.file("Water.toml", &manifest(""));
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        let spirv_opt = item(&items, ids::SPIRV_OPT);
        assert_eq!(spirv_opt.status, CheckStatus::Ok);
        assert_eq!(
            spirv_opt.message.as_deref(),
            Some("spirv-opt 1.0.0 (waterui-test)")
        );
    }

    /// Outside a project the host decides: Hydrolysis everywhere, Apple on
    /// macOS, GTK4 on Linux, `WinUI` on Windows; Android and Dew optional.
    #[test]
    fn scope_outside_a_project_follows_the_host() {
        let project = project_less();
        assert_eq!(
            project.scope(TargetBackend::Hydrolysis),
            BackendScope::HostDefault
        );
        assert_eq!(
            project.scope(TargetBackend::Android),
            BackendScope::Optional
        );
        assert_eq!(project.scope(TargetBackend::Dew), BackendScope::Optional);
        let host_only = |backend, on_host: bool| {
            let expected = if on_host {
                BackendScope::HostDefault
            } else {
                BackendScope::Optional
            };
            assert_eq!(project.scope(backend), expected, "{backend:?}");
        };
        host_only(TargetBackend::Apple, cfg!(target_os = "macos"));
        host_only(TargetBackend::Gtk4, cfg!(target_os = "linux"));
        host_only(TargetBackend::WinUi, cfg!(target_os = "windows"));
    }

    /// Inside a project every backend is in scope: the CLI generates
    /// whichever one a command selects.
    #[test]
    fn scope_inside_a_project_selects_every_backend() {
        let project = project_with("");
        for backend in [
            TargetBackend::Apple,
            TargetBackend::Android,
            TargetBackend::Gtk4,
            TargetBackend::Hydrolysis,
            TargetBackend::WinUi,
            TargetBackend::Dew,
        ] {
            assert_eq!(
                project.scope(backend),
                BackendScope::Selected,
                "{backend:?}"
            );
        }
    }

    /// Outside a project the host's backends are probed and Android / ESP32
    /// are reported as optional rather than skipped-as-unselected.
    #[test]
    fn doctor_checks_the_hosts_backends_outside_a_project() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));

        for id in [ids::WASM32_TARGET, ids::WASM_PACK] {
            let hydrolysis = item(&items, id);
            assert_eq!(hydrolysis.status, CheckStatus::Missing, "{id}");
            assert!(!hydrolysis.optional, "{id} is in scope on every desktop");
        }
        let apple_targets = item(&items, ids::APPLE_RUST_TARGETS);
        if cfg!(target_os = "macos") {
            assert_eq!(apple_targets.status, CheckStatus::Missing);
            assert!(!apple_targets.optional);
        } else {
            assert_eq!(apple_targets.status, CheckStatus::Skipped);
        }
        if cfg!(target_os = "linux") {
            assert!(!item(&items, ids::GTK4).optional);
        }
        for id in [
            ids::ANDROID_SDK,
            ids::ANDROID_RUST_TARGETS,
            ids::ESP32_TOOLCHAIN,
        ] {
            assert!(
                item(&items, id).optional,
                "{id} is optional without a project"
            );
        }
        assert_eq!(
            item(&items, ids::ESP32_TOOLCHAIN).status,
            CheckStatus::Skipped
        );
    }

    /// The terminal groups follow the new-user order: Rust, the host's
    /// backends, optional backends, helpers — with every in-scope backend
    /// ahead of every optional one.
    #[test]
    fn sections_order_rust_host_optional_helpers() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let sections = sections(smol::block_on(doctor(&host)));
        let groups: Vec<DoctorGroup> = sections.iter().map(|section| section.group).collect();
        assert_eq!(groups.first(), Some(&DoctorGroup::Rust));
        assert_eq!(groups.last(), Some(&DoctorGroup::Helpers));
        let first_optional = sections.iter().position(|section| section.optional);
        let last_required_backend = sections
            .iter()
            .rposition(|section| !section.optional && section.group.backend().is_some());
        if let (Some(first_optional), Some(last_required)) = (first_optional, last_required_backend)
        {
            assert!(last_required < first_optional);
        }
        assert!(
            sections
                .iter()
                .find(|section| section.group == DoctorGroup::Android)
                .is_some_and(|section| section.optional)
        );
        assert!(
            sections
                .iter()
                .find(|section| section.group == DoctorGroup::Hydrolysis)
                .is_some_and(|section| !section.optional)
        );
    }

    /// Inside a project every backend is probed, even on a bare host.
    #[test]
    fn doctor_probes_every_backend_inside_a_project() {
        let machine = TestMachine::new();
        machine.file("Water.toml", &manifest("[esp32]\nchip = \"esp32c3\"\n"));
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));

        for id in [
            ids::ANDROID_RUST_TARGETS,
            ids::WASM32_TARGET,
            ids::WASM_PACK,
            ids::ESP32_TOOLCHAIN,
        ] {
            assert_eq!(
                item(&items, id).status,
                CheckStatus::Missing,
                "{id} must be probed on a bare host"
            );
        }
        if cfg!(target_os = "macos") {
            assert_eq!(
                item(&items, ids::APPLE_RUST_TARGETS).status,
                CheckStatus::Missing
            );
        }
    }

    /// An `[esp32]` chip the CLI does not support is a diagnostic,
    /// not a skipped item.
    #[test]
    fn doctor_reports_invalid_esp32_chip() {
        let machine = TestMachine::new();
        machine.file("Water.toml", &manifest("[esp32]\nchip = \"atmega328p\"\n"));
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));

        let esp32 = item(&items, ids::ESP32_TOOLCHAIN);
        assert_eq!(esp32.status, CheckStatus::Missing);
        assert!(
            esp32
                .message
                .as_deref()
                .is_some_and(|message| message.contains("Invalid")),
            "the invalid chip must be diagnosed: {:?}",
            esp32.message
        );
    }

    /// A `--fix` pass runs each fixable item's install; a re-diagnosis must
    /// then observe the repair — the fix-loop property `water doctor --fix`
    /// relies on.
    #[test]
    #[cfg(unix)]
    fn doctor_fix_loop_repairs_pinned_toolchain() {
        let machine = TestMachine::new();
        machine.file("Water.toml", &manifest(""));
        machine.file("rust-toolchain.toml", "[toolchain]\nchannel = \"1.90\"\n");
        for tool in ["rustup", "cargo", "rustc"] {
            machine.install(tool);
        }
        let host = machine.host([
            (
                String::from("WATERUI_FAKE_RUSTUP_TOOLCHAIN_NOT_INSTALLED"),
                String::from("1.90"),
            ),
            (
                String::from("WATERUI_FAKE_RUSTC_VERSION"),
                String::from("99.0.0"),
            ),
            (
                String::from("WATERUI_FAKE_RUSTC_HOST"),
                String::from("x86_64-unknown-fake"),
            ),
            (
                String::from("WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS"),
                String::from("x86_64-unknown-fake"),
            ),
        ]);

        let items = smol::block_on(doctor(&host));
        let rust = items
            .into_iter()
            .find(|item| item.id == ids::RUST)
            .expect("rust item");
        assert_eq!(rust.status, CheckStatus::Missing);
        let install = rust.install_fn.expect("the pin repair must be fixable");
        smol::block_on(install()).expect("install must succeed on the fake host");

        let items = smol::block_on(doctor(&host));
        assert_eq!(
            item(&items, ids::RUST).status,
            CheckStatus::Ok,
            "after `rustup toolchain install 1.90` the rust item must be ok"
        );
    }

    #[test]
    #[cfg(unix)]
    fn doctor_reports_complete_android_chain_when_fully_staged() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        machine.install_android_platform("android-37.0");
        machine.install_android_build_tools("37.0.0");
        machine.install_android_ndk("29.0.14206865");
        machine.install_android_emulator();
        machine.install("rustup");
        machine.file("Water.toml", &manifest(""));
        machine.respond("EMULATOR_AVDS", "Medium_Phone_API_37\n");
        machine.respond(
            "RUSTUP_ACTIVE_TOOLCHAIN",
            "stable-x86_64-unknown-fake (default)",
        );
        machine.respond(
            "RUSTUP_INSTALLED_TARGETS",
            &[
                "aarch64-linux-android",
                "armv7-linux-androideabi",
                "i686-linux-android",
                "x86_64-linux-android",
            ]
            .join("\n"),
        );
        let host = machine.host([(
            String::from("ANDROID_SDK_ROOT"),
            sdk.as_os_str().to_os_string(),
        )]);
        let items = smol::block_on(doctor(&host));
        for id in [
            ids::ANDROID_SDK,
            ids::ANDROID_PLATFORM_TOOLS,
            ids::ANDROID_SDK_PLATFORMS,
            ids::ANDROID_BUILD_TOOLS,
            ids::ANDROID_NDK,
            ids::ANDROID_RUST_TARGETS,
            ids::ANDROID_RUN_TARGETS,
        ] {
            assert_eq!(
                item(&items, id).status,
                CheckStatus::Ok,
                "{id} must be ok on a fully staged SDK: {:?}",
                item(&items, id).message
            );
        }
    }
}
