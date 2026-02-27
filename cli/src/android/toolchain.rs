use std::{env, path::PathBuf};

use crate::{
    toolchain::{Installation, Toolchain, cmake::Cmake},
    utils::which,
};

/// Complete Android toolchain including SDK, NDK, Kotlin, and `CMake`.
pub type AndroidToolchain = (AndroidSdk, AndroidNdk, Kotlin, Cmake);

/// Android SDK toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidSdk;

impl AndroidSdk {
    /// Detect the path to the Android SDK installation.
    #[must_use]
    pub fn detect_path() -> Option<PathBuf> {
        // Environment variables (prefer the modern variable first).
        for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
            if let Ok(raw) = env::var(key) {
                let sdk_path = PathBuf::from(raw);
                if sdk_path.exists() {
                    return Some(sdk_path);
                }
            }
        }

        if cfg!(target_os = "macos") {
            let home_sdk = PathBuf::from(env::var("HOME").ok()?).join("Library/Android/sdk");
            if home_sdk.exists() {
                return Some(home_sdk);
            }
        }

        if cfg!(target_os = "linux") {
            let home_sdk = PathBuf::from(env::var("HOME").ok()?).join("Android/Sdk");
            if home_sdk.exists() {
                return Some(home_sdk);
            }
        }

        if cfg!(target_os = "windows") {
            if let Ok(localappdata) = env::var("LOCALAPPDATA") {
                let sdk_path = PathBuf::from(localappdata).join("Android/Sdk");
                if sdk_path.exists() {
                    return Some(sdk_path);
                }
            }
        }

        None
    }

    /// Get the path to the `adb` executable.
    #[must_use]
    pub fn adb_path() -> Option<PathBuf> {
        let sdk_path = Self::detect_path()?;
        let adb = sdk_path
            .join("platform-tools")
            .join(if cfg!(target_os = "windows") {
                "adb.exe"
            } else {
                "adb"
            });
        if adb.exists() { Some(adb) } else { None }
    }

    /// Get the path to the `emulator` executable.
    #[must_use]
    pub fn emulator_path() -> Option<PathBuf> {
        let sdk_path = Self::detect_path()?;
        let emulator = sdk_path
            .join("emulator")
            .join(if cfg!(target_os = "windows") {
                "emulator.exe"
            } else {
                "emulator"
            });
        if emulator.exists() {
            Some(emulator)
        } else {
            None
        }
    }
}

/// Installation procedure for the Android SDK.
#[derive(Debug, Clone, Default)]
pub struct AndroidSdkInstallation;

/// An Android NDK toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidNdk;

/// Java toolchain component for Android development.
#[derive(Debug, Clone, Default)]
pub struct Java;

impl Java {
    /// Detect the path to the Java installation for Android development.
    ///
    /// Priority order:
    /// 1. Android Studio's bundled JBR (guaranteed compatible with AGP)
    /// 2. JAVA_HOME environment variable (may be incompatible)
    /// 3. Java from PATH (fallback)
    ///
    /// We prioritize Android Studio's JBR because system Java (e.g., Homebrew's
    /// JDK 25) may be incompatible with the Android Gradle Plugin.
    pub async fn detect_path() -> Option<PathBuf> {
        // Check Android Studio's bundled JBR first (guaranteed compatible)
        if cfg!(target_os = "macos") {
            const ANDROID_STUDIO_JBRS: &[&str] = &[
                "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java",
                "/Applications/Android Studio Preview.app/Contents/jbr/Contents/Home/bin/java",
            ];
            for path in ANDROID_STUDIO_JBRS {
                let java_path = PathBuf::from(path);
                if java_path.exists() {
                    return Some(java_path);
                }
            }
        }

        // Check Android Studio on Linux
        if cfg!(target_os = "linux") {
            if let Ok(home) = env::var("HOME") {
                let paths = [
                    format!(
                        "{home}/.local/share/JetBrains/Toolbox/apps/android-studio/jbr/bin/java"
                    ),
                    format!("{home}/android-studio/jbr/bin/java"),
                ];
                for path in paths {
                    let java_path = PathBuf::from(&path);
                    if java_path.exists() {
                        return Some(java_path);
                    }
                }
            }
        }

        // Check Android Studio on Windows
        if cfg!(target_os = "windows") {
            if let Ok(program_files) = env::var("ProgramFiles") {
                let java_path =
                    PathBuf::from(&program_files).join("Android/Android Studio/jbr/bin/java.exe");
                if java_path.exists() {
                    return Some(java_path);
                }
            }
        }

        // Fall back to JAVA_HOME (may be incompatible with AGP)
        if let Ok(home) = env::var("JAVA_HOME") {
            let java_path = PathBuf::from(home).join("bin/java");
            if java_path.exists() {
                return Some(java_path);
            }
        }

        // Check PATH as last resort
        which("java").await.ok()
    }

