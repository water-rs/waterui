//! The pinned Android NDK release and its runtime-Gradle parser.

/// NDK version the toolchain installs.
///
/// Mirrors the Android runtime's Gradle `ndkVersion`, embedded as a literal so
/// an installed CLI never has to locate any source checkout to answer it. The
/// runtime lives in `water-rs/android-backend`, pinned as the
/// `backends/android` gitlink of the `water-rs/waterui` revision this crate's
/// manifest builds against;
/// `embedded_ndk_version_matches_the_pinned_android_backend` asserts the two
/// stay in lockstep.
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

    /// `ANDROID_NDK_VERSION` must not drift from the runtime Gradle
    /// declaration. The runtime is the `android-backend-revision` the
    /// enclosing workspace's root manifest declares, so the comparison
    /// resolves that commit and fetches `runtime/build.gradle.kts` there —
    /// network-bound, hence nightly-only.
    #[test]
    #[ignore = "fetches the pinned water-rs/android-backend revision over the network"]
    fn embedded_ndk_version_matches_the_pinned_android_backend() {
        let (backend_commit, backend) = crate::pinned_framework::android_backend();
        let contents = String::from_utf8(crate::pinned_framework::fetch(
            &crate::pinned_framework::raw_url(
                &backend,
                &backend_commit,
                "runtime/build.gradle.kts",
            ),
        ))
        .expect("runtime build.gradle.kts is UTF-8");
        assert_eq!(
            parse_android_ndk_version_from_runtime_build_gradle(&contents).as_deref(),
            Some(ANDROID_NDK_VERSION),
            "ANDROID_NDK_VERSION drifted from the pinned android-backend's \
             runtime `ndkVersion` ({backend}@{backend_commit})"
        );
    }
}
