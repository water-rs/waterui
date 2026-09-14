//! Toolchain diagnostics for the `water doctor` command.
//!
//! [`doctor`] runs every check against an explicit [`Host`], so the report is
//! fully determined by that host's environment, PATH, and filesystem — never
//! by ambient process state. Each [`DoctorItem`] carries a stable
//! machine-readable `id` (`DoctorItem::id`) for `--json` output and tests.

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;

use eyre;
use serde::{Deserialize, Serialize};
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
    gtk4::toolchain::Gtk4Toolchain,
    toolchain::{
        Host, Installation, Toolchain, ToolchainError, UnfixableToolchain,
        cmake::Cmake,
        linux::LinuxSystemToolchain,
        rust::RustToolchain,
        sccache::Sccache,
        web::{Wasm32UnknownUnknownTarget, WasmPack},
        windows_arm64_llvm::WindowsArm64LlvmToolchain,
    },
};

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

/// A single item in the doctor report.
pub struct DoctorItem {
    /// Stable machine-readable identifier (e.g. `android-sdk`).
    pub id: &'static str,
    /// Human-readable name of the toolchain or component.
    pub name: &'static str,
    /// Status of the check.
    pub status: CheckStatus,
    /// Optional message with details or suggestions.
    pub message: Option<String>,
    /// Optional installation function if the issue can be fixed automatically.
    pub install_fn: Option<BoxedInstallFn>,
}

impl std::fmt::Debug for DoctorItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoctorItem")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("status", &self.status)
            .field("message", &self.message)
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
    /// Event discriminator for the record stream.
    pub event: Cow<'static, str>,
    /// Stable machine-readable item identifier.
    pub id: Cow<'static, str>,
    /// Human-readable item name.
    pub name: Cow<'static, str>,
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
            event: Cow::Borrowed("doctor-item"),
            id: Cow::Borrowed(item.id),
            name: Cow::Borrowed(item.name),
            status: Cow::Borrowed(item.status.as_str()),
            fixable: item.is_fixable(),
            message: item.message.clone(),
        }
    }
}

impl DoctorItem {
    const fn ok(id: &'static str, name: &'static str) -> Self {
        Self {
            id,
            name,
            status: CheckStatus::Ok,
            message: None,
            install_fn: None,
        }
    }

    fn missing(id: &'static str, name: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            name,
            status: CheckStatus::Missing,
            message: Some(message.into()),
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
        Self {
            id,
            name,
            status: CheckStatus::Missing,
            message: Some(message.into()),
            install_fn: Some(Box::new(move || {
                Box::pin(async move { installation.install(&host).await.map_err(Into::into) })
            })),
        }
    }

    const fn skipped(id: &'static str, name: &'static str) -> Self {
        Self {
            id,
            name,
            status: CheckStatus::Skipped,
            message: None,
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
            status: CheckStatus::Skipped,
            message: Some(message.into()),
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
    /// Distribution packages the Linux backends build against.
    pub const LINUX_SYSTEM_PACKAGES: &str = "linux-system-packages";
    /// GTK4/pango pkg-config probes.
    pub const GTK4: &str = "gtk4";
    /// `sccache` compile cache.
    pub const SCCACHE: &str = "sccache";
}

fn unfixable_message(error: &UnfixableToolchain) -> String {
    format!(
        "Cannot auto-fix: {}. Next step: {}",
        error.message(),
        error.suggestion()
    )
}

async fn push_toolchain_check<T>(
    host: &Host,
    items: &mut Vec<DoctorItem>,
    id: &'static str,
    name: &'static str,
    fixable_message: &'static str,
    toolchain: T,
) where
    T: Toolchain,
    T::Installation: Send + 'static,
{
    match toolchain.check(host).await {
        Ok(()) => items.push(DoctorItem::ok(id, name)),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                id,
                name,
                fixable_message,
                installation,
                host,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(id, name, unfixable_message(&error)));
        }
    }
}

