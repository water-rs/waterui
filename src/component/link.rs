//! Link component for `WaterUI`
//!
//! This module provides a Link component that displays clickable text that opens a URL.
//! Link is implemented as a Button with link style, using `waterkit-deeplink` to open URLs.

use executor_core::spawn_local;
use nami::Computed;
use nami::Signal;
use nami::signal::IntoComputed;
use waterui_controls::{
    IntoLabel,
    button::{ButtonStyle, button},
};
use waterui_core::env::with;
use waterui_core::{Environment, View};
use waterui_url::Url;

/// Opens a URL in the system's default browser/handler.
pub(crate) fn open_url(url: &str) {
    let url = url.to_owned();
    spawn_local(async move {
        if let Err(e) = waterkit_deeplink::open_url(&url).await {
            tracing::error!("Failed to open URL '{}': {:?}", url, e);
        }
    })
    .detach();
}

/// A tappable text link that opens a URL.
///
/// Link displays styled text that navigates to the specified URL when tapped.
/// Internally, Link uses a Button with `ButtonStyle::Link` styling.
///
/// # Layout Behavior
///
/// Link sizes itself to fit its label content and never stretches to fill extra space.
/// In a stack, it takes only the space it needs, just like Text.
///
/// # Examples
///
/// ```
/// use waterui::prelude::*;
///
/// // Create a simple link
/// let my_link = link("Visit website", "https://waterui.dev");
/// ```
#[derive(Debug)]
pub struct Link<Label> {
    label: Label,
    url: Computed<Url>,
}

impl<Label> Link<Label>
where
    Label: IntoLabel + 'static,
{
    /// Creates a new `Link` view with the specified label and URL.
    pub fn new(label: Label, url: impl IntoComputed<Url>) -> Self {
        Self {
            label,
            url: url.into_computed(),
        }
    }
}

/// The URL a [`link`] points at, published into the subtree's environment.
///
/// Backends read it to surface the target natively — an OSC 8 hyperlink
/// escape on terminal backends, a status-bar preview, an accessibility
/// description — without intercepting the button's action.
#[derive(Debug, Clone)]
pub struct LinkTarget(pub Computed<Url>);

impl<Label> View for Link<Label>
where
    Label: IntoLabel + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let url = self.url;
        let target = url.clone();

        with(
            button(self.label).style(ButtonStyle::Link).action(move || {
                let target_url = url.snapshot();
                open_url(target_url.as_str());
            }),
            LinkTarget(target),
        )
    }
}

/// Convenience constructor for building a `Link` view inline.
///
/// # Arguments
///
/// * `label` - The text or view to display as the link
/// * `url` - The URL to navigate to when the link is tapped
pub fn link<Label>(label: Label, url: impl IntoComputed<Url>) -> Link<Label>
where
    Label: IntoLabel + 'static,
{
    Link::new(label, url)
}

#[cfg(test)]
mod tests {
    use nami::{Binding, Signal, SignalExt as _};
    use waterui_url::Url;

    use super::{Link, link};

    #[test]
    fn link_accepts_literal_url_binding_and_computed() {
        let _ = Link::new("Static", "https://waterui.dev");
        let parsed = Url::new("https://waterui.dev/docs");
        let _ = link("Url", parsed.clone());

        let url_binding = Binding::container(parsed);
        let bound_link = Link::new("Binding", url_binding.clone());
        assert_eq!(
            bound_link.url.snapshot().as_str(),
            "https://waterui.dev/docs"
        );

        url_binding.set(Url::new("https://waterui.dev/api"));
        assert_eq!(
            bound_link.url.snapshot().as_str(),
            "https://waterui.dev/api"
        );

        let computed_link = link("Computed", url_binding.map(|u| u).computed());
        assert_eq!(
            computed_link.url.snapshot().as_str(),
            "https://waterui.dev/api"
        );
    }
}
