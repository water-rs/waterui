//! Exports the facade's enabled features to its direct dependents.
//!
//! Cargo hands a `links` package's `cargo::metadata` to the build scripts of
//! the packages that depend on it directly, as `DEP_WATERUI_<KEY>`. A
//! backend reads `DEP_WATERUI_FEATURES` and gates each component it
//! realizes on the facade feature that provides the component, so it never
//! has to turn a facade feature on to compile.

use std::env;

fn main() {
    let mut features: Vec<String> = env::vars()
        .filter_map(|(key, _)| {
            key.strip_prefix("CARGO_FEATURE_")
                .map(|name| name.to_ascii_lowercase().replace('_', "-"))
        })
        .collect();
    features.sort_unstable();
    println!("cargo::metadata=features={}", features.join(","));
}
