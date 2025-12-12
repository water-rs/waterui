//! Link component for `WaterUI`
//!
//! This module provides a Link component that displays clickable text that opens a URL.
//! Link is implemented as a Button with link style, using `robius-open` to open URLs.

use nami::Computed;
use nami::Signal;
use nami::signal::IntoComputed;
use nami::signal::IntoSignal;
use waterui_controls::button::{Button, ButtonStyle};
use waterui_core::{Environment, Str, View};

/// Opens a URL in the system's default browser/handler.
fn open_url(url: &str) {
    if let Err(e) = robius_open::Uri::new(url).open() {
        tracing::error!("Failed to open URL '{}': {:?}", url, e);
    }
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
/// let my_link = link("Visit website", "https://example.com");
/// ```
#[derive(Debug)]
pub struct Link<Label> {
    label: Label,
    url: Computed<Str>,
}

impl<Label> Link<Label>
where
    Label: View,
{
    pub fn new(label: Label, url: impl IntoComputed<Str>) -> Link<Label> {
        Link {
            label,
            url: url.into_computed(),
        }
    }
}

impl<Label> View for Link<Label>
where
    Label: View,
{
    fn body(self, _env: &Environment) -> impl View {
        let url = self.url;

        Button::new(self.label)
            .style(ButtonStyle::Link)
            .action(move || {
                let url_str = url.get();
                open_url(&url_str);
            })
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
    Label: View,
{
    Link::new(label, url)
}
