//! Shared toolchain checks for terminal commands.

use color_eyre::eyre::{Result, bail};

use waterui_cli::{
    android::toolchain::{AndroidNdk, AndroidPlatformTools, AndroidSdk, Java},
    apple::toolchain::{AppleSdk, Xcode},
    gtk4::toolchain::Gtk4Toolchain,
    toolchain::{Installation, Toolchain, ToolchainError, cmake::Cmake},
};

fn toolchain_check_message<I: Installation>(component: &str, error: ToolchainError<I>) -> String {
    match error {
        ToolchainError::Fixable(_) => format!(
            "{component} toolchain check failed: missing dependencies can be fixed automatically with `water doctor --fix`."
        ),
        ToolchainError::Unfixable(unfixable) => {
            format!("{component} toolchain check failed: {unfixable}")
        }
    }
}

pub async fn check_apple(sdk: AppleSdk) -> Result<()> {
    let xcode = Xcode;
    if let Err(e) = xcode.check().await {
        bail!("{}", toolchain_check_message("Xcode", e));
    }
    if let Err(e) = sdk.check().await {
        bail!("{}", toolchain_check_message(&sdk.to_string(), e));
    }
    Ok(())
}

pub async fn check_android_build_or_package() -> Result<()> {
    let sdk = AndroidSdk;
    if let Err(e) = sdk.check().await {
        bail!("{}", toolchain_check_message("Android SDK", e));
    }
    let ndk = AndroidNdk;
    if let Err(e) = ndk.check().await {
        bail!("{}", toolchain_check_message("Android NDK", e));
    }
    let cmake = Cmake::default();
    if let Err(e) = cmake.check().await {
        bail!("{}", toolchain_check_message("Host CMake", e));
    }
    let java = Java;
    if let Err(e) = java.check().await {
        bail!("{}", toolchain_check_message("Java", e));
    }
    Ok(())
}

pub async fn check_android_run() -> Result<()> {
    check_android_build_or_package().await?;
    let platform_tools = AndroidPlatformTools;
    if let Err(e) = platform_tools.check().await {
        bail!("{}", toolchain_check_message("Android Platform-Tools", e));
    }
    Ok(())
}

pub async fn check_gtk4() -> Result<()> {
    let toolchain = Gtk4Toolchain;
    if let Err(e) = toolchain.check().await {
        bail!("{}", toolchain_check_message("GTK4", e));
    }
    Ok(())
}
