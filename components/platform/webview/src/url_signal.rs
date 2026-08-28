//! Reactive URL inputs for [`WebView::open`](crate::WebView::open).

use waterui_core::{Binding, Computed};
use waterui_url::{IntoUrl, Url};

/// A URL, or a signal of URLs, that a [`WebView`](crate::WebView) can follow.
///
/// Writing a new [`Url`] into a bound `Binding<Url>` navigates the existing native
/// web view; it never rebuilds it.
///
/// The implementations are deliberately concrete rather than blanket. A blanket
/// impl over every `Signal<Output = Url>` cannot coexist with the plain-value
/// impls under Rust's coherence rules, and listing the cases keeps the accepted
/// inputs visible in the docs. Any other signal converts with
/// [`SignalExt::computed`](waterui_core::SignalExt::computed).
///
/// # Examples
///
/// ```ignore
/// // A literal, checked when it is written.
/// WebView::open("https://waterui.dev");
///
/// // A binding: writing to it navigates the live web view.
/// let url = binding(Url::new("https://waterui.dev"));
/// let view = WebView::open(url.clone());
/// url.set(Url::new("https://waterui.dev/docs"));
/// ```
pub trait IntoUrlSignal {
    /// Converts `self` into the signal the web view follows.
    fn into_url_signal(self) -> Computed<Url>;
}

impl IntoUrlSignal for Url {
    fn into_url_signal(self) -> Computed<Self> {
        Computed::constant(self)
    }
}

impl IntoUrlSignal for &Url {
    fn into_url_signal(self) -> Computed<Url> {
        Computed::constant(self.clone())
    }
}

impl IntoUrlSignal for &'static str {
    /// # Panics
    ///
    /// Panics when the literal is not a valid URL, matching [`Url::new`].
    fn into_url_signal(self) -> Computed<Url> {
        Computed::constant(self.into_url())
    }
}

impl IntoUrlSignal for Binding<Url> {
    fn into_url_signal(self) -> Computed<Url> {
        Computed::from(self)
    }
}

impl IntoUrlSignal for &Binding<Url> {
    fn into_url_signal(self) -> Computed<Url> {
        Computed::from(self.clone())
    }
}

impl IntoUrlSignal for Computed<Url> {
    fn into_url_signal(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::IntoUrlSignal;
    use waterui_core::{Binding, Signal};
    use waterui_url::Url;

    #[test]
    fn a_literal_becomes_a_constant_signal() {
        let signal = "https://waterui.dev".into_url_signal();
        assert_eq!(signal.get().as_str(), "https://waterui.dev");
    }

    #[test]
    fn a_binding_tracks_later_writes() {
        let url = Binding::container(Url::new("https://waterui.dev"));
        let signal = url.clone().into_url_signal();
        assert_eq!(signal.get().as_str(), "https://waterui.dev");

        url.set(Url::new("https://waterui.dev/docs"));
        assert_eq!(signal.get().as_str(), "https://waterui.dev/docs");
    }
}
