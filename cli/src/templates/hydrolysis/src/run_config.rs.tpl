//! Shared run-config loading for the generated Hydrolysis runtime modes.

use std::path::PathBuf;

/// Reads and parses the JSON run configuration whose path `env_var` carries.
///
/// `what` names the runtime mode in panic messages (for example `preview`).
pub(crate) fn load_run_config<T: serde::de::DeserializeOwned>(env_var: &str, what: &str) -> T {
    let path = std::env::var_os(env_var).unwrap_or_else(|| {
        panic!("hydrolysis {what}: missing environment variable `{env_var}`")
    });
    let raw = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "hydrolysis {what}: failed to read run config `{}`: {error}",
            PathBuf::from(&path).display()
        )
    });
    serde_json::from_slice(&raw).unwrap_or_else(|error| {
        panic!(
            "hydrolysis {what}: failed to parse run config `{}`: {error}",
            PathBuf::from(&path).display()
        )
    })
}