    /// Get the JAVA_HOME directory (parent of bin/).
    pub async fn detect_home() -> Option<PathBuf> {
        let java_path = Self::detect_path().await?;
        // java_path is .../bin/java, we want ...
        java_path.parent()?.parent().map(PathBuf::from)
    }
}

/// Kotlin toolchain component for Android development.
#[derive(Debug, Clone, Default)]
pub struct Kotlin;

impl Kotlin {
    /// Detect the path to the kotlinc compiler.
    pub async fn detect_path() -> Option<PathBuf> {
        // Check KOTLIN_HOME first
        if let Ok(home) = env::var("KOTLIN_HOME") {
            let kotlinc_path = PathBuf::from(&home).join("bin/kotlinc");
            if kotlinc_path.exists() {
                return Some(kotlinc_path);
            }
        }

        // Check PATH first (prefers system-installed Kotlin from Homebrew, etc.)
        if let Ok(path) = which("kotlinc").await {
            return Some(path);
        }

        // Fall back to Android Studio's bundled Kotlin on macOS
        if cfg!(target_os = "macos") {
            const ANDROID_STUDIO_KOTLINS: &[&str] = &[
                "/Applications/Android Studio.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
                "/Applications/Android Studio Preview.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
            ];
            for path in ANDROID_STUDIO_KOTLINS {
                let kotlinc_path = PathBuf::from(path);
                if kotlinc_path.exists() {
                    return Some(kotlinc_path);
                }
            }
        }

        // Fall back to Android Studio on Linux
        if cfg!(target_os = "linux") {
            if let Ok(home) = env::var("HOME") {
                let paths = [
                    format!(
                        "{home}/.local/share/JetBrains/Toolbox/apps/android-studio/plugins/Kotlin/kotlinc/bin/kotlinc"
                    ),
                    format!("{home}/android-studio/plugins/Kotlin/kotlinc/bin/kotlinc"),
                ];
                for path in paths {
                    let kotlinc_path = PathBuf::from(&path);
                    if kotlinc_path.exists() {
                        return Some(kotlinc_path);
                    }
                }
            }
        }

        // Fall back to Android Studio on Windows
        if cfg!(target_os = "windows") {
            if let Ok(program_files) = env::var("ProgramFiles") {
                let kotlinc_path = PathBuf::from(&program_files)
                    .join("Android/Android Studio/plugins/Kotlin/kotlinc/bin/kotlinc.bat");
                if kotlinc_path.exists() {
                    return Some(kotlinc_path);
                }
            }
        }

        None
    }

    /// Get the KOTLIN_HOME directory (parent of bin/).
    pub async fn detect_home() -> Option<PathBuf> {
        let kotlinc_path = Self::detect_path().await?;
        // kotlinc_path is .../bin/kotlinc, we want ...
        kotlinc_path.parent()?.parent().map(PathBuf::from)
    }
}

/// Kotlin installation handler.
#[derive(Debug)]
pub struct KotlinInstallation;

/// Errors that can occur when installing Kotlin.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallKotlin {}

impl Toolchain for Kotlin {
    type Installation = KotlinInstallation;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        use crate::toolchain::ToolchainError;

        let kotlinc_path = Self::detect_path().await.ok_or_else(|| {
            ToolchainError::unfixable(
                "Kotlin compiler (kotlinc) not found",
                "Install Android Studio from https://developer.android.com/studio \
                 (includes Kotlin), or install Kotlin separately via 'brew install kotlin' \
                 or set KOTLIN_HOME environment variable.",
            )
        })?;

        // Check if kotlinc is executable (common issue on macOS with quarantined apps)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = smol::unblock({
                let kotlinc_path = kotlinc_path.clone();
                move || std::fs::metadata(&kotlinc_path)
            })
            .await
            {
                let permissions = metadata.permissions();
                if permissions.mode() & 0o111 == 0 {
                    return Err(ToolchainError::unfixable(
                        "Kotlin compiler (kotlinc) is not executable",
                        &format!(
                            "The kotlinc script at '{}' doesn't have execute permission. \
                             Fix it with: sudo chmod +x '{}'\n\
                             Alternatively, install Kotlin via Homebrew: brew install kotlin",
                            kotlinc_path.display(),
                            kotlinc_path.display()
                        ),
                    ));
                }
            }
        }

        Ok(())
    }
}

