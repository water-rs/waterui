//! What a web view is created with.
//!
//! An engine honors some inputs only while its native view is being
//! constructed — `WKWebView` takes scheme handlers on its configuration before
//! the view exists, and CEF hands a renderer process the browser's
//! `extra_info` before that process loads its first document. Everything a
//! document may receive before its first line of script therefore arrives
//! here, resolved, rather than through calls on a handle that already exists:
//! the asset server, the bridge's admission rules, the application's scripts
//! and the declarations of the state the page mirrors.

#[cfg(not(target_arch = "wasm32"))]
use suiteki::Str;

#[cfg(not(target_arch = "wasm32"))]
use crate::{AssetServer, NavigationPolicy, OriginPolicy, ScriptInjectionTime, StateDeclarations};

/// Creation-time configuration for a web view.
///
/// The input to
/// [`CustomWebViewController::open`](crate::CustomWebViewController::open).
/// [`WebView::open`](crate::WebView::open) and
/// [`WebView::open_assets`](crate::WebView::open_assets) resolve it — the
/// admission rules against the URL the view opens at, which for bundled assets
/// is under the engine's own [asset origin](crate::WebViewController::asset_origin)
/// — before the engine creates anything.
#[derive(Clone)]
#[non_exhaustive]
pub struct WebViewConfig {
    #[cfg(not(target_arch = "wasm32"))]
    /// The server behind the engine's local asset origin, when the view was
    /// opened with one.
    pub asset_server: Option<AssetServer>,
    #[cfg(not(target_arch = "wasm32"))]
    /// Which documents may reach the bridge, for the view's whole life.
    pub bridge_origins: OriginPolicy,
    #[cfg(not(target_arch = "wasm32"))]
    /// The policy that decides navigations the page initiates; `None`
    /// allows them all.
    pub navigation_policy: Option<NavigationPolicy>,
    #[cfg(not(target_arch = "wasm32"))]
    /// The application's scripts, in the order they run. Keys are unique.
    pub scripts: Vec<UserScript>,
    #[cfg(not(target_arch = "wasm32"))]
    /// The keys the page mirrors — names and writability, never values.
    pub state: StateDeclarations,
}

impl WebViewConfig {
    /// Creates a `WebViewConfig` from its parts.
    #[must_use]
    pub const fn new(
        asset_server: Option<AssetServer>,
        bridge_origins: OriginPolicy,
        navigation_policy: Option<NavigationPolicy>,
        scripts: Vec<UserScript>,
        state: StateDeclarations,
    ) -> Self {
        Self {
            asset_server,
            bridge_origins,
            navigation_policy,
            scripts,
            state,
        }
    }

    /// The server behind the engine's local asset origin, when the view was
    /// opened with one.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn asset_server(&self) -> Option<&AssetServer> {
        self.asset_server.as_ref()
    }

    /// Sets the `asset_server`.
    pub fn set_asset_server(&mut self, asset_server: Option<AssetServer>) {
        self.asset_server = asset_server;
    }

    /// Which documents may reach the bridge, for the view's whole life.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn bridge_origins(&self) -> &OriginPolicy {
        &self.bridge_origins
    }

    /// The policy that decides navigations the page initiates; `None`
    /// allows them all.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn navigation_policy(&self) -> Option<&NavigationPolicy> {
        self.navigation_policy.as_ref()
    }

    /// The application's scripts, in the order they run. Keys are unique.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn scripts(&self) -> &[UserScript] {
        self.scripts.as_slice()
    }

    /// The keys the page mirrors — names and writability, never values.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn state(&self) -> &StateDeclarations {
        &self.state
    }
}

impl Default for WebViewConfig {
    /// A view with no asset origin, no script and a bridge no document may
    /// reach.
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            asset_server: None,
            #[cfg(not(target_arch = "wasm32"))]
            bridge_origins: OriginPolicy::deny_all(),
            #[cfg(not(target_arch = "wasm32"))]
            navigation_policy: None,
            #[cfg(not(target_arch = "wasm32"))]
            scripts: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            state: StateDeclarations::default(),
        }
    }
}

