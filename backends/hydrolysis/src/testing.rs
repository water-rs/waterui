//! Deterministic environment setup for Hydrolysis-backed tests.

use waterui::Environment;

/// Installs the framework default colour and font tokens required by
/// Hydrolysis rendering, underneath any entries the environment already
/// carries.
///
/// This goes through the same assembly the runtimes use
/// (`crate::theme::install_theme_tokens`) with no style. It is deliberately
/// *not* a [`crate::Style`]: it carries no widget theme, so tests that mount
/// widgets on the rendered runtime must still supply a real style.
pub fn install_theme(env: &mut Environment) {
    crate::theme::install_theme_tokens(env, None);
}
