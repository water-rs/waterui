use nami::Computed;
use nami::signal::IntoComputed;
use waterui_core::{AnyView, NativeView, configurable};
use waterui_core::{Str, View};

#[derive(Debug)]
/// Configuration for the `Link` component.
pub struct LinkConfig {
    /// The label of the link.
    pub label: AnyView,
    /// The URL the link points to.
    pub url: Computed<Str>,
}

impl NativeView for LinkConfig {}

configurable!(
    /// A tappable text link that opens a URL.
    ///
    /// Link displays styled text that navigates to the specified URL when tapped.
    ///
    /// # Layout Behavior
    ///
    /// Link sizes itself to fit its label content and never stretches to fill extra space.
    /// In a stack, it takes only the space it needs, just like Text.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Stretch Axis: `None` - Link never expands to fill available space.
    // Size: Determined by label content (same as Text)
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Link,
    LinkConfig
);

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