impl std::fmt::Debug for WebViewConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("WebViewConfig");
        #[cfg(not(target_arch = "wasm32"))]
        debug
            .field("asset_server", &self.asset_server.as_ref().map(|_| ".."))
            .field("bridge_origins", &self.bridge_origins)
            .field(
                "navigation_policy",
                &self.navigation_policy.as_ref().map(|_| ".."),
            )
            .field("scripts", &self.scripts)
            .field("state", &self.state);
        debug.finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl WebViewConfig {
    /// Admits the documents `bridge_origins` admits to the bridge.
    #[must_use]
    pub fn with_bridge_origins(mut self, bridge_origins: OriginPolicy) -> Self {
        self.bridge_origins = bridge_origins;
        self
    }

    /// Runs `scripts`, in order, in every admitted document.
    #[must_use]
    pub fn with_scripts(mut self, scripts: Vec<UserScript>) -> Self {
        self.scripts = scripts;
        self
    }

    /// Every script an admitted document runs besides the bridge itself, in
    /// order: the mirrored-state seed, then the application's scripts.
    ///
    /// The seed goes first so a script that watches a key at document start
    /// finds it declared. It carries no value, so it is as static as the rest
    /// of the configuration.
    #[must_use]
    pub fn user_scripts(&self) -> Vec<UserScript> {
        let seed = UserScript {
            key: Str::from_static(crate::state::SEED_SCRIPT_KEY),
            source: Str::from(self.state.seed_script(&self.bridge_origins)),
            time: ScriptInjectionTime::DocumentStart,
        };
        core::iter::once(seed)
            .chain(self.scripts.iter().cloned())
            .collect()
    }

    /// The keys of [`user_scripts`](Self::user_scripts), in the same order,
    /// without rendering the seed.
    pub(crate) fn script_keys(&self) -> impl Iterator<Item = Str> + '_ {
        core::iter::once(Str::from_static(crate::state::SEED_SCRIPT_KEY))
            .chain(self.scripts.iter().map(|script| script.key.clone()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The keyed scripts an admitted document runs, as a backend keeps them: the
/// creation-time ones, fixed, then those injected after creation.
///
/// A backend whose engine injects scripts as one list — `WKUserContentController`
/// removes its scripts only as a set, Android's `ScriptHandler` set is pushed
/// whole — rebuilds that list from [`iter`](Self::iter) after every
/// [`inject`](Self::inject). The creation-time half cannot be written to:
/// [`AnyWebViewHandle::inject_script`](crate::AnyWebViewHandle::inject_script)
/// refuses their keys, so an injection only ever adds or replaces a
/// post-creation script.
#[derive(Debug, Clone)]
pub struct DocumentScripts {
    creation: Box<[UserScript]>,
    injected: Vec<UserScript>,
}

#[cfg(not(target_arch = "wasm32"))]
impl DocumentScripts {
    /// The scripts of a view opened with `config`, before any injection.
    #[must_use]
    pub fn new(config: &WebViewConfig) -> Self {
        Self {
            creation: config.user_scripts().into_boxed_slice(),
            injected: Vec::new(),
        }
    }

    /// Adds a post-creation script, or replaces the one already injected
    /// under its key in the position that one took, so the order the page
    /// runs them in does not change when one is re-rendered.
    pub fn inject(&mut self, script: UserScript) {
        match self
            .injected
            .iter_mut()
            .find(|existing| existing.key == script.key)
        {
            Some(existing) => *existing = script,
            None => self.injected.push(script),
        }
    }

    /// Every script, in the order a document runs them: the creation-time
    /// ones, then the injected ones in first-injection order.
    pub fn iter(&self) -> impl Iterator<Item = &UserScript> {
        self.creation.iter().chain(&self.injected)
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// One script the application injects into its documents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserScript {
    /// Names the script. Injecting again under the key replaces it.
    pub key: Str,
    /// The program.
    pub source: Str,
    /// When in a document's load it runs.
    pub time: ScriptInjectionTime,
}

#[cfg(not(target_arch = "wasm32"))]
impl UserScript {
    /// Names the script. Injecting again under the key replaces it.
    #[must_use]
    pub const fn key(&self) -> &Str {
        &self.key
    }

    /// The program.
    #[must_use]
    pub const fn source(&self) -> &Str {
        &self.source
    }

    /// When in a document's load it runs.
    #[must_use]
    pub const fn time(&self) -> ScriptInjectionTime {
        self.time
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl UserScript {
    /// The script `source`, named `key`, injected at `time`.
    #[must_use]
    pub fn new(key: impl Into<Str>, source: impl Into<Str>, time: ScriptInjectionTime) -> Self {
        Self {
            key: key.into(),
            source: source.into(),
            time,
        }
    }
}
