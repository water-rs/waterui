//! The log-level contract between the `water` CLI and an application.
//!
//! `water run --logs <level>` hands the level to the app through
//! [`LOG_LEVEL_ENV`]; every backend that installs a `tracing` subscriber on a
//! host with a process environment builds its filter with [`env_filter`], so
//! the variable means the same thing everywhere.

use tracing_subscriber::EnvFilter;
use waterui_core::contract_violation;

/// The environment variable a launcher sets to name the level the application
/// logs at — `water run --logs <level>` writes it.
pub const LOG_LEVEL_ENV: &str = "WATERUI_LOG";

/// The graphics stack's directives: it stays at `error` whatever level the
/// application asks for, since its `debug` output drowns the application's.
const GRAPHICS_DIRECTIVES: &str = "wgpu_core=error,wgpu_hal=error,naga=error,metal=error";

/// Whether the environment asks for logs: `RUST_LOG` or [`LOG_LEVEL_ENV`] is
/// set.
#[must_use]
pub fn log_requested() -> bool {
    std::env::var_os("RUST_LOG").is_some() || std::env::var_os(LOG_LEVEL_ENV).is_some()
}

/// The `tracing` filter this process runs with: `RUST_LOG` wins outright,
/// otherwise [`LOG_LEVEL_ENV`] names the level (`error` when unset) and the
/// graphics stack stays at `error`.
///
/// # Contract
/// A [`LOG_LEVEL_ENV`] that is not a valid `tracing` level breaks the
/// launcher's contract: a debug build panics, and a release build runs at the
/// `error` level.
#[must_use]
pub fn env_filter() -> EnvFilter {
    filter(
        std::env::var("RUST_LOG").ok().as_deref(),
        std::env::var(LOG_LEVEL_ENV).ok().as_deref(),
    )
}

fn filter(rust_log: Option<&str>, level: Option<&str>) -> EnvFilter {
    if let Some(filter) = rust_log.and_then(|directives| EnvFilter::try_new(directives).ok()) {
        return filter;
    }
    let level = level.unwrap_or("error");
    EnvFilter::try_new(format!("{level},{GRAPHICS_DIRECTIVES}")).unwrap_or_else(|error| {
        contract_violation!("{LOG_LEVEL_ENV}={level:?} is not a tracing level: {error}");
        error_filter()
    })
}

/// The filter of the `error` level, with the graphics stack at `error`.
fn error_filter() -> EnvFilter {
    EnvFilter::new(format!("error,{GRAPHICS_DIRECTIVES}"))
}

#[cfg(test)]
mod tests {
    use super::filter;

    #[test]
    fn level_applies_to_the_app_and_keeps_graphics_at_error() {
        let filter = filter(None, Some("debug")).to_string();
        assert!(filter.contains("debug"), "{filter}");
        for target in ["wgpu_core", "wgpu_hal", "naga", "metal"] {
            assert!(filter.contains(&format!("{target}=error")), "{filter}");
        }
    }

    #[test]
    fn rust_log_wins_outright() {
        let filter = filter(Some("waterui=trace"), Some("debug")).to_string();
        assert!(filter.contains("waterui=trace"), "{filter}");
        assert!(!filter.contains("wgpu_core"), "{filter}");
    }

    #[test]
    fn unset_level_is_error() {
        let filter = filter(None, None).to_string();
        assert!(filter.contains("error"), "{filter}");
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "is not a tracing level"))]
    fn invalid_level_runs_at_error() {
        let filter = filter(None, Some("loud==")).to_string();
        assert_eq!(filter, super::error_filter().to_string());
    }
}
