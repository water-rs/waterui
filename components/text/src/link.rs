use nami::signal::IntoComputed;
use nami::Computed;
use waterui_core::{Str, View};
use waterui_core::{AnyView, configurable};

#[derive(Debug)]
/// Configuration for the `Link` component.
pub struct LinkConfig {
    /// The label of the link.
    pub label: AnyView,
    /// The URL the link points to.
    pub url: Computed<Str>,
}

configurable!(Link, LinkConfig);

impl Link{
    /// Creates a new link component.
    pub fn new(label: impl View, url: impl IntoComputed<Str>) -> Self {
        Self(LinkConfig {
            label: AnyView::new(label),
            url: url.into_computed(),
        })
    }
}

pub fn link(label: impl View, url: impl IntoComputed<Str>) -> Link {
    Link::new(label, url)
}