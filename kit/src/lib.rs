use core_foundation::bundle::CFBundle;
use std::path::PathBuf;

pub fn bundle_path() -> PathBuf {
    let bundle = CFBundle::main_bundle();

    bundle.bundle_resources_url().unwrap().to_path().unwrap()
}

pub fn assets_path() -> PathBuf {
    bundle_path().join("assets")
}
