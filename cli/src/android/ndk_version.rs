//! The pinned Android NDK release and its runtime-Gradle parser.

/// NDK version the toolchain installs.
///
/// Mirrors the Android runtime's Gradle `ndkVersion`, embedded as a literal so
/// an installed CLI never has to locate the `WaterUI` source checkout to
/// answer it. `build.rs` asserts this literal still matches the runtime
/// Gradle file on every in-workspace build; update it together with
/// `backends/android/runtime/build.gradle.kts`.
pub const ANDROID_NDK_VERSION: &str = "29.0.14206865";

/// Workspace-relative path of the Android runtime manifest that declares
/// `ndkVersion`.
pub const RUNTIME_BUILD_GRADLE_RELATIVE_PATH: &str = "backends/android/runtime/build.gradle.kts";

/// Extracts the `ndkVersion = "…"` assignment from the Android runtime's
/// `build.gradle.kts`. Returns `None` when the declaration is absent or
/// malformed; comment-prefixed lines are ignored.
#[must_use]
pub fn parse_android_ndk_version_from_runtime_build_gradle(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.split("//").next()?.trim();
        let remainder = line.strip_prefix("ndkVersion")?;
        let (_, value) = remainder.split_once('=')?;
        let version = value.trim().trim_matches('"');
        if version.is_empty() {
            None
        } else {
            Some(version.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_android_ndk_version_from_runtime_build_gradle_extracts_declared_version() {
        let contents = r#"
android {
    compileSdk = 37
    ndkVersion = "29.0.14206865"
}
"#;
        assert_eq!(
            parse_android_ndk_version_from_runtime_build_gradle(contents),
            Some("29.0.14206865".to_string())
        );
    }

    #[test]
    fn embedded_ndk_version_matches_runtime_gradle_declaration() {
        // Only meaningful inside a WaterUI checkout; a packaged `.crate` test
        // run has no workspace tree to compare against.
        let gradle = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(RUNTIME_BUILD_GRADLE_RELATIVE_PATH);
        let Ok(contents) = std::fs::read_to_string(&gradle) else {
            return;
        };
        assert_eq!(
            parse_android_ndk_version_from_runtime_build_gradle(&contents).as_deref(),
            Some(ANDROID_NDK_VERSION),
            "ANDROID_NDK_VERSION drifted from the runtime Gradle `ndkVersion`",
        );
    }
}
