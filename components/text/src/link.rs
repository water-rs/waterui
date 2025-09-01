use waterui_core::Str;
use waterui_core::{AnyView, configurable};
use nami::Computed;

#[derive(Debug)]
/// Configuration for the `Link` component.
pub struct LinkConfig {
    /// The label of the link.
    pub label: AnyView,
    /// The URL the link points to.
    pub url: Computed<Str>,
}

configurable!(Link, LinkConfig);
