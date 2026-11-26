//! Link component for `WaterUI`.
//!
//! Displays an interactive hyperlink that delegates its label and destination
//! URL to the active backend through [`LinkConfig`].

use core::fmt;

use nami::Computed;
use nami::signal::IntoComputed;
use waterui_controls::{ControlLabel, IntoLabel};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_url::Url;

/// Resolved configuration passed to [`NativeBackend`](waterui_backend_core::NativeBackend)
/// for a [`Link`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LinkConfig<S> {
    label: ControlLabel<S>,
    url: Computed<Url>,
}

impl<S> LinkConfig<S> {
    /// Returns the semantic label displayed by the link.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// Returns the destination URL opened when the link is activated.
    #[must_use]
    pub const fn url(&self) -> &Computed<Url> {
        &self.url
    }

    /// Deconstructs the configuration into its label and destination URL.
    #[must_use]
    pub fn into_parts(self) -> (ControlLabel<S>, Computed<Url>) {
        (self.label, self.url)
    }
}

impl<S: 'static> MapSlots<S> for LinkConfig<S> {
    type With<T: 'static> = LinkConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> LinkConfig<T> {
        LinkConfig {
            label: self.label.map_slots(f),
            url: self.url,
        }
    }
}

impl<S: 'static> NativeView for LinkConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}

impl<S: 'static> fmt::Display for LinkConfig<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Link")
    }
}

/// A tappable link that opens a URL.
///
/// Backends realize `Link` with native link semantics, link styling, pointer
/// affordances, and system URL activation.
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
#[derive(Debug, Clone)]
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

    fn into_config<S: ChildSlot>(self, env: &Environment) -> LinkConfig<S> {
        LinkConfig {
            label: self.label.into_label().into_control_label::<S>(env),
            url: self.url,
        }
    }
}

impl<Label> View for Link<Label>
where
    Label: IntoLabel + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
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
    use waterui_core::backend::{MapSlots, peel_native};
    use waterui_core::layout::StretchAxis;
    use waterui_core::{AnyView, Environment, NativeView};
    use waterui_url::Url;

    use super::{Link, LinkConfig, link};

    #[test]
    fn link_accepts_literal_url_binding_and_computed() {
        let _ = Link::new("Static", "https://waterui.dev");
        let parsed = Url::new("https://waterui.dev/docs");
        let _ = link("Url", parsed.clone());

        let url_binding = Binding::new(parsed);
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

    #[test]
    fn link_dispatches_link_config_and_maps_slots() {
        let env = Environment::new();
        let config =
            peel_native::<LinkConfig<AnyView>>(link("Docs", "https://waterui.dev/docs"), &env)
                .expect("Link must dispatch LinkConfig directly");

        assert_eq!(config.stretch_axis(), StretchAxis::None);
        assert_eq!(config.url().snapshot().as_str(), "https://waterui.dev/docs");

        let mapped = Link::new("Mapped", "https://waterui.dev/guide")
            .into_config::<AnyView>(&env)
            .map_slots(AnyView::new);
        assert_eq!(
            mapped.url().snapshot().as_str(),
            "https://waterui.dev/guide"
        );
    }
}
