//! Apple deployment validation shared by the build script and its tests.

/// Checked against the framework metadata by the deployment integration test.
pub const APPLE_DEPLOYMENT_FLOOR: u32 = 27;

/// Require the native API floor for downstream builds as well as this workspace.
///
/// # Panics
/// Panics when `version` is empty, non-numeric, or older than macOS/iOS 27.
pub fn require_floor(version: &str) {
    let major: u32 = version
        .split('.')
        .next()
        .expect("deployment version")
        .parse()
        .expect("rustc reports a numeric deployment version");
    // Present in every profile, including release.
    assert!(
        major >= APPLE_DEPLOYMENT_FLOOR,
        "cherenkov-gpu requires macOS/iOS {APPLE_DEPLOYMENT_FLOOR} or newer; resolved deployment target is {version}. Set MACOSX_DEPLOYMENT_TARGET or IPHONEOS_DEPLOYMENT_TARGET to {APPLE_DEPLOYMENT_FLOOR}.0 or newer."
    );
}

#[cfg(test)]
mod tests {
    #[test]
    #[should_panic(expected = "requires macOS/iOS 27 or newer")]
    fn rejects_pre_27_deployment() {
        super::require_floor("26.9");
    }

    #[test]
    fn accepts_the_declared_floor_and_newer() {
        for version in ["27", "27.0", "27.1.0", "28.0"] {
            super::require_floor(version);
        }
    }
}