async fn push_toolchain_check_with_unfixable<T, F>(
    host: &Host,
    items: &mut Vec<DoctorItem>,
    id: &'static str,
    name: &'static str,
    fixable_message: &'static str,
    toolchain: T,
    unfixable_message_fn: F,
) where
    T: Toolchain,
    T::Installation: Send + 'static,
    F: FnOnce(&UnfixableToolchain) -> String,
{
    match toolchain.check(host).await {
        Ok(()) => items.push(DoctorItem::ok(id, name)),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                id,
                name,
                fixable_message,
                installation,
                host,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(id, name, unfixable_message_fn(&error)));
        }
    }
}

async fn push_apple_checks(host: &Host, items: &mut Vec<DoctorItem>) {
    if !cfg!(target_os = "macos") {
        items.push(DoctorItem::skipped(ids::XCODE, "Xcode"));
        items.push(DoctorItem::skipped(ids::IOS_SDK, "iOS SDK"));
        items.push(DoctorItem::skipped(
            ids::IOS_SIMULATOR_SDK,
            "iOS Simulator SDK",
        ));
        items.push(DoctorItem::skipped(ids::IOS_SIMULATORS, "iOS Simulators"));
        items.push(DoctorItem::skipped(ids::MACOS_SDK, "macOS SDK"));
        return;
    }

    push_simple_check(items, ids::XCODE, "Xcode", Xcode.check(host).await);
    push_simple_check(
        items,
        ids::IOS_SDK,
        "iOS SDK",
        AppleSdk::Ios.check(host).await,
    );
    push_simple_check(
        items,
        ids::IOS_SIMULATOR_SDK,
        "iOS Simulator SDK",
        AppleSdk::IosSimulator.check(host).await,
    );
    push_ios_simulator_check(host, items).await;
    push_simple_check(
        items,
        ids::MACOS_SDK,
        "macOS SDK",
        AppleSdk::Macos.check(host).await,
    );
}

fn push_simple_check(
    items: &mut Vec<DoctorItem>,
    id: &'static str,
    name: &'static str,
    result: Result<(), impl std::fmt::Display>,
) {
    match result {
        Ok(()) => items.push(DoctorItem::ok(id, name)),
        Err(error) => items.push(DoctorItem::missing(id, name, error.to_string())),
    }
}

async fn push_ios_simulator_check(host: &Host, items: &mut Vec<DoctorItem>) {
    match AppleSimulator::scan_ios(host).await {
        Ok(simulators) if simulators.is_empty() => items.push(DoctorItem::missing(
            ids::IOS_SIMULATORS,
            "iOS Simulators",
            "No iOS simulators available. Install a simulator runtime in Xcode Settings > Platforms.",
        )),
        Ok(_) => items.push(DoctorItem::ok(ids::IOS_SIMULATORS, "iOS Simulators")),
        Err(error) => items.push(DoctorItem::missing(
            ids::IOS_SIMULATORS,
            "iOS Simulators",
            format!("Failed to list iOS simulators: {error}"),
        )),
    }
}

async fn push_android_sdk_checks(host: &Host, items: &mut Vec<DoctorItem>) -> bool {
    push_toolchain_check(
        host,
        items,
        ids::ANDROID_SDK,
        "Android SDK",
        "Android SDK is missing (automatic install is supported on this host)",
        AndroidSdk,
    )
    .await;

    AndroidSdk::sdkmanager_path(host).await.is_some()
}

