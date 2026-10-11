//! The build script's Apple floor is enforced independently of Cargo config.

#[path = "../deployment.rs"]
mod deployment;

#[test]
fn framework_metadata_and_cargo_config_agree_with_enforced_floor() {
    let manifest: toml::Value = toml::from_str(include_str!("../../../../Cargo.toml")).unwrap();
    let config: toml::Value =
        toml::from_str(include_str!("../../../../.cargo/config.toml")).unwrap();
    let targets = &manifest["package"]["metadata"]["waterui"]["apple-deployment-targets"];
    let floor = format!("{}.0", deployment::APPLE_DEPLOYMENT_FLOOR);
    for (platform, variable) in [
        ("macos", "MACOSX_DEPLOYMENT_TARGET"),
        ("ios", "IPHONEOS_DEPLOYMENT_TARGET"),
    ] {
        assert_eq!(targets[platform].as_str(), Some(floor.as_str()));
        assert_eq!(config["env"][variable], targets[platform]);
        deployment::require_floor(targets[platform].as_str().unwrap());
    }
}
