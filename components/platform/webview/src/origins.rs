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

    /// The rules a backend matches the calling frame's origin against.
    ///
    /// An empty list denies every document. That case is reachable — the default
    /// [`BridgeOrigins::Initial`] resolves to it whenever the view is opened at a
    /// URL with no origin to compare, such as a `file:` or `data:` document — and
    /// it must stay distinguishable from [`OriginRule::Any`]. Collapsing the two
    /// is what let a `file://` view hand every registered handler to whatever it
    /// navigated to next.
    #[must_use]
    pub fn rules(&self) -> Vec<OriginRule> {
        match &self.origins {
            BridgeOrigins::Any => vec![OriginRule::Any],
            BridgeOrigins::LocalFiles => vec![OriginRule::LocalFiles],
            BridgeOrigins::Initial => self
                .initial
                .clone()
                .map(OriginRule::Exact)
                .into_iter()
                .collect(),
            BridgeOrigins::Allowed(allowed) => {
                allowed.iter().cloned().map(OriginRule::Exact).collect()
            }
        }
    }

    /// Whether a frame reporting `origin` may use the bridge.
    ///
    /// `origin` is the `scheme://host[:port]` form every engine reports for a
    /// frame — `WKSecurityOrigin`, a CDP execution context, `WebKitFrame`. The
    /// comparison lives here rather than in each backend because every backend
    /// that wrote its own got it wrong in a different way: one dropped the port
    /// so `http://localhost:3000` was refused and `https://app.example:8443`
    /// admitted, another compared against a `file://*` glob that a local
    /// document's `file://` origin never matches.
    ///
    /// An empty or opaque origin matches nothing except [`BridgeOrigins::Any`],
    /// which is the point: it cannot be authenticated.
    #[must_use]
    pub fn allows_origin(&self, origin: &str) -> bool {
        self.rules().iter().any(|rule| match rule {
            OriginRule::Any => true,
            OriginRule::LocalFiles => origin.starts_with("file://"),
            OriginRule::Exact(allowed) => allowed == origin,
        })
    }

    /// The rules in the newline-separated form the FFI carries.
    ///
    /// The empty string is deny-all, so a backend can tell it apart from the
    /// single-rule `*` that means allow-all. [`OriginRule::parse_wire`] reads it
    /// back; every backend matches with the same two tokens rather than
    /// inventing its own glob dialect.
    #[must_use]
    pub fn wire(&self) -> Str {
        Str::from(
            self.rules()
                .iter()
                .map(OriginRule::as_token)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

/// One rule a backend matches a calling frame's origin against.
///
/// Backends receive these as tokens rather than URI globs because the two
/// questions differ: a native injection filter wants a URI pattern, while
/// authenticating a message needs an exact origin comparison. Mixing them is
/// what made `file://*` — a valid `WebKit` injection rule — unmatchable against
/// the `file://` origin every engine reports for a local document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginRule {
    /// Every origin, including whatever the page navigates to next.
    Any,
    /// Any document loaded from the local filesystem.
    LocalFiles,
    /// Exactly this `scheme://host[:port]`.
    Exact(Str),
}

impl OriginRule {
    /// The token this rule crosses the FFI as.
    #[must_use]
    pub const fn as_token(&self) -> &str {
        match self {
            Self::Any => "*",
            Self::LocalFiles => "file:",
            Self::Exact(origin) => origin.as_str(),
        }
    }

    /// Reads the rules back from the wire form produced by
    /// [`OriginPolicy::wire`].
    ///
    /// An empty string is deny-all and yields no rules.
    #[must_use]
    pub fn parse_wire(wire: &str) -> Vec<Self> {
        wire.split('\n')
            .filter(|line| !line.is_empty())
            .map(|line| match line {
                "*" => Self::Any,
                "file:" => Self::LocalFiles,
                origin => Self::Exact(Str::from(origin.to_owned())),
            })
            .collect()
    }

    /// The URI pattern a native injection filter takes for this rule.
    ///
    /// # This is a coarse pre-filter, never the authentication
    ///
    /// A backend must still decide every individual bridge message with
    /// [`OriginPolicy::allows_origin`]. Engines disagree about what a pattern
    /// may even say, and one of those disagreements fails silently: `WebKit`'s
    /// `UserContentURLPattern` **rejects a host containing a colon**, so the
    /// ported form this returns for `http://localhost:3000` parses as invalid,
    /// an invalid pattern matches nothing, and the bridge is injected *nowhere*
    /// — the whole feature dead on any dev server, with no error anywhere.
    ///
    /// A backend whose engine cannot express a port therefore widens the
    /// pattern to what it can (`scheme://host/*`) and relies on
    /// `allows_origin` for the exact decision. Widening the *filter* is safe
    /// precisely because it is not the check; widening the check is not.
    #[must_use]
    pub fn injection_pattern(&self) -> Str {
        match self {
            Self::Any => Str::from_static("*"),
            Self::LocalFiles => Str::from_static("file://*"),
            Self::Exact(origin) => Str::from(format!("{origin}/*")),
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
    use super::{BridgeOrigins, OriginPolicy, OriginRule};
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
            policy(BridgeOrigins::Initial, "https://app.waterui.dev")
                .rules()
                .iter()
                .map(OriginRule::injection_pattern)
                .collect::<Vec<_>>(),
            vec!["https://app.waterui.dev/*"]
        );
        assert_eq!(
            policy(BridgeOrigins::Any, "https://a.dev")
                .rules()
                .iter()
                .map(OriginRule::injection_pattern)
                .collect::<Vec<_>>(),
            vec!["*"]
        );
    }

    /// The case that made the default policy fail open: a view opened at a URL
    /// with no origin resolves to deny-all, and deny-all must not look like
    /// allow-all on the wire.
    #[test]
    fn a_policy_with_nothing_to_match_denies_rather_than_admits() {
        let policy = policy(BridgeOrigins::Initial, "file:///tmp/app.html");

        assert!(policy.rules().is_empty());
        assert_eq!(policy.wire().as_str(), "");
        assert!(OriginRule::parse_wire(policy.wire().as_str()).is_empty());
        assert_ne!(policy.wire(), policy_any().wire());
    }

    fn policy_any() -> OriginPolicy {
        policy(BridgeOrigins::Any, "https://app.waterui.dev")
    }

    /// The comparison backends actually perform, against the origin string an
    /// engine reports rather than against a URL.
    #[test]
    fn an_origin_string_is_matched_with_its_port() {
        let dev_server = policy(BridgeOrigins::Initial, "http://localhost:3000/app");
        assert!(dev_server.allows_origin("http://localhost:3000"));
        // Dropping the port is what refused every call from a dev server.
        assert!(!dev_server.allows_origin("http://localhost"));
        // ...and what admitted a different origin that merely shares a host.
        let production = policy(BridgeOrigins::Initial, "https://app.example/");
        assert!(!production.allows_origin("https://app.example:8443"));
        assert!(production.allows_origin("https://app.example"));
    }

    /// `file://` is how engines report a local document, and it has to match the
    /// opt-in that exists for exactly that case.
    #[test]
    fn local_file_origins_match_the_local_files_opt_in() {
        let policy = policy(BridgeOrigins::LocalFiles, "file:///tmp/app.html");
        assert!(policy.allows_origin("file://"));
        assert!(policy.allows_origin("file:///tmp/app.html"));
        assert!(!policy.allows_origin("https://app.waterui.dev"));
    }

    /// Deny-all must deny, including the empty origin an opaque document
    /// reports.
    #[test]
    fn a_policy_with_no_rules_admits_no_origin() {
        let policy = policy(BridgeOrigins::Initial, "file:///tmp/app.html");
        for origin in ["", "https://evil.example", "file://", "null"] {
            assert!(!policy.allows_origin(origin), "{origin} must be refused");
        }
    }

    #[test]
    fn the_wire_form_round_trips_every_rule() {
        for origins in [
            BridgeOrigins::Any,
            BridgeOrigins::LocalFiles,
            BridgeOrigins::Initial,
            BridgeOrigins::Allowed(vec![
                "https://app.waterui.dev".into(),
                "https://docs.waterui.dev".into(),
            ]),
        ] {
            let policy = policy(origins, "https://app.waterui.dev");
            assert_eq!(
                OriginRule::parse_wire(policy.wire().as_str()),
                policy.rules()
            );
        }
    }
}
