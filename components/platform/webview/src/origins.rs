//! Which documents may use the bridge.
//!
//! A handler is a capability. Until this existed the bridge was installed
//! globally, so whatever page happened to be loaded could call every registered
//! handler — and a web view that can navigate is a web view that can end up
//! somewhere you did not choose. The default is therefore the narrowest one that
//! still works: the origin the view was opened at, main frame only.
//!
//! Widening is explicit and visible at the call site:
//!
//! ```ignore
//! WebView::open("https://app.waterui.dev")
//!     .handler("save", ...)
//!     .bridge_origins(["https://app.waterui.dev", "https://docs.waterui.dev"])
//! ```

use waterui_str::Str;
use waterui_url::Url;

/// Which origins may reach the bridge.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BridgeOrigins {
    /// Only the origin the web view was opened at. The default.
    #[default]
    Initial,
    /// The listed origins, each `scheme://host[:port]`.
    Allowed(Vec<Str>),
    /// Local files, whose origins engines treat inconsistently — some give every
    /// file its own opaque origin — so they cannot be matched by comparison and
    /// have to be opted into as a group.
    LocalFiles,
    /// Every origin, including whatever a page navigates to next.
    ///
    /// Every registered handler becomes reachable by any page the view loads.
    /// Only appropriate when the web view shows content you control end to end.
    Any,
}

/// Resolves a policy against the origin a web view was opened at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginPolicy {
    origins: BridgeOrigins,
    initial: Option<Str>,
}

impl OriginPolicy {
    /// Builds a policy for a view opened at `initial`.
    #[must_use]
    pub fn new(origins: BridgeOrigins, initial: &Url) -> Self {
        Self {
            origins,
            initial: origin_of(initial),
        }
    }

    /// Whether a document at `url` may use the bridge.
    #[must_use]
    pub fn allows(&self, url: &Url) -> bool {
        match &self.origins {
            BridgeOrigins::Any => true,
            BridgeOrigins::LocalFiles => url.is_local(),
            BridgeOrigins::Initial => match (&self.initial, origin_of(url)) {
                (Some(initial), Some(origin)) => *initial == origin,
                // An opaque origin — `data:`, `blob:`, a sandboxed frame — matches
                // nothing, which is the point: it cannot be authenticated.
                _ => false,
            },
            BridgeOrigins::Allowed(allowed) => {
                origin_of(url).is_some_and(|origin| allowed.contains(&origin))
            }
        }
    }

    /// The origins as URI patterns, for backends that filter injection natively.
    ///
    /// `None` means "every origin", which is how `WebKit` spells an unrestricted
    /// allow list.
    #[must_use]
    pub fn uri_patterns(&self) -> Option<Vec<Str>> {
        match &self.origins {
            BridgeOrigins::Any => None,
            BridgeOrigins::LocalFiles => Some(vec![Str::from_static("file://*")]),
            BridgeOrigins::Initial => self
                .initial
                .clone()
                .map(|origin| vec![Str::from(format!("{origin}/*"))]),
            BridgeOrigins::Allowed(allowed) => Some(
                allowed
                    .iter()
                    .map(|origin| Str::from(format!("{origin}/*")))
                    .collect(),
            ),
        }
    }
}

/// Extracts `scheme://host[:port]`, or `None` when the URL has no origin to
/// compare — a local path, a `data:` or `blob:` document, an opaque scheme.
fn origin_of(url: &Url) -> Option<Str> {
    if !url.is_web() {
        return None;
    }
    let scheme = url.scheme()?;
    let host = url.host()?;
    Some(url.port().map_or_else(
        || Str::from(format!("{scheme}://{host}")),
        |port| Str::from(format!("{scheme}://{host}:{port}")),
    ))
}

/// Accepted by [`WebViewOpen::bridge_origins`](crate::WebViewOpen::bridge_origins).
///
/// Lets a list of origins be passed directly, which is the common case, without
/// hiding the explicit [`BridgeOrigins`] variants.
pub trait IntoBridgeOrigins {
    /// Converts into a policy.
    fn into_bridge_origins(self) -> BridgeOrigins;
}

