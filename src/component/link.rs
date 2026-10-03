//! Link component for `WaterUI`
//!
//! This module provides a Link component that displays clickable text that opens a URL.
//! Link is implemented as a Button with link style, using `robius-open` to open URLs.

use nami::Computed;
use nami::Signal;
use nami::signal::IntoComputed;
use waterui_controls::{
    IntoLabel,
    button::{ButtonStyle, button},
};
use waterui_core::env::with;
use waterui_core::{Environment, Str, View};

/// Opens a URL in the system's default browser/handler.
#[cfg(not(target_os = "espidf"))]
fn open_url(url: &str) {
    if let Err(e) = robius_open::Uri::new(url).open() {
        tracing::error!("Failed to open URL '{}': {:?}", url, e);
    }
}

/// Embedded targets have no default URL handler; opening is a no-op.
#[cfg(target_os = "espidf")]
fn open_url(_url: &str) {}

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
    url: Computed<Str>,
}

impl<Label> Link<Label>
where
    Label: IntoLabel + 'static,
{
    /// Creates a new `Link` view with the specified label and URL.
    pub fn new(label: Label, url: impl IntoComputed<Str>) -> Self {
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
pub struct LinkTarget(pub Computed<Str>);

impl<Label> View for Link<Label>
where
    Label: IntoLabel + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let url = self.url;
        let target = url.clone();

        with(
            button(self.label).style(ButtonStyle::Link).action(move || {
                let url_str = url.snapshot();
                open_url(&url_str);
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
pub fn link<Label>(label: Label, url: impl IntoComputed<Str>) -> Link<Label>
where
    Label: IntoLabel + 'static,
{
    Link::new(label, url)
}
