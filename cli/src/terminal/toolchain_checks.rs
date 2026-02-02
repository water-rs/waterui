//! Shared toolchain checks for terminal commands.

use color_eyre::eyre::{Result, bail};

use waterui_cli::{
    android::toolchain::{AndroidNdk, AndroidSdk},
    apple::toolchain::{AppleSdk, Xcode},
    gtk4::toolchain::Gtk4Toolchain,
    toolchain::{Toolchain, cmake::Cmake},
};

pub async fn check_apple(sdk: AppleSdk) -> Result<()> {
    let xcode = Xcode;
    if let Err(e) = xcode.check().await {
        bail!("Xcode toolchain check failed: {e}");
    }
    if let Err(e) = sdk.check().await {
        bail!("{sdk} toolchain check failed: {e}");
    }
    Ok(())
}

pub async fn check_android() -> Result<()> {
    let sdk = AndroidSdk;
    if let Err(e) = sdk.check().await {
        bail!("Android SDK toolchain check failed: {e}");
    }
    let ndk = AndroidNdk;
    if let Err(e) = ndk.check().await {
        bail!("Android NDK toolchain check failed: {e}");
    }
    let cmake = Cmake::default();
    if let Err(e) = cmake.check().await {
        bail!("CMake toolchain check failed: {e}");
    }
    Ok(())
}

pub async fn check_gtk4() -> Result<()> {
    let toolchain = Gtk4Toolchain;
    if let Err(e) = toolchain.check().await {
        bail!("GTK4 toolchain check failed: {e}");
    }
    Ok(())
}
