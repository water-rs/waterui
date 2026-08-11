//! End-to-end accessibility-semantics tests for the `webview` component.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::rc::Rc;
use std::time::Duration;

use cookie::Cookie;
use hydrolysis_m3::install as install_m3;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::component::{button, vstack};
use waterui::text;
use waterui::{Computed, Environment, Signal, SignalExt, Str, View};
use waterui_core::{Binding, binding};
use waterui_testing::{Role, Selector, WaitOptions, WaitResult, ui};
use waterui_webview::{
    BackendEvent, CustomWebViewController, ScriptInjectionTime, Url, WatcherGuard, WatcherSet, WebView,
    WebViewController, WebViewEvent, WebViewHandle,
};
const DOCS_URL: &str = "https://waterui.dev/docs";
const API_URL: &str = "https://waterui.dev/api";

#[derive(Default, Clone)]
struct FakeWebViewController {
    state: Rc<RefCell<FakeWebViewState>>,
}

impl CustomWebViewController for FakeWebViewController {
    fn open(&self) -> impl WebViewHandle {
        self.state.borrow_mut().open_count += 1;
        FakeWebViewHandle {
            state: self.state.clone(),
        }
    }
}

#[derive(Default, Clone)]
struct FakeWebViewHandle {
    state: Rc<RefCell<FakeWebViewState>>,
}

type MessageHandlers = BTreeMap<String, Box<waterui_webview::ScriptMessageHandler>>;

#[derive(Default)]
struct FakeWebViewState {
    open_count: usize,
    history: Vec<Url>,
    index: Option<usize>,
    user_agent: Option<String>,
    redirects_enabled: Option<Computed<bool>>,
    cookies: Vec<Cookie<'static>>,
    watchers: WatcherSet<BackendEvent>,
    handlers: MessageHandlers,
    bridge_origins: Option<waterui_webview::OriginPolicy>,
}

impl FakeWebViewHandle {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "test double; takes the event by value to mirror the real handle's API"
    )]
    fn emit(&self, event: impl Into<BackendEvent>) {
        let watchers = self.state.borrow().watchers.clone();
        watchers.emit(&event.into());
    }

    fn emit_state_changed(&self) {
        self.emit(BackendEvent::NavigationState {
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
        });
    }

    fn navigate_to(&self, url: Url) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(index) = state.index {
                state.history.truncate(index + 1);
            }
            state.history.push(url.clone());
            state.index = Some(state.history.len() - 1);
        }
        self.emit(WebViewEvent::WillNavigate { url });
        self.emit(WebViewEvent::Loading { progress: 1.0 });
        self.emit(WebViewEvent::Loaded);
        self.emit_state_changed();
    }
}

impl WebViewHandle for FakeWebViewHandle {
    fn go_back(&self) {
        let url = {
            let mut state = self.state.borrow_mut();
            let Some(index) = state.index else {
                return;
            };
            if index == 0 {
                return;
            }
            let next_index = index - 1;
            state.index = Some(next_index);
            state.history[next_index].clone()
        };
        self.emit(WebViewEvent::WillNavigate { url });
        self.emit(WebViewEvent::Loaded);
        self.emit_state_changed();
    }

    fn go_forward(&self) {
        let url = {
            let mut state = self.state.borrow_mut();
            let Some(index) = state.index else {
                return;
            };
            let next_index = index + 1;
            if next_index >= state.history.len() {
                return;
            }
            state.index = Some(next_index);
            state.history[next_index].clone()
        };
        self.emit(WebViewEvent::WillNavigate { url });
        self.emit(WebViewEvent::Loaded);
        self.emit_state_changed();
    }

    fn go_to(&self, url: &Url) {
        self.navigate_to(url.clone());
    }

    fn inject_script(&self, _script: &str, _time: ScriptInjectionTime) {}

    fn add_handler(&self, name: &str, handler: Box<waterui_webview::ScriptMessageHandler>) {
        self.state
            .borrow_mut()
            .handlers
            .insert(name.to_owned(), handler);
    }

    fn set_bridge_origins(&self, policy: waterui_webview::OriginPolicy) {
        self.state.borrow_mut().bridge_origins = Some(policy);
    }

    fn remove_handler(&self, name: &str) {
        self.state.borrow_mut().handlers.remove(name);
    }

    fn stop(&self) {}

    fn refresh(&self) {
        let state = self.state.borrow();
        let Some(index) = state.index else {
            return;
        };
        let url = state.history[index].clone();
        drop(state);
        self.emit(WebViewEvent::WillNavigate { url });
        self.emit(WebViewEvent::Loaded);
    }

    fn set_user_agent(&self, user_agent: &str) {
        self.state.borrow_mut().user_agent = Some(user_agent.to_owned());
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        self.state.borrow_mut().redirects_enabled = Some(Computed::new(enabled));
    }

    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        let watchers = self.state.borrow().watchers.clone();
        watchers.insert(f)
    }

    fn can_go_back(&self) -> bool {
        self.state.borrow().index.is_some_and(|index| index > 0)
    }

    fn can_go_forward(&self) -> bool {
        let state = self.state.borrow();
        state
            .index
            .is_some_and(|index| index + 1 < state.history.len())
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        self.state.borrow_mut().cookies.push(cookie);
    }

    fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> {
        ready(self.state.borrow().cookies.clone())
    }

    #[expect(
        clippy::future_not_send,
        reason = "test double for a main-thread `WebViewController`; `Str` is `!Send` by design"
    )]
    fn run_javascript(
        &self,
        script: &str,
    ) -> impl Future<Output = Result<waterui::Str, waterui::Str>> {
        ready(Ok(waterui::Str::from(script.to_owned())))
    }
}

