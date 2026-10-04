//! Apple deployment validation shared by the build script and its tests.

/// Require the native API floor for downstream builds as well as this workspace.
///
/// # Panics
/// Panics when `version` is empty, non-numeric, or older than macOS/iOS 26.
pub fn require_floor(version: &str) {
    let major: u32 = version
        .split('.')
        .next()
        .expect("deployment version")
        .parse()
        .expect("rustc reports a numeric deployment version");
    // Present in every profile, including release.
    assert!(
        major >= 26,
        "cherenkov-gpu requires macOS/iOS 26 or newer; resolved deployment target is {version}. Set MACOSX_DEPLOYMENT_TARGET or IPHONEOS_DEPLOYMENT_TARGET to 26.0 or newer."
    );
}

#[cfg(test)]
mod tests {
    #[test]
    #[should_panic(expected = "requires macOS/iOS 26 or newer")]
    fn rejects_pre_26_deployment() {
        super::require_floor("25.9");
    }

    #[test]
    fn accepts_the_declared_floor_and_newer() {
        for version in ["26", "26.0", "26.1.0", "27.0"] {
            super::require_floor(version);
        }
    }
}
