//! What a `WebView` looks like from the accessibility tree.
//!
//! The tree is `WaterUI`'s real observation channel, and it is all a screen
//! reader gets here: page content lives in a texture or a native subview that
//! the host tree cannot see into, so the component's own node is the whole
//! surface.
//!
//! The tests mount the shape a real engine deployment has — a
//! [`WebViewController`] carried in the environment plus the `Hook<WebView>`
//! that draws the component — with [`TestController`] standing in for the
//! engine so no page ever loads. What they assert is what the harness can
//! still observe: the realization publishes exactly one accessibility node,
//! and writing a new URL into a bound `Binding` navigates the web view it
//! already has instead of building another one.

use std::cell::{Cell, RefCell};
use std::future::{Future, ready};
use std::rc::Rc;
use std::time::Duration;

use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::shape::{Rectangle, ShapeExt as _};
use waterui::text;
use waterui::{Color, Environment, SignalExt as _, Str};
use waterui_core::accessibility::{AccessibilityRole, default_role};
use waterui_core::{AnyView, Metadata, Retain, Signal, binding};
use waterui_testing::{Role, UiBuilder};
use waterui_webview::{
    BackendEvent, Cookie, CustomWebViewController, OriginPolicy, ScriptInjectionTime,
    ScriptMessageHandler, Url, WatcherGuard, WatcherSet, WebView, WebViewConfig, WebViewController,
    WebViewHandle,
};

const DOCS_URL: &str = "https://waterui.dev/docs";
const API_URL: &str = "https://waterui.dev/api";

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
    #[expect(
        clippy::future_not_send,
        reason = "a web view and its handle are main-thread-affine"
    )]
    fn run_javascript(&self, _script: &str) -> impl Future<Output = Result<Str, Str>> {
        ready(Err(Str::from_static(
            "the test web view has no page to run JavaScript in",
        )))
    }

    /// Fails for the same reason as [`Self::run_javascript`]: an async call's
    /// result has no truthful empty value either.
    #[expect(
        clippy::future_not_send,
        reason = "a web view and its handle are main-thread-affine"
    )]
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

/// Writing a new URL into the binding a web view opened with navigates that
/// web view. The give-away for the other outcome — the view being torn down
/// and a fresh one built — is a second `open` on the controller, so that is
/// what this reads: one opened web view, navigated once at mount and once by
/// the write, while the value derived from the same binding follows too.
#[waterui::test(viewport = (420, 420))]
fn changing_the_url_binding_keeps_the_same_webview(ui: UiBuilder) {
    let url = binding(Url::new(DOCS_URL));
    let url_for_view = url.clone();

    let controller = TestController::new();
    let mut env = Environment::new();
    env.insert(WebViewController::new(controller.clone()));
    env.insert_hook::<WebView, AnyView>(realization);
    let mut app = ui.environment(env).mount(move || {
        let address = url_for_view
            .clone()
            .map(|url: Url| Str::from(url.as_str().to_owned()));
        vstack((
            text!("Address: {address}"),
            WebView::open(url_for_view.clone())
                .a11y_label("Docs WebView")
                .size(WEBVIEW_WIDTH, WEBVIEW_HEIGHT),
        ))
    });

    app.query()
        .label(format!("Address: {DOCS_URL}"))
        .assert_exists();
    app.query().label("Docs WebView").assert_exists();

    url.set(Url::new(API_URL));

    assert!(
        app.query()
            .label(format!("Address: {API_URL}"))
            .wait_for_existence(Duration::from_millis(250)),
        "the URL binding drives the view, so the derived address has to follow it"
    );
    assert_eq!(
        controller.opens.get(),
        1,
        "a new URL navigates the existing web view; a second `open` means the \
         component was rebuilt instead"
    );
    assert_eq!(
        controller.navigations.borrow().as_slice(),
        &[Url::new(DOCS_URL), Url::new(API_URL)],
        "the existing web view received the initial URL at mount and the new \
         URL as a navigation"
    );
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
#[waterui::test(viewport = (420, 420))]
fn an_installed_realization_draws_the_webview_and_keeps_its_node(ui: UiBuilder) {
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

    let mut app = ui.environment(env).mount(|| {
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
}
