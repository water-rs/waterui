//! The pinned Android NDK release and its runtime-Gradle parser.

/// NDK version the toolchain installs.
///
/// The floor the Hydrolysis Android host compiles against, embedded as a
/// literal so an installed CLI never has to locate any source checkout to
/// answer it.
pub const ANDROID_NDK_VERSION: &str = "29.0.14206865";

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
}
