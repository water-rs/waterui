//! Shared toolchain checks for the terminal commands and the `water mcp`
//! `preview` tool.

use std::path::{Path, PathBuf};

use eyre::{Result, bail};

use crate::{
    android::{
        AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
        AndroidSdkPlatforms, Java, Kotlin,
        platform::{ALL_ABIS, AndroidAbi},
    },
    apple::toolchain::{AppleSdk, Xcode},
    gtk4::toolchain::Gtk4Toolchain,
    toolchain::{
        Host, Installation, Toolchain, ToolchainError,
        cmake::Cmake,
        doctor::{CheckStatus, doctor, ids},
        web::WebToolchain,
        windows_arm64_llvm::WindowsArm64LlvmToolchain,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AndroidCheckScope {
    BuildOrPackage,
    Run,
}

fn toolchain_check_message<I: Installation>(component: &str, error: &ToolchainError<I>) -> String {
    match error {
        ToolchainError::Fixable(_) => format!(
            "{component} toolchain check failed: missing dependencies can be fixed automatically with `water doctor --fix`."
        ),
        ToolchainError::Unfixable(unfixable) => {
            format!("{component} toolchain check failed: {unfixable}")
        }
    }
}

fn android_doctor_item_in_scope(id: &str, scope: AndroidCheckScope) -> bool {
    match id {
        ids::ANDROID_SDK
        | ids::ANDROID_SDK_PLATFORMS
        | ids::ANDROID_BUILD_TOOLS
        | ids::ANDROID_NDK
        | ids::ANDROID_RUST_TARGETS
        | ids::CMAKE
        | ids::JAVA
        | ids::KOTLIN => true,
        ids::ANDROID_PLATFORM_TOOLS => scope == AndroidCheckScope::Run,
        _ => false,
    }
}

fn format_path_or_missing(label: &str, path: Option<&Path>) -> String {
    path.map_or_else(
        || format!("- {label}: <not detected>"),
        |path| format!("- {label}: {}", path.display()),
    )
}

async fn android_detection_summary(host: &Host) -> String {
    let sdk_root = AndroidSdk::detect_path(host);
    let d8_jar = AndroidSdk::d8_jar_path(host);
    let ndk_root = AndroidNdk::detect_path(host);
    let java_bin: Option<PathBuf> = Java::detect_path(host).await;
    let java_home: Option<PathBuf> = Java::detect_home(host).await;

    [
        "Detected Android/JDK configuration:".to_string(),
        format_path_or_missing("Android SDK root", sdk_root.as_deref()),
        format_path_or_missing("Android build-tools d8.jar", d8_jar.as_deref()),
        format_path_or_missing("Android NDK root", ndk_root.as_deref()),
        format_path_or_missing("Java executable", java_bin.as_deref()),
        format_path_or_missing("JAVA_HOME", java_home.as_deref()),
    ]
    .join("\n")
}

fn format_doctor_missing_item(
    name: &'static str,
    message: Option<String>,
    is_fixable: bool,
) -> String {
    let mode = if is_fixable { "fixable" } else { "manual" };
    message.map_or_else(
        || format!("- {name} [{mode}]"),
        |message| format!("- {name} [{mode}]: {message}"),
    )
}

async fn android_doctor_summary(host: &Host, scope: AndroidCheckScope) -> String {
    let mut lines = vec!["Relevant doctor diagnostics:".to_string()];

    for item in doctor(host).await {
        if item.status != CheckStatus::Missing || !android_doctor_item_in_scope(item.id, scope) {
            continue;
        }
        let is_fixable = item.is_fixable();
        let message = item.message;
        lines.push(format_doctor_missing_item(item.name, message, is_fixable));
    }

    if lines.len() == 1 {
        lines.push("- No additional Android diagnostics were reported by doctor.".to_string());
    }

    lines.join("\n")
}

async fn android_failure_message<I: Installation>(
    host: &Host,
    component: &str,
    error: &ToolchainError<I>,
    scope: AndroidCheckScope,
) -> String {
    [
        toolchain_check_message(component, error),
        android_detection_summary(host).await,
        android_doctor_summary(host, scope).await,
        "Next steps: run `water doctor` for full diagnostics, then `water doctor --fix` to auto-install fixable dependencies.".to_string(),
    ]
    .join("\n")
}

/// Verify Xcode and the requested Apple SDK are installed.
///
/// # Errors
/// Returns an error describing any missing toolchain component and the `water doctor --fix` remedy.
pub async fn check_apple(host: &Host, sdk: AppleSdk) -> Result<()> {
    let xcode = Xcode;
    if let Err(e) = xcode.check(host).await {
        bail!("{}", toolchain_check_message("Xcode", &e));
    }
    if let Err(e) = sdk.check(host).await {
        bail!("{}", toolchain_check_message(&sdk.to_string(), &e));
    }
    Ok(())
}

/// Verify the Android toolchain covers building and packaging for all ABIs.
///
/// # Errors
/// Returns an error describing any missing toolchain component and the `water doctor --fix` remedy.
pub async fn check_android_build_or_package(host: &Host) -> Result<()> {
    check_android_build_or_package_for_abis(host, ALL_ABIS).await
}

/// Verify the Android toolchain covers building and packaging for `required_abis`.
///
/// # Errors
/// Returns an error describing any missing toolchain component and the `water doctor --fix` remedy.
pub async fn check_android_build_or_package_for_abis(
    host: &Host,
    required_abis: &[AndroidAbi],
) -> Result<()> {
    let sdk = AndroidSdk;
    if let Err(e) = sdk.check(host).await {
        bail!(
            "{}",
            android_failure_message(host, "Android SDK", &e, AndroidCheckScope::BuildOrPackage)
                .await
        );
    }
    let platforms = AndroidSdkPlatforms;
    if let Err(e) = platforms.check(host).await {
        bail!(
            "{}",
            android_failure_message(
                host,
                "Android SDK Platforms",
                &e,
                AndroidCheckScope::BuildOrPackage
            )
            .await
        );
    }
    let build_tools = AndroidBuildTools;
    if let Err(e) = build_tools.check(host).await {
        bail!(
            "{}",
            android_failure_message(
                host,
                "Android SDK Build-Tools (d8)",
                &e,
                AndroidCheckScope::BuildOrPackage
            )
            .await
        );
    }
    let ndk = AndroidNdk;
    if let Err(e) = ndk.check(host).await {
        bail!(
            "{}",
            android_failure_message(host, "Android NDK", &e, AndroidCheckScope::BuildOrPackage)
                .await
        );
    }
    let cmake = Cmake::default();
    if let Err(e) = cmake.check(host).await {
        bail!(
            "{}",
            android_failure_message(host, "Host CMake", &e, AndroidCheckScope::BuildOrPackage)
                .await
        );
    }
    let java = Java;
    if let Err(e) = java.check(host).await {
        bail!(
            "{}",
            android_failure_message(host, "Java", &e, AndroidCheckScope::BuildOrPackage).await
        );
    }
    let rust_targets = AndroidRustTargets::for_abis(required_abis);
    if let Err(e) = rust_targets.check(host).await {
        bail!(
            "{}",
            android_failure_message(
                host,
                "Android Rust Targets",
                &e,
                AndroidCheckScope::BuildOrPackage
            )
            .await
        );
    }
    let kotlin = Kotlin;
    if let Err(e) = kotlin.check(host).await {
        bail!(
            "{}",
            android_failure_message(host, "Kotlin", &e, AndroidCheckScope::BuildOrPackage).await
        );
    }
    Ok(())
}

/// Verify the Android toolchain covers running an app (adds `adb` to the build requirements).
///
/// # Errors
/// Returns an error describing any missing toolchain component and the `water doctor --fix` remedy.
pub async fn check_android_run(host: &Host) -> Result<()> {
    check_android_build_or_package(host).await?;
    let platform_tools = AndroidPlatformTools;
    if let Err(e) = platform_tools.check(host).await {
        bail!(
            "{}",
            android_failure_message(host, "Android Platform-Tools", &e, AndroidCheckScope::Run)
                .await
        );
    }
    Ok(())
}

/// Verify the GTK4 toolchain is installed.
///
/// # Errors
/// Returns an error describing any missing toolchain component and the `water doctor --fix` remedy.
pub async fn check_gtk4(host: &Host) -> Result<()> {
    let toolchain = Gtk4Toolchain;
    if let Err(e) = toolchain.check(host).await {
        bail!("{}", toolchain_check_message("GTK4", &e));
    }
    Ok(())
}

/// Verify the host toolchain components Hydrolysis builds need.
///
/// # Errors
/// Returns an error describing any missing toolchain component and the `water doctor --fix` remedy.
pub async fn check_hydrolysis(host: &Host) -> Result<()> {
    let llvm = WindowsArm64LlvmToolchain;
    if let Err(e) = llvm.check(host).await {
        bail!(
            "{}",
            toolchain_check_message("Windows ARM64 LLVM toolchain", &e)
        );
    }
    Ok(())
}

/// Verify the web toolchain is installed.
///
/// # Errors
/// Returns an error describing any missing toolchain component and the `water doctor --fix` remedy.
pub async fn check_web(host: &Host) -> Result<()> {
    let toolchain: WebToolchain = Default::default();
    if let Err(error) = toolchain.check(host).await {
        bail!(
            "Web toolchain check failed: {error}. Run `water doctor --fix` to install fixable components."
        );
    }
    Ok(())
}
