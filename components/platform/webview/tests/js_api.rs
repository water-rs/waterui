//! `#[js_api]` exposes an object's methods and state to the page.
#![expect(
    clippy::future_not_send,
    reason = "a web view and the API it serves live on the UI thread"
)]
#![expect(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "`async` is how `#[js_api]` is told a method is a handler rather than \
              state, whether or not a particular body has something to await"
)]

use std::cell::RefCell;
use std::future::{Future, ready};
use std::rc::Rc;

use waterui::webview::{
    BackendEvent, Cookie, CustomWebViewController, Json, ScriptInjectionTime, Url, WatcherGuard,
    WatcherSet, WebViewController, WebViewHandle, serde,
};
// `SignalExt` is deliberately *not* imported at file scope: nami implements
// `Signal` for plain values, so its `contains`/`map` would shadow the `str` and
// `Iterator` methods the assertions below rely on.
use waterui::{Binding, Computed, Environment, Signal, js_api};
use waterui_testing::ui;

/// Records what a web view was asked to do, so a test can read it back.
#[derive(Default, Clone)]
struct RecordingController {
    state: Rc<RefCell<Recorded>>,
}

#[derive(Default)]
struct Recorded {
    handlers: Vec<String>,
    injected: Vec<String>,
}

impl CustomWebViewController for RecordingController {
    fn open(&self) -> impl WebViewHandle {
        RecordingHandle {
            state: self.state.clone(),
            watchers: WatcherSet::new(),
        }
    }
}

#[derive(Clone)]
struct RecordingHandle {
    state: Rc<RefCell<Recorded>>,
    watchers: WatcherSet<BackendEvent>,
}

impl WebViewHandle for RecordingHandle {
    fn go_back(&self) {}
    fn go_forward(&self) {}
    fn go_to(&self, _url: &Url) {}
    fn stop(&self) {}
    fn refresh(&self) {}
    fn set_user_agent(&self, _user_agent: &str) {}
    fn can_go_back(&self) -> bool {
        false
    }
    fn can_go_forward(&self) -> bool {
        false
    }
    fn set_redirects_enabled(&self, _enabled: impl Signal<Output = bool>) {}
    fn set_bridge_origins(&self, _policy: waterui::webview::OriginPolicy) {}
    fn set_cookie(&self, _cookie: Cookie<'static>) {}

    fn inject_script(&self, script: &str, _time: ScriptInjectionTime) {
        self.state.borrow_mut().injected.push(script.to_owned());
    }

    fn add_handler(&self, name: &str, _handler: Box<waterui::webview::ScriptMessageHandler>) {
        self.state.borrow_mut().handlers.push(name.to_owned());
    }

    fn remove_handler(&self, _name: &str) {}

    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.watchers.insert(f)
    }

    fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> {
        ready(Vec::new())
    }

    fn run_javascript(
        &self,
        _script: &str,
    ) -> impl Future<Output = Result<waterui::Str, waterui::Str>> {
        ready(Ok(waterui::Str::from_static(r#"{"ok":true}"#)))
    }
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
#[serde(crate = "waterui::webview::serde")]
struct Greeting {
    text: String,
}

struct App {
    theme: Binding<String>,
    count: Binding<u32>,
}

#[js_api(namespace = "app")]
impl App {
    /// Mirrored state: a binding is writable from the page.
    fn theme(&self) -> Binding<String> {
        self.theme.clone()
    }

    /// A derived value the page may read but not assign to.
    fn doubled(&self) -> Computed<u32> {
        use waterui::SignalExt as _;
        self.count.clone().map(|count| count * 2).computed()
    }

    /// A handler the page calls as `await app.greet({ name: "Lexo" })`.
    async fn greet(&self, name: String) -> Json<Greeting> {
        Json(Greeting {
            text: format!("Hi {name}"),
        })
    }

    /// Renaming changes only what the page sees.
    #[js(rename = "reset")]
    async fn reset_counter(&self) {
        self.count.set(0);
    }

    /// Internal helpers stay internal.
    #[js(skip)]
    fn internal(&self, factor: u32) -> u32 {
        self.count.get() * factor
    }
}

#[test]
fn an_impl_block_becomes_the_surface_the_page_calls() {
    let controller = RecordingController::default();
    let recorded = controller.state.clone();

    // A skipped method stays an ordinary Rust method, arguments and all.
    let internal = App {
        theme: Binding::container(String::from("light")),
        count: Binding::container(2),
    };
    assert_eq!(internal.internal(3), 6);

    let mut env = Environment::new();
    env.insert(WebViewController::new(controller));
    let _app = ui().environment(env).viewport(320, 240).mount(move || {
        waterui::webview::WebView::open("https://app.waterui.dev").serve(App {
            theme: Binding::container(String::from("light")),
            count: Binding::container(1),
        })
    });

    let recorded = recorded.borrow();
    let handler = |wanted: &str| recorded.handlers.iter().any(|name| name == wanted);
    let any_handler_mentioning = |fragment: &str| {
        recorded
            .handlers
            .iter()
            .any(|name| str::contains(name, fragment))
    };

    // Async methods become handlers, under the namespace, honouring `rename`.
    assert!(handler("app.greet"), "{:?}", recorded.handlers);
    assert!(handler("app.reset"), "{:?}", recorded.handlers);
    assert!(
        !any_handler_mentioning("internal"),
        "#[js(skip)] must keep a method off the surface"
    );
    assert!(
        !any_handler_mentioning("reset_counter"),
        "a renamed method is exposed only under its new name"
    );

    // Exposed state is seeded into the document rather than pushed after load,
    // so the page reads correct values on its first line.
    let seed = recorded
        .injected
        .iter()
        .find(|script| str::contains(script, "__wateruiState"))
        .expect("state is seeded at document start");
    assert!(str::contains(seed, "app.theme"), "{seed}");
    assert!(str::contains(seed, "app.doubled"), "{seed}");
}
