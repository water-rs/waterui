//! Renderer presentation tests for `WebView`: the realized surface's bounds
//! and single accessibility node on the rendered runtime.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); the case names its origin file and asserts what it
//! asserted there, mounted under `Material3::defaults()` on the rendered
//! runtime.

use std::cell::{Cell, RefCell};
use std::future::{Future, ready};
use std::rc::Rc;

use waterui::ViewExt as _;
use waterui::shape::{Rectangle, ShapeExt as _};
use waterui::{Color, Environment, Str};
use waterui_core::accessibility::{AccessibilityRole, default_role};
use waterui_core::{AnyView, Metadata, Retain, Signal};
use waterui_testing::{Role, Styled, UiBuilder};
use waterui_webview::{
    BackendEvent, Cookie, CustomWebViewController, OriginPolicy, ScriptInjectionTime,
    ScriptMessageHandler, Url, WatcherGuard, WatcherSet, WebView, WebViewConfig, WebViewController,
    WebViewHandle,
};

const DOCS_URL: &str = "https://waterui.dev/docs";

const WEBVIEW_WIDTH: f32 = 320.0;
const WEBVIEW_HEIGHT: f32 = 260.0;

/// The controller these tests run against.
///
/// It fills the seat an engine's controller fills in a real deployment —
/// carried in the environment, opening a web view when the component is
/// created — while recording the two facts a test can observe without a page:
/// how many web views were opened, and where each was navigated. A second
/// `open` is the signature of the component being torn down and rebuilt
/// instead of navigated.
#[derive(Clone)]
struct TestController {
    opens: Rc<Cell<usize>>,
    navigations: Rc<RefCell<Vec<Url>>>,
}

impl TestController {
    fn new() -> Self {
        Self {
            opens: Rc::new(Cell::new(0)),
            navigations: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl CustomWebViewController for TestController {
    fn open(&self, _config: WebViewConfig) -> impl WebViewHandle {
        self.opens.set(self.opens.get() + 1);
        TestHandle {
            navigations: Rc::clone(&self.navigations),
            watchers: WatcherSet::new(),
        }
    }
}

/// A web view with no page behind it.
///
/// Configuration is accepted and discarded — there is no page to inject a
/// script into and no bridge for a handler to be reachable from — the history
/// stays permanently empty, and `go_to` is the one method with observable
/// behavior because navigation is what the binding test asserts on. The
/// watcher set is kept so guards register and unregister normally; nothing
/// ever emits into it.
#[derive(Debug, Clone)]
struct TestHandle {
    navigations: Rc<RefCell<Vec<Url>>>,
    watchers: WatcherSet<BackendEvent>,
}

impl WebViewHandle for TestHandle {
    fn go_back(&self) {}

    fn go_forward(&self) {}

    fn go_to(&self, url: &Url) {
        self.navigations.borrow_mut().push(url.clone());
    }

    fn stop(&self) {}

    fn refresh(&self) {}

    fn set_user_agent(&self, _user_agent: &str) {}

    fn can_go_back(&self) -> bool {
        false
    }

    fn can_go_forward(&self) -> bool {
        false
    }

    fn inject_script(&self, _key: &str, _script: &str, _time: ScriptInjectionTime) {}

    fn add_handler(&self, _name: &str, _handler: Box<ScriptMessageHandler>) {}

    fn remove_handler(&self, _name: &str) {}

    fn set_bridge_origins(&self, _policy: OriginPolicy) {}

    fn set_cookie(&self, _cookie: Cookie<'static>) {}

    /// No engine means no interception facility an asset origin could stand on.
    fn asset_origin(&self) -> Option<Url> {
        None
    }

    fn set_redirects_enabled(&self, _enabled: impl Signal<Output = bool>) {}

    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.watchers.insert(f)
    }

    fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> {
        ready(Vec::new())
    }

    /// Fails rather than answering, because there is no page to answer for.
    fn run_javascript(&self, _script: &str) -> impl Future<Output = Result<Str, Str>> {
        ready(Err(Str::from_static(
            "the test web view has no page to run JavaScript in",
        )))
    }

    /// Fails for the same reason as [`Self::run_javascript`]: an async call's
    /// result has no truthful empty value either.
    fn call_async_javascript(&self, _body: &str) -> impl Future<Output = Result<Str, Str>> {
        ready(Err(Str::from_static(
            "the test web view has no page to call JavaScript in",
        )))
    }
}

/// The realization both tests install in place of a linked engine.
///
/// It stands in for `waterui_browser_cef::install` / `..._wpe::install` the
/// way those look from the tree: [`default_role`] wrapped around opaque
/// self-drawn content, holding the `WebView` alive for the surface's
/// lifetime. A filled shape stands in for the surface because it reaches the
/// renderer's self-drawn-graphics leaf the same way and needs no GPU device.
fn realization(env: &Environment, webview: WebView) -> AnyView {
    AnyView::new(Metadata::new(
        default_role(
            env,
            Rectangle.fill(Color::srgb_hex("#3B82F6")),
            AccessibilityRole::Group,
        ),
        Retain::new(webview),
    ))
}

/// A browser engine the application links installs a `Hook<WebView>`, and that
/// realization is what draws the component — including on a backend that
/// bridges a platform engine of its own and would otherwise take the
/// component by type before `body` ever ran.
///
/// The filled shape the hook returns also proves the accessibility half of
/// the engine path: the node a screen reader gets when nothing in the host
/// tree can see into the page, and the role the realization gives it in place
/// of the `Image` a graphics leaf would default to.
///
/// Origin: waterui `components/platform/webview/tests/e2e_semantics.rs`.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (420, 420))]
fn an_installed_realization_draws_the_webview_and_keeps_its_node(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let drawn = Rc::new(Cell::new(false));
    let mut env = Environment::new();
    env.insert(WebViewController::new(TestController::new()));
    env.insert_hook::<WebView, AnyView>({
        let drawn = Rc::clone(&drawn);
        move |env: &Environment, webview: WebView| {
            drawn.set(true);
            // The realization owns the semantic component for the surface's
            // lifetime, the way every engine install does.
            realization(env, webview)
        }
    });

    let mut app = ui.environment(env).mount_offscreen(|| {
        WebView::open(DOCS_URL)
            .a11y_label("Docs WebView")
            .size(WEBVIEW_WIDTH, WEBVIEW_HEIGHT)
    });

    assert!(
        drawn.get(),
        "the installed realization draws the web view, not the backend's own path"
    );

    let nodes = app.query().label("Docs WebView").all();
    assert_eq!(
        nodes.len(),
        1,
        "the realization publishes exactly one accessibility node, found: {nodes:?}"
    );
    let webview = app.query().label("Docs WebView").single();
    assert_eq!(
        webview.node().role(),
        Role::GROUP,
        "page content is opaque, so the surface itself reads as a container"
    );
    let bounds = webview.bounds();
    assert!(
        (bounds.width() - WEBVIEW_WIDTH).abs() < 0.5
            && (bounds.height() - WEBVIEW_HEIGHT).abs() < 0.5,
        "the published node covers the web view's own bounds, got {}x{}",
        bounds.width(),
        bounds.height(),
    );
}