#[test]
fn redirect_policy_retains_the_reactive_signal() {
    let backend = FakeWebViewController::default();
    let controller = WebViewController::new(backend.clone());
    let redirects = binding(false);
    let webview = controller.open().redirects_enabled(redirects.clone());
    let configured = backend
        .state
        .borrow()
        .redirects_enabled
        .clone()
        .expect("WebView backend did not take ownership of the redirect signal");

    assert!(!configured.get());
    redirects.set(true);
    assert!(configured.get());

    drop(webview);
}

#[test]
fn open_tracks_url_binding_without_recreating_the_webview() {
    let backend = FakeWebViewController::default();
    let controller = WebViewController::new(backend.clone());
    let url = binding(Url::new(DOCS_URL));
    let url_for_view = url.clone();

    let mut env = Environment::new();
    install_m3(&mut env);
    env.insert(controller);
    let _app = ui()
        .environment(env)
        .viewport(420, 420)
        .mount(move || WebView::open(url_for_view.clone()).size(320.0, 260.0));

    {
        let state = backend.state.borrow();
        assert_eq!(state.open_count, 1);
        assert_eq!(state.history.len(), 1);
        assert_eq!(state.history[0].as_str(), DOCS_URL);
    }

    url.set(Url::new(API_URL));

    let state = backend.state.borrow();
    assert_eq!(state.open_count, 1);
    assert_eq!(state.history.len(), 2);
    assert_eq!(state.history[1].as_str(), API_URL);
}

fn webview_test_view(webview: WebView) -> impl View {
    let webview_for_back = webview.clone();
    let webview_for_forward = webview.clone();
    let webview_for_navigate = webview.clone();

    // The assertions below read these two labels, so the view has to publish the
    // state they describe. Both are driven by the web view's own signals rather
    // than by the test, which is the point: this checks that a `WebView` reports
    // load and history state into the accessibility tree.
    let status: Binding<Str> = binding("Idle");
    let status_guard = webview.events().watch({
        let status = status.clone();
        move |context| {
            // No "nothing happened" or navigation-state variants to skip any
            // more: the signal is `Option`, and history state has its own signals.
            let Some(event) = context.into_value() else {
                return;
            };
            let label = match event {
                WebViewEvent::WillNavigate { .. } => "Navigating",
                WebViewEvent::Loading { .. } => "Loading",
                WebViewEvent::Loaded => "Loaded",
                WebViewEvent::Error(_) => "Error",
                WebViewEvent::Redirect { .. } => "Redirect",
            };
            status.set(Str::from_static(label));
        }
    });

    let can_go_back = webview.can_go_back();
    let can_go_forward = webview.can_go_forward();

    vstack((
        text!("Status: {status}"),
        text!("Navigation: back={can_go_back} forward={can_go_forward}"),
        button("Back")
            .action(move || webview_for_back.go_back())
            .disabled(webview.can_go_back().not()),
        button("Forward")
            .action(move || webview_for_forward.go_forward())
            .disabled(webview.can_go_forward().not()),
        button("Go API").action(move || webview_for_navigate.go_to(API_URL)),
        webview
            .a11y_role(AccessibilityRole::Group)
            .a11y_label("Docs WebView")
            .size(320.0, 260.0),
    ))
    .retain(status_guard)
}

#[test]
fn webview_exposes_accessibility_surface_and_navigation_state() {
    let controller = WebViewController::new(FakeWebViewController::default());
    let webview = controller.open();

    let mut env = Environment::new();
    install_m3(&mut env);
    env.insert(controller);
    let mut app = ui()
        .environment(env)
        .viewport(420, 420)
        .mount({
            let webview = webview.clone();
            move || webview_test_view(webview.clone())
        });

    // Navigate only once the view is mounted: the handle emits its events
    // synchronously, so a navigation started before mounting would complete
    // before anything is subscribed.
    webview.go_to(DOCS_URL);

    assert!(
        app.wait_for(
            &[app.expect_exists(Selector::default().label("Status: Loaded"))],
            WaitOptions::new(Duration::from_millis(250)),
        ) == WaitResult::Completed,
        "webview-exposes-accessibility-surface-and-navigation-state: expected initial navigation to finish"
    );

    app.query().label("Docs WebView").assert_exists();
    app.query()
        .label("Navigation: back=false forward=false")
        .assert_exists();
    assert!(
        !app.query()
            .role(Role::BUTTON)
            .label("Back")
            .single()
            .node()
            .enabled(),
        "webview-exposes-accessibility-surface-and-navigation-state: back should start disabled"
    );

    assert!(
        app.query().role(Role::BUTTON).label("Go API").tap(),
        "webview-exposes-accessibility-surface-and-navigation-state: go api tap should succeed"
    );
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default().label("Navigation: back=true forward=false"),
            )],
            WaitOptions::new(Duration::from_millis(250)),
        ) == WaitResult::Completed,
        "webview-exposes-accessibility-surface-and-navigation-state: expected forward history after navigating to API"
    );
    assert!(
        app.query()
            .role(Role::BUTTON)
            .label("Back")
            .single()
            .node()
            .enabled(),
        "webview-exposes-accessibility-surface-and-navigation-state: back should enable after second page"
    );

    assert!(
        app.query().role(Role::BUTTON).label("Back").tap(),
        "webview-exposes-accessibility-surface-and-navigation-state: back tap should succeed"
    );
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default().label("Navigation: back=false forward=true"),
            )],
            WaitOptions::new(Duration::from_millis(250)),
        ) == WaitResult::Completed,
        "webview-exposes-accessibility-surface-and-navigation-state: expected forward availability after going back"
    );
    assert!(
        app.query()
            .role(Role::BUTTON)
            .label("Forward")
            .single()
            .node()
            .enabled(),
        "webview-exposes-accessibility-surface-and-navigation-state: forward should enable after going back"
    );
}