async fn push_android_component_checks(host: &Host, items: &mut Vec<DoctorItem>, sdk_ready: bool) {
    if !sdk_ready {
        push_blocked_android_component_checks(items);
        return;
    }

    push_toolchain_check(
        host,
        items,
        ids::ANDROID_PLATFORM_TOOLS,
        "Android Platform-Tools (adb)",
        "Required for `water run --platform android`",
        AndroidPlatformTools,
    )
    .await;
    push_toolchain_check(
        host,
        items,
        ids::ANDROID_SDK_PLATFORMS,
        "Android SDK Platforms",
        "Required for Android build/package workflows",
        AndroidSdkPlatforms,
    )
    .await;
    push_toolchain_check(
        host,
        items,
        ids::ANDROID_BUILD_TOOLS,
        "Android SDK Build-Tools (d8)",
        "Required for Android build/package workflows",
        AndroidBuildTools,
    )
    .await;
    push_toolchain_check(
        host,
        items,
        ids::ANDROID_NDK,
        "Android NDK",
        "Required for Android build/package workflows",
        AndroidNdk,
    )
    .await;
    push_toolchain_check(
        host,
        items,
        ids::ANDROID_RUST_TARGETS,
        "Android Rust Targets",
        "Required for Android Rust cross-compilation",
        AndroidRustTargets::default(),
    )
    .await;
}

/// Items emitted when the SDK is missing, in the same order as the probed
/// branch above so `--json` ordering does not depend on the diagnosis path.
fn push_blocked_android_component_checks(items: &mut Vec<DoctorItem>) {
    for (id, name) in [
        (ids::ANDROID_PLATFORM_TOOLS, "Android Platform-Tools (adb)"),
        (ids::ANDROID_SDK_PLATFORMS, "Android SDK Platforms"),
        (ids::ANDROID_BUILD_TOOLS, "Android SDK Build-Tools (d8)"),
        (ids::ANDROID_NDK, "Android NDK"),
        (ids::ANDROID_RUST_TARGETS, "Android Rust Targets"),
    ] {
        items.push(DoctorItem::missing(
            id,
            name,
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        ));
    }
}

async fn push_android_run_target_check(host: &Host, items: &mut Vec<DoctorItem>) {
    if AndroidSdk::adb_path(host).is_none() {
        items.push(DoctorItem::missing(
            ids::ANDROID_RUN_TARGETS,
            "Android Run Targets",
            "Blocked: Android Platform-Tools (`adb`) is not ready yet.",
        ));
        return;
    }

    match AndroidDevice::scan(host).await {
        Ok(devices) if !devices.is_empty() => {
            items.push(DoctorItem::ok(ids::ANDROID_RUN_TARGETS, "Android Run Targets"));
        }
        Ok(_) => match AndroidPlatform::list_avds(host).await {
            Ok(avds) if !avds.is_empty() => {
                items.push(DoctorItem::ok(ids::ANDROID_RUN_TARGETS, "Android Run Targets"));
            }
            Ok(_) => items.push(DoctorItem::missing(
                ids::ANDROID_RUN_TARGETS,
                "Android Run Targets",
                "No connected Android devices and no emulator AVDs were found. Connect a device or create an AVD.",
            )),
            Err(error) => items.push(DoctorItem::missing(
                ids::ANDROID_RUN_TARGETS,
                "Android Run Targets",
                format!(
                    "No connected Android devices and failed to list AVDs: {error}. Install Android emulator components or connect a device."
                ),
            )),
        },
        Err(error) => items.push(DoctorItem::missing(
            ids::ANDROID_RUN_TARGETS,
            "Android Run Targets",
            format!("Failed to query Android devices via adb: {error}"),
        )),
    }
}