impl IntoBridgeOrigins for BridgeOrigins {
    fn into_bridge_origins(self) -> Self {
        self
    }
}

impl<I, S> IntoBridgeOrigins for I
where
    I: IntoIterator<Item = S>,
    S: Into<Str>,
{
    fn into_bridge_origins(self) -> BridgeOrigins {
        BridgeOrigins::Allowed(self.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{BridgeOrigins, OriginPolicy};
    use waterui_url::Url;

    fn policy(origins: BridgeOrigins, initial: &str) -> OriginPolicy {
        let initial: Url = initial.parse().expect("a valid initial URL");
        OriginPolicy::new(origins, &initial)
    }

    #[test]
    fn the_default_admits_only_the_origin_the_view_was_opened_at() {
        let policy = policy(BridgeOrigins::Initial, "https://app.waterui.dev/start");

        assert!(
            policy.allows(
                &"https://app.waterui.dev/other"
                    .parse::<Url>()
                    .expect("parses")
            )
        );
        // A different host, scheme or port is a different origin.
        assert!(!policy.allows(&"https://evil.example/".parse::<Url>().expect("parses")));
        assert!(!policy.allows(&"http://app.waterui.dev/".parse::<Url>().expect("parses")));
        assert!(
            !policy.allows(
                &"https://app.waterui.dev:8443/"
                    .parse::<Url>()
                    .expect("parses")
            )
        );
    }

    /// The case that motivates the whole policy: a view that navigates away must
    /// not carry its handlers to wherever it lands.
    #[test]
    fn navigating_away_loses_the_bridge() {
        let policy = policy(BridgeOrigins::Initial, "https://app.waterui.dev");
        assert!(
            !policy.allows(
                &"https://news.example/article"
                    .parse::<Url>()
                    .expect("parses")
            )
        );
    }

    #[test]
    fn opaque_origins_never_match() {
        let policy = policy(BridgeOrigins::Initial, "https://app.waterui.dev");
        for url in [
            "data:text/html,hi",
            "blob:https://app.waterui.dev/1",
            "about:blank",
        ] {
            let url: Url = url.parse().expect("parses");
            assert!(!policy.allows(&url), "{url} must not reach the bridge");
        }
    }

    #[test]
    fn an_allow_list_admits_exactly_its_entries() {
        let policy = policy(
            BridgeOrigins::Allowed(vec![
                "https://app.waterui.dev".into(),
                "https://docs.waterui.dev".into(),
            ]),
            "https://app.waterui.dev",
        );

        assert!(
            policy.allows(
                &"https://docs.waterui.dev/guide"
                    .parse::<Url>()
                    .expect("parses")
            )
        );
        assert!(!policy.allows(&"https://other.waterui.dev/".parse::<Url>().expect("parses")));
    }

    #[test]
    fn local_files_are_their_own_opt_in() {
        let policy = policy(BridgeOrigins::LocalFiles, "https://app.waterui.dev");
        let file: Url = "file:///tmp/app.html".parse().expect("parses");

        assert!(policy.allows(&file));
        assert!(!policy.allows(&"https://app.waterui.dev".parse::<Url>().expect("parses")));
    }

    #[test]
    fn any_admits_everything_including_what_the_page_navigates_to() {
        let policy = policy(BridgeOrigins::Any, "https://app.waterui.dev");
        assert!(policy.allows(&"https://evil.example/".parse::<Url>().expect("parses")));
    }

    #[test]
    fn patterns_describe_the_same_policy_for_native_filters() {
        assert_eq!(
            policy(BridgeOrigins::Initial, "https://app.waterui.dev").uri_patterns(),
            Some(vec!["https://app.waterui.dev/*".into()])
        );
        assert_eq!(
            policy(BridgeOrigins::Any, "https://a.dev").uri_patterns(),
            None
        );
    }
}
