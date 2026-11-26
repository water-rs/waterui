//! Exports the framework's enabled features to direct dependents.
//!
//! Cargo hands a `links` package's `cargo::metadata` to the build scripts of
//! the packages that depend on it directly, as `DEP_WATERUI_<KEY>`. Backends
//! and style crates read `DEP_WATERUI_FEATURES` and gate each component they
//! realize on the feature that provides it, without depending on the top-level
//! `waterui` facade.

use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let mut features: Vec<String> = env::vars()
        .filter_map(|(key, _)| {
            key.strip_prefix("CARGO_FEATURE_")
                .map(|name| name.to_ascii_lowercase().replace('_', "-"))
        })
        .collect();
    features.sort_unstable();
    println!("cargo::metadata=features={}", features.join(","));
}
