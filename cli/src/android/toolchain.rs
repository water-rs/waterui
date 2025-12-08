use std::{env, path::PathBuf};

use crate::{
    toolchain::{Installation, Toolchain, cmake::Cmake},
    utils::which,
};

pub type AndroidToolchain = (AndroidSdk, AndroidNdk, Cmake);

#[derive(Debug, Clone, Default)]
pub struct AndroidSdk;

impl AndroidSdk {
    #[must_use]
    pub fn detect_path() -> Option<PathBuf> {
        // Check ANDROID_HOME environment variable
        if let Ok(android_home) = env::var("ANDROID_HOME") {
            let sdk_path = PathBuf::from(android_home);
            if sdk_path.exists() {
                return Some(sdk_path);
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
}

pub struct AndroidSdkInstallation;

#[derive(Debug, Clone, Default)]
pub struct AndroidNdk;
pub struct Java;

impl Java {
    pub async fn detect_path() -> Option<PathBuf> {
        // Check JAVA_HOME first
        if let Ok(home) = env::var("JAVA_HOME") {
            let java_path = PathBuf::from(home).join("bin/java");
            if java_path.exists() {
                return Some(java_path);
            }
        }

        // Check Android Studio's bundled JBR on macOS
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

        // Check PATH
        which("java").await.ok()
    }
}

impl AndroidNdk {
    async fn detect_path() -> Option<PathBuf> {
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

#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidSdk {}

impl Toolchain for AndroidSdk {
    type Installation = AndroidSdkInstallation;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        todo!()
    }
}

impl Installation for AndroidSdkInstallation {
    type Error = FailToInstallAndroidSdk;

    async fn install(&self) -> Result<(), Self::Error> {
        todo!()
    }
}

pub struct AndroidNdkInstallation {}

impl Toolchain for AndroidNdk {
    type Installation = AndroidNdkInstallation;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        todo!()
    }
}

impl Installation for AndroidNdkInstallation {
    type Error = FailToInstallAndroidNdk;

    async fn install(&self) -> Result<(), Self::Error> {
        todo!()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidNdk {}
