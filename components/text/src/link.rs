use nami::Computed;
use nami::signal::IntoComputed;
use waterui_core::{AnyView, configurable};
use waterui_core::{Str, View};

#[derive(Debug)]
/// Configuration for the `Link` component.
pub struct LinkConfig {
    /// The label of the link.
    pub label: AnyView,
    /// The URL the link points to.
    pub url: Computed<Str>,
}

configurable!(
    /// A hyperlink component that navigates to a specified URL when clicked.
    Link, LinkConfig);

impl Link {
    /// Creates a new link component.
    pub fn new(label: impl View, url: impl IntoComputed<Str>) -> Self {
        Self(LinkConfig {
            label: AnyView::new(label),
            url: url.into_computed(),
        })
    }
}

/// Convenience constructor for building a `Link` view inline.
pub fn link(label: impl View, url: impl IntoComputed<Str>) -> Link {
    Link::new(label, url)
}