async fn push_desktop_and_web_checks(host: &Host, items: &mut Vec<DoctorItem>) {
    push_toolchain_check(
        host,
        items,
        ids::CMAKE,
        "Host CMake",
        "Required for native Rust dependencies in Android builds",
        Cmake::default(),
    )
    .await;

    if WindowsArm64LlvmToolchain::required_on_host() {
        push_toolchain_check(
            host,
            items,
            ids::WINDOWS_ARM64_LLVM,
            "Windows ARM64 LLVM toolchain",
            "Required by native assembly dependencies in Windows ARM64 hydrolysis builds",
            WindowsArm64LlvmToolchain,
        )
        .await;
    } else {
        items.push(DoctorItem::skipped_with_message(
            ids::WINDOWS_ARM64_LLVM,
            "Windows ARM64 LLVM toolchain",
            "Only required on Windows ARM64 hosts for native assembly dependencies.",
        ));
    }

    push_toolchain_check(
        host,
        items,
        ids::JAVA,
        "Java",
        "Required for Android Gradle builds",
        Java,
    )
    .await;
    push_toolchain_check(
        host,
        items,
        ids::KOTLIN,
        "Kotlin",
        "Required for Android Kotlin helper compilation",
        Kotlin,
    )
    .await;
    push_toolchain_check_with_unfixable(
        host,
        items,
        ids::WASM32_TARGET,
        "Rust wasm32 target",
        "wasm32-unknown-unknown target not installed",
        Wasm32UnknownUnknownTarget,
        ToString::to_string,
    )
    .await;
    push_toolchain_check_with_unfixable(
        host,
        items,
        ids::WASM_PACK,
        "wasm-pack",
        "wasm-pack not found (required for web packaging)",
        WasmPack,
        ToString::to_string,
    )
    .await;
}

async fn push_linux_checks(host: &Host, items: &mut Vec<DoctorItem>) {
    if !cfg!(target_os = "linux") {
        items.push(DoctorItem::skipped(
            ids::LINUX_SYSTEM_PACKAGES,
            "Linux system packages",
        ));
        items.push(DoctorItem::skipped(ids::GTK4, "GTK4"));
        return;
    }

    let linux_packages_fixable = match LinuxSystemToolchain.check(host).await {
        Ok(()) => {
            items.push(DoctorItem::ok(
                ids::LINUX_SYSTEM_PACKAGES,
                "Linux system packages",
            ));
            false
        }
        Err(ToolchainError::Fixable(installation)) => {
            let msg = format!(
                "Missing packages for {}: {}. Install command: {}",
                installation.package_manager_name(),
                installation.missing_packages().join(", "),
                installation.install_command_hint(),
            );
            items.push(DoctorItem::fixable(
                ids::LINUX_SYSTEM_PACKAGES,
                "Linux system packages",
                msg,
                installation,
                host,
            ));
            true
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(
                ids::LINUX_SYSTEM_PACKAGES,
                "Linux system packages",
                unfixable_message(&error),
            ));
            false
        }
    };

    match Gtk4Toolchain.check(host).await {
        Ok(()) => items.push(DoctorItem::ok(ids::GTK4, "GTK4")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                ids::GTK4,
                "GTK4",
                "GTK4 dependencies are missing",
                installation,
                host,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            if linux_packages_fixable {
                items.push(DoctorItem::missing(
                    ids::GTK4,
                    "GTK4",
                    "GTK4 probe failed because required Linux packages are missing. Run `water doctor --fix` to install Linux system packages, then re-run `water doctor`.",
                ));
            } else {
                items.push(DoctorItem::missing(
                    ids::GTK4,
                    "GTK4",
                    unfixable_message(&error),
                ));
            }
        }
    }
}

/// Run diagnostics on all toolchains on `host` and return a report.
///
/// Item order is fixed and platform branching is driven by `cfg!`, so two runs
/// on equal hosts produce identical item sequences — the property the
/// orchestration tests and `--json` consumers rely on.
pub async fn doctor(host: &Host) -> Vec<DoctorItem> {
    let mut items = Vec::new();
    push_apple_checks(host, &mut items).await;
    push_rust_toolchain_check(host, &mut items).await;
    let sdk_ready = push_android_sdk_checks(host, &mut items).await;
    push_android_component_checks(host, &mut items, sdk_ready).await;
    push_android_run_target_check(host, &mut items).await;
    push_desktop_and_web_checks(host, &mut items).await;
    push_linux_checks(host, &mut items).await;
    push_toolchain_check(
        host,
        &mut items,
        ids::SCCACHE,
        "sccache",
        "sccache not found (recommended for faster builds)",
        Sccache,
    )
    .await;

    items
}

