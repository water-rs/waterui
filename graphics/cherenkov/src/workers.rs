//! Resolve the renderer's worker count once, before creating its pool.

use std::num::NonZeroUsize;

use crate::EngineError;

/// Resolve an explicit worker count or the host's available parallelism.
///
/// # Errors
/// Returns an error for invalid configuration or unavailable CPU topology.
pub fn count(explicit: Option<usize>) -> Result<NonZeroUsize, EngineError> {
    if let Some(count) = explicit {
        return positive(count, "renderer worker count");
    }
    match std::env::var("RAYON_NUM_THREADS") {
        Ok(value) => {
            let count = value.parse::<usize>().map_err(|error| {
                EngineError::Backend(format!(
                    "RAYON_NUM_THREADS must be a positive integer: {error}"
                ))
            })?;
            positive(count, "RAYON_NUM_THREADS")
        }
        Err(std::env::VarError::NotPresent) => automatic(),
        Err(std::env::VarError::NotUnicode(_)) => Err(EngineError::Backend(
            "RAYON_NUM_THREADS must be a Unicode positive integer".into(),
        )),
    }
}

fn positive(count: usize, source: &str) -> Result<NonZeroUsize, EngineError> {
    NonZeroUsize::new(count)
        .ok_or_else(|| EngineError::Backend(format!("{source} must be positive")))
}

#[cfg(target_arch = "wasm32")]
fn automatic() -> Result<NonZeroUsize, EngineError> {
    Err(EngineError::Backend(
        "automatic CPU worker discovery is unsupported on WebAssembly".into(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn automatic() -> Result<NonZeroUsize, EngineError> {
    std::thread::available_parallelism()
        .map_err(|error| EngineError::Backend(format!("available_parallelism: {error}")))
}

#[cfg(test)]
mod tests {
    use super::count;

    #[test]
    fn explicit_worker_count_is_positive() {
        assert!(count(Some(0)).is_err());
        assert_eq!(count(Some(8)).expect("explicit count").get(), 8);
    }
}
