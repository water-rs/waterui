//! Light and dark appearance.
//!
//! Reading the scheme and observing its changes are platform-specific:
//! `appkit::Application::color_scheme` on macOS and
//! `uikit::ViewController::color_scheme` on iOS, each with an
//! `observe_color_scheme` next to it.

/// Whether the interface is drawn light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorScheme {
    /// Dark content on light backgrounds.
    Light,
    /// Light content on dark backgrounds.
    Dark,
}
