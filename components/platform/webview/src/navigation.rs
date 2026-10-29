//! Which page-initiated navigations a web view may perform.
//!
//! A web view is a browsing context: the page in it can send the main frame
//! anywhere — a link, a `window.location` write, a form submission, a
//! `window.open`. The policy installed through
//! [`WebViewOpen::navigation_policy`](crate::WebViewOpen::navigation_policy)
//! answers each of those before the engine commits to it. Loads the
//! application itself directs — the URL the view was opened at, every write
//! to its URL signal — are never asked about, and neither are server
//! redirects, which [`WebViewOpen::redirects_enabled`](crate::WebViewOpen::redirects_enabled)
//! governs.

use std::sync::Arc;

use waterui_url::Url;

/// What a navigation policy decides for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NavigationDecision {
    /// The engine proceeds with the navigation.
    Allow,
    /// The engine cancels it; nothing happens.
    Cancel,
    /// The engine cancels it and the URL is opened in the system browser.
    ///
    /// The shared layer performs the open — through `robius-open` — inside
    /// the decision itself, so a backend only ever answers "proceed" or
    /// "cancel" to its engine's hook. A failed open is logged and the
    /// navigation stays cancelled.
    OpenExternally,
}

/// One page-initiated navigation, as a policy sees it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NavigationRequest {
    /// The URL the navigation targets.
    pub url: Url,
    /// The document the navigation leaves, when there is one.
    pub from: Option<Url>,
    /// Whether the main frame is the target. A targetless action —
    /// `window.open`, a `target="_blank"` — that a backend reroutes into
    /// its own view counts as one.
    pub is_main_frame: bool,
    /// Whether a user gesture triggered the navigation: a link activation.
    /// Programmatic jumps (`window.location`) report `false`.
    pub user_initiated: bool,
}

impl NavigationRequest {
    /// The URL the navigation targets.
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// The document the navigation leaves, when there is one.
    #[must_use]
    pub const fn from(&self) -> Option<&Url> {
        self.from.as_ref()
    }

    /// Whether the main frame is the target. A targetless action —
    /// `window.open`, a `target="_blank"` — that a backend reroutes into
    /// its own view counts as one.
    #[must_use]
    pub const fn is_main_frame(&self) -> bool {
        self.is_main_frame
    }

    /// Whether a user gesture triggered the navigation: a link activation.
    /// Programmatic jumps (`window.location`) report `false`.
    #[must_use]
    pub const fn user_initiated(&self) -> bool {
        self.user_initiated
    }
}

impl NavigationRequest {
    /// The request a backend hands to the policy: every engine knows its
    /// own spelling of the target, the document being left, the frame and
    /// the gesture.
    #[must_use]
    pub const fn new(
        url: Url,
        from: Option<Url>,
        is_main_frame: bool,
        user_initiated: bool,
    ) -> Self {
        Self {
            url,
            from,
            is_main_frame,
            user_initiated,
        }
    }
}

/// The shared navigation-policy closure, carried by [`WebViewConfig`](crate::WebViewConfig).
///
/// [`WebViewOpen::navigation_policy`](crate::WebViewOpen::navigation_policy)
/// wraps the application's closure in this; a backend calls
/// [`allows`](Self::allows) from its engine's synchronous decision hook.
#[derive(Clone)]
pub struct NavigationPolicy {
    decide: Arc<dyn Fn(&NavigationRequest) -> NavigationDecision + Send + Sync>,
}

impl std::fmt::Debug for NavigationPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationPolicy").finish_non_exhaustive()
    }
}

impl NavigationPolicy {
    /// Wraps `policy` — `Fn(&NavigationRequest) -> NavigationDecision`.
    #[must_use]
    pub fn new(
        policy: impl Fn(&NavigationRequest) -> NavigationDecision + Send + Sync + 'static,
    ) -> Self {
        Self {
            decide: Arc::new(policy),
        }
    }