impl Installation for KotlinInstallation {
    type Error = FailToInstallKotlin;

    async fn install(&self) -> Result<(), Self::Error> {
        // Kotlin installation is typically bundled with Android Studio
        // or installed via package managers. This is intentionally a no-op.
        Ok(())
    }
}

impl AndroidNdk {
    /// Detect the Android NDK path from environment variables or standard locations.
    #[must_use]
    pub fn detect_path() -> Option<PathBuf> {
        // Check ANDROID_NDK_ROOT environment variable
        if let Ok(ndk_root) = env::var("ANDROID_NDK_ROOT") {
            let ndk_path = PathBuf::from(ndk_root);
            if ndk_path.exists() {
                return Some(ndk_path);
            }
        }

        // Check ANDROID_NDK_HOME environment variable
        if let Ok(ndk_home) = env::var("ANDROID_NDK_HOME") {
            let ndk_path = PathBuf::from(ndk_home);
            if ndk_path.exists() {
                return Some(ndk_path);
            }
        }

        // Check in the Android SDK path
        let sdk_path = AndroidSdk::detect_path()?;

        let ndk_dir = sdk_path.join("ndk");
        if ndk_dir.exists() {
            // Find the latest NDK version
            if let Ok(entries) = std::fs::read_dir(&ndk_dir) {
                let mut versions: Vec<PathBuf> = entries
                    .filter_map(std::result::Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect();

                versions.sort();
                if let Some(latest) = versions.last() {
                    return Some(latest.clone());
                }
            }
        }

        None
    }
}

/// Errors that can occur when installing the Android SDK.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidSdk {}

impl Toolchain for AndroidSdk {
    type Installation = AndroidSdkInstallation;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        use crate::toolchain::ToolchainError;

        if Self::detect_path().is_none() {
            return Err(ToolchainError::unfixable(
                "Android SDK not found",
                "Install Android Studio from https://developer.android.com/studio \
                 or set ANDROID_HOME environment variable to your SDK path.",
            ));
        }

        // Check for adb executable
        if Self::adb_path().is_none() {
            return Err(ToolchainError::unfixable(
                "Android SDK platform-tools not found",
                "Open Android Studio -> SDK Manager -> SDK Tools -> check 'Android SDK Platform-Tools'",
            ));
        }

        Ok(())
    }
}

impl Installation for AndroidSdkInstallation {
    type Error = FailToInstallAndroidSdk;

    async fn install(&self) -> Result<(), Self::Error> {
        // Android SDK installation is complex and platform-specific
        // We guide the user to install it manually via Android Studio
        // This is intentionally a no-op as the check returns unfixable errors
        Ok(())
    }
}

/// Android NDK installation handler.
#[derive(Debug)]
pub struct AndroidNdkInstallation;

impl Toolchain for AndroidNdk {
    type Installation = AndroidNdkInstallation;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        use crate::toolchain::ToolchainError;

        let ndk_path = Self::detect_path().ok_or_else(|| {
            ToolchainError::unfixable(
                "Android NDK not found",
                "Open Android Studio -> SDK Manager -> SDK Tools -> check 'NDK (Side by side)' \
                 or set ANDROID_NDK_ROOT environment variable.",
            )
        })?;

        // Check for LLVM toolchain (required for building)
        let llvm_dir = ndk_path.join("toolchains/llvm/prebuilt");
        if !llvm_dir.exists() {
            return Err(ToolchainError::unfixable(
                "Android NDK LLVM toolchain not found",
                "The NDK installation appears to be incomplete. \
                 Try reinstalling the NDK via Android Studio SDK Manager.",
            ));
        }

        Ok(())
    }
}

impl Installation for AndroidNdkInstallation {
    type Error = FailToInstallAndroidNdk;

    async fn install(&self) -> Result<(), Self::Error> {
        // NDK installation is handled via Android Studio SDK Manager
        // This is intentionally a no-op as the check returns unfixable errors
        Ok(())
    }
}

/// Errors that can occur when installing the Android NDK.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidNdk {}
