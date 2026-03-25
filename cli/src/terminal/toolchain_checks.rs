//! Shared toolchain checks for terminal commands.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail};

use waterui_cli::{
    android::{
        AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
        AndroidSdkPlatforms, Java, Kotlin,
    },
    apple::toolchain::{AppleSdk, Xcode},
    gtk4::toolchain::Gtk4Toolchain,
    toolchain::{
        Installation, Toolchain, ToolchainError,
        cmake::Cmake,
        doctor::{CheckStatus, doctor},
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

fn android_doctor_item_in_scope(name: &str, scope: AndroidCheckScope) -> bool {
    match name {
        "Android SDK"
        | "Android SDK Platforms"
        | "Android SDK Build-Tools (d8)"
        | "Android NDK"
        | "Android Rust Targets"
        | "Host CMake"
        | "Java"
        | "Kotlin" => true,
        "Android Platform-Tools (adb)" => scope == AndroidCheckScope::Run,
        _ => false,
    }
}

fn format_path_or_missing(label: &str, path: Option<&Path>) -> String {
    path.map_or_else(
        || format!("- {label}: <not detected>"),
        |path| format!("- {label}: {}", path.display()),
    )
}

async fn android_detection_summary() -> String {
    let sdk_root = AndroidSdk::detect_path();
    let d8_jar = AndroidSdk::d8_jar_path();
    let ndk_root = AndroidNdk::detect_path();
    let java_bin: Option<PathBuf> = Java::detect_path().await;
    let java_home: Option<PathBuf> = Java::detect_home().await;

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

async fn android_doctor_summary(scope: AndroidCheckScope) -> String {
    let mut lines = vec!["Relevant doctor diagnostics:".to_string()];

    for item in doctor().await {
        if item.status != CheckStatus::Missing || !android_doctor_item_in_scope(item.name, scope) {
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
    component: &str,
    error: &ToolchainError<I>,
    scope: AndroidCheckScope,
) -> String {
    [
        toolchain_check_message(component, error),
        android_detection_summary().await,
        android_doctor_summary(scope).await,
        "Next steps: run `water doctor` for full diagnostics, then `water doctor --fix` to auto-install fixable dependencies.".to_string(),
    ]
    .join("\n")
}

pub async fn check_apple(sdk: AppleSdk) -> Result<()> {
    let xcode = Xcode;
    if let Err(e) = xcode.check().await {
        bail!("{}", toolchain_check_message("Xcode", &e));
    }
    if let Err(e) = sdk.check().await {
        bail!("{}", toolchain_check_message(&sdk.to_string(), &e));
    }
    Ok(())
}

pub async fn check_android_build_or_package() -> Result<()> {
    let sdk = AndroidSdk;
    if let Err(e) = sdk.check().await {
        bail!(
            "{}",
            android_failure_message("Android SDK", &e, AndroidCheckScope::BuildOrPackage).await
        );
    }
    let platforms = AndroidSdkPlatforms;
    if let Err(e) = platforms.check().await {
        bail!(
            "{}",
            android_failure_message(
                "Android SDK Platforms",
                &e,
                AndroidCheckScope::BuildOrPackage
            )
            .await
        );
    }
    let build_tools = AndroidBuildTools;
    if let Err(e) = build_tools.check().await {
        bail!(
            "{}",
            android_failure_message(
                "Android SDK Build-Tools (d8)",
                &e,
                AndroidCheckScope::BuildOrPackage
            )
            .await
        );
    }
    let ndk = AndroidNdk;
    if let Err(e) = ndk.check().await {
        bail!(
            "{}",
            android_failure_message("Android NDK", &e, AndroidCheckScope::BuildOrPackage).await
        );
    }
    let cmake = Cmake::default();
    if let Err(e) = cmake.check().await {
        bail!(
            "{}",
            android_failure_message("Host CMake", &e, AndroidCheckScope::BuildOrPackage).await
        );
    }
    let java = Java;
    if let Err(e) = java.check().await {
        bail!(
            "{}",
            android_failure_message("Java", &e, AndroidCheckScope::BuildOrPackage).await
        );
    }
    let rust_targets = AndroidRustTargets;
    if let Err(e) = rust_targets.check().await {
        bail!(
            "{}",
            android_failure_message(
                "Android Rust Targets",
                &e,
                AndroidCheckScope::BuildOrPackage
            )
            .await
        );
    }
    let kotlin = Kotlin;
    if let Err(e) = kotlin.check().await {
        bail!(
            "{}",
            android_failure_message("Kotlin", &e, AndroidCheckScope::BuildOrPackage).await
        );
    }
    Ok(())
}

pub async fn check_android_run() -> Result<()> {
    check_android_build_or_package().await?;
    let platform_tools = AndroidPlatformTools;
    if let Err(e) = platform_tools.check().await {
        bail!(
            "{}",
            android_failure_message("Android Platform-Tools", &e, AndroidCheckScope::Run).await
        );
    }
    Ok(())
}

pub async fn check_gtk4() -> Result<()> {
    let toolchain = Gtk4Toolchain;
    if let Err(e) = toolchain.check().await {
        bail!("{}", toolchain_check_message("GTK4", &e));
    }
    Ok(())
}

pub async fn check_hydrolysis() -> Result<()> {
    let llvm = WindowsArm64LlvmToolchain;
    if let Err(e) = llvm.check().await {
        bail!(
            "{}",
            toolchain_check_message("Windows ARM64 LLVM toolchain", &e)
        );
    }
    Ok(())
}

pub async fn check_web() -> Result<()> {
    let toolchain: WebToolchain = Default::default();
    if let Err(error) = toolchain.check().await {
        bail!(
            "Web toolchain check failed: {error}. Run `water doctor --fix` to install fixable components."
        );
    }
    Ok(())
}