async fn push_rust_toolchain_check(host: &Host, items: &mut Vec<DoctorItem>) {
    match RustToolchain.check(host).await {
        Ok(()) => items.push(DoctorItem::ok(ids::RUST, "Rust toolchain")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                ids::RUST,
                "Rust toolchain",
                format!(
                    "Rust toolchain is missing, outdated, or incomplete. Planned automatic fixes: {}",
                    installation.summary()
                ),
                installation,
                host,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => items.push(DoctorItem::missing(
            ids::RUST,
            "Rust toolchain",
            unfixable_message(&error),
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::{CheckStatus, DoctorItemRecord, doctor, ids};
    use crate::toolchain::testing::TestMachine;

    /// The complete per-OS identifier set in emission order. Apple items are
    /// skipped (not omitted) off macOS and Linux items skipped off Linux, so
    /// the sequence is identical on every host.
    const EXPECTED_IDS: &[&str] = &[
        ids::XCODE,
        ids::IOS_SDK,
        ids::IOS_SIMULATOR_SDK,
        ids::IOS_SIMULATORS,
        ids::MACOS_SDK,
        ids::RUST,
        ids::ANDROID_SDK,
        ids::ANDROID_PLATFORM_TOOLS,
        ids::ANDROID_SDK_PLATFORMS,
        ids::ANDROID_BUILD_TOOLS,
        ids::ANDROID_NDK,
        ids::ANDROID_RUST_TARGETS,
        ids::ANDROID_RUN_TARGETS,
        ids::CMAKE,
        ids::WINDOWS_ARM64_LLVM,
        ids::JAVA,
        ids::KOTLIN,
        ids::WASM32_TARGET,
        ids::WASM_PACK,
        ids::LINUX_SYSTEM_PACKAGES,
        ids::GTK4,
        ids::SCCACHE,
    ];

    const ANDROID_COMPONENT_IDS: &[&str] = &[
        ids::ANDROID_PLATFORM_TOOLS,
        ids::ANDROID_SDK_PLATFORMS,
        ids::ANDROID_BUILD_TOOLS,
        ids::ANDROID_NDK,
        ids::ANDROID_RUST_TARGETS,
    ];

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
        assert_eq!(ids_of(&items), EXPECTED_IDS);
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
        // No rustup on the fake PATH → Android Rust targets are unfixable.
        assert!(!item(&items, ids::ANDROID_RUST_TARGETS).is_fixable());
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

        // wasm-pack installs via `cargo install` → always fixable.
        let wasm_pack = item(&items, ids::WASM_PACK);
        assert_eq!(wasm_pack.status, CheckStatus::Missing);
        assert!(wasm_pack.is_fixable());

        // On Linux a bare host still plans an SDK install into ~/Android/Sdk.
        #[cfg(target_os = "linux")]
        assert!(item(&items, ids::ANDROID_SDK).is_fixable());
    }

    #[test]
    fn doctor_item_record_round_trips_typed_schema() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let items = smol::block_on(doctor(&host));
        for doctor_item in &items {
            let record = DoctorItemRecord::from(doctor_item);
            assert_eq!(record.event.as_ref(), "doctor-item");
            assert!(EXPECTED_IDS.contains(&record.id.as_ref()));
            assert!(
                ["ok", "missing", "skipped"].contains(&record.status.as_ref()),
                "unexpected status {:?}",
                record.status
            );
            assert_eq!(record.fixable, doctor_item.is_fixable());
            let json = serde_json::to_string(&record).expect("serialize record");
            let back: DoctorItemRecord =
                serde_json::from_str(&json).expect("record must deserialize");
            assert_eq!(back, record);
        }
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
        machine.respond("EMULATOR_AVDS", "Medium_Phone_API_37\n");
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