    /// Whether the engine proceeds with `request`.
    ///
    /// `Allow` answers `true`; `Cancel` and `OpenExternally` both answer
    /// `false`, and `OpenExternally` opens the target with the system
    /// browser first — the engine's hooks cannot express "send this to the
    /// system", so the shared layer carries the decision out itself. An
    /// open that fails is logged and still answers `false`.
    #[must_use]
    pub fn allows(&self, request: &NavigationRequest) -> bool {
        match (self.decide)(request) {
            NavigationDecision::Allow => true,
            NavigationDecision::Cancel => false,
            NavigationDecision::OpenExternally => {
                if let Err(error) = robius_open::Uri::new(request.url.as_str()).open() {
                    tracing::error!(
                        url = %request.url,
                        ?error,
                        "webview: opening a navigation externally failed"
                    );
                }
                false
            }
        }
    }
}

/// The policy `include_web!` installs: main-frame navigations stay on the
/// origin of the document they leave, every other main-frame target goes to
/// the system browser, and subframes are allowed.
///
/// "Same origin" is [`Url::origin`] equality — scheme, host and port, with a
/// scheme's default port elided — so the bundled frontend's
/// in-app links keep working inside the view while `https://` targets open
/// outside it. A navigation that leaves no document at all — before the
/// first commit — has no origin to stay on and opens externally too.
#[must_use]
pub fn stay_on_origin(request: &NavigationRequest) -> NavigationDecision {
    if !request.is_main_frame {
        return NavigationDecision::Allow;
    }
    // An opaque origin equals nothing, so only two tuple origins can match.
    let stays = request
        .from
        .as_ref()
        .and_then(Url::origin)
        .is_some_and(|from| request.url.origin() == Some(from));
    if stays {
        NavigationDecision::Allow
    } else {
        NavigationDecision::OpenExternally
    }
}

#[cfg(test)]
mod tests {
    use super::{NavigationDecision, NavigationPolicy, NavigationRequest, stay_on_origin};
    use waterui_url::Url;

    fn request(to: &str, from: Option<&str>, main: bool, gesture: bool) -> NavigationRequest {
        NavigationRequest {
            url: to.parse::<Url>().expect("a valid target URL"),
            from: from.map(|from| from.parse::<Url>().expect("a valid source URL")),
            is_main_frame: main,
            user_initiated: gesture,
        }
    }

    #[test]
    fn stay_on_origin_allows_same_origin_main_frame() {
        for (from, to) in [
            ("waterui://localhost/index.html", "waterui://localhost/page"),
            ("https://app.dev/a", "https://app.dev/b"),
            // The default port is the origin's own.
            ("https://app.dev:443/a", "https://app.dev/b"),
        ] {
            let request = request(to, Some(from), true, true);
            assert_eq!(
                stay_on_origin(&request),
                NavigationDecision::Allow,
                "{from} -> {to} stays"
            );
        }
    }

    #[test]
    fn stay_on_origin_sends_every_other_main_frame_target_out() {
        for (from, to) in [
            ("waterui://localhost/index.html", "https://waterui.dev/"),
            ("https://app.dev/a", "https://evil.example/"),
            // A different scheme or port is a different origin.
            ("https://app.dev/a", "http://app.dev/b"),
            ("https://app.dev/a", "https://app.dev:8443/b"),
            // An opaque target shares no origin.
            ("https://app.dev/a", "about:blank"),
        ] {
            let request = request(to, Some(from), true, true);
            assert_eq!(
                stay_on_origin(&request),
                NavigationDecision::OpenExternally,
                "{from} -> {to} leaves the origin"
            );
        }
    }

    #[test]
    fn stay_on_origin_allows_subframes() {
        // The CSP, not the navigation policy, governs framing.
        let request = request(
            "https://ads.example/frame",
            Some("https://app.dev/"),
            false,
            false,
        );
        assert_eq!(stay_on_origin(&request), NavigationDecision::Allow);
    }

    #[test]
    fn stay_on_origin_opens_externally_when_there_is_no_document_to_leave() {
        let request = request("https://app.dev/", None, true, false);
        assert_eq!(stay_on_origin(&request), NavigationDecision::OpenExternally);
    }

    #[test]
    fn the_policy_maps_allow_and_cancel_to_the_engine_answer() {
        let target = request("https://app.dev/", None, true, false);
        assert!(NavigationPolicy::new(|_| NavigationDecision::Allow).allows(&target));
        assert!(!NavigationPolicy::new(|_| NavigationDecision::Cancel).allows(&target));
        // `OpenExternally`'s mapping is `false`, and the open itself is the
        // system call a unit test must not make.
    }
}
