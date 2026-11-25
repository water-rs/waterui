//! `WebView` example.
//!
//! The whole web view is described before it exists: `WebView::open` returns a
//! builder that records the URL it follows, its redirect policy, the user agent,
//! injected scripts, handlers the page can call, and what to do with its events.
//! Nothing here reaches into the environment for a controller, and no
//! subscription guard has to be retained by hand.
//!
//! Controls drive the page through [`WebViewProxy`], extracted from the
//! environment exactly the way `State<T>` is.
//!
//! On the web the page is a cross-origin `<iframe>`, which an embedder can
//! navigate and nothing more: the history controls, the redirect policy, the
//! injected script, the page API and JavaScript evaluation are not compiled
//! there, so this example gates them the same way.

use waterui::app::{App, Windows};
#[cfg(not(target_arch = "wasm32"))]
use waterui::js_api;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::theme::color::{Foreground, MutedForeground};
#[cfg(not(target_arch = "wasm32"))]
use waterui::webview::{Json, ScriptInjectionTime, TsType, serde_json};
use waterui::webview::{Url, WebView, WebViewEvent, WebViewProxy};

#[cfg(not(target_arch = "wasm32"))]
/// What the `greet` handler answers with.
#[derive(serde::Serialize, serde::Deserialize, TsType)]
struct Greeting {
    text: String,
}

#[cfg(not(target_arch = "wasm32"))]
/// Everything the page is allowed to reach.
///
/// One object instead of a list of `.handler(...)` and `.expose(...)` calls,
/// each repeating a name as a string that nothing checks. `#[js_api]` writes
/// those calls from the method names.
struct PageApi {
    /// The same binding the toolbar's address field edits, so a write from
    /// either side moves both.
    address: Binding<Str>,
    greetings: Binding<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
#[js_api]
impl PageApi {
    /// Mirrored state, writable: once `waterui.ready` has resolved,
    /// `waterui.state.address = "https://..."` in the page moves the toolbar,
    /// because this is one binding with two views onto it.
    fn address(&self) -> Binding<Str> {
        self.address.clone()
    }

    /// Mirrored state, read-only: derived in Rust, so the page may read
    /// `waterui.state.greetings` and subscribe with `waterui.watch`, while
    /// assigning to it throws.
    fn greetings(&self) -> Computed<u32> {
        self.greetings.clone().computed()
    }

    /// A handler: `await waterui.invoke("greet", { name: "Lexo" })` resolves to
    /// `{ text: "Hi Lexo" }` — the object, not a string of it.
    ///
    /// `async` is how `#[js_api]` tells a handler from mirrored state.
    #[expect(
        clippy::future_not_send,
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "#[js_api] classifies a method as a handler by its async signature and dispatches on the UI thread"
    )]
    async fn greet(&self, name: String) -> Json<Greeting> {
        self.greetings.with_mut(|v| *v += 1);
        Json(Greeting {
            text: format!("Hi {name}"),
        })
    }
}

/// Applies one web view event to the UI state.
fn handle_webview_event(
    event: WebViewEvent,
    status: &Binding<Str>,
    address: &Binding<Str>,
    allow_redirects: &Binding<bool>,
) {
    match event {
        WebViewEvent::WillNavigate { url } => {
            address.set(Str::from(url.to_string()));
            status.set(Str::from(format!("Navigating to {url}")));
        }
        WebViewEvent::Loading { progress } => {
            status.set(Str::from(format!("Loading {:.0}%", progress * 100.0)));
        }
        WebViewEvent::Loaded => {
            status.set(Str::from_static("Loaded"));
        }
        WebViewEvent::Redirect { from, to } => {
            if allow_redirects.snapshot() {
                address.set(Str::from(to.to_string()));
                status.set(Str::from(format!("Redirect: {from} -> {to}")));
            } else {
                status.set(Str::from(format!("Redirect blocked: {from} -> {to}")));
            }
        }
        WebViewEvent::Error(err) => {
            status.set(Str::from(format!("Error: {err}")));
        }
        _ => {}
    }
}

/// The bindings the "Go" action reads and writes.
#[state]
#[derive(Clone)]
struct NavigateState {
    address: Binding<Str>,
    status: Binding<Str>,
}

/// The controls beside the page. Every action takes a [`WebViewProxy`], which the
/// surrounding `proxy_scope` supplies alongside live page state. `bridge` holds
/// the controls that need more of the page than navigation.
fn toolbar<B: View>(
    status: &Binding<Str>,
    address: &Binding<Str>,
    web: &WebViewProxy,
    bridge: B,
) -> impl View + use<B> {
    vstack((
        text("WebView Playground").title().foreground(Foreground),
        text(web.title()).caption().foreground(MutedForeground),
        hstack((
            TextField::new("Address", address).hide_label(),
            button("Go")
                .style(ButtonStyle::Bordered)
                .action(|proxy: WebViewProxy, nav: NavigateState| {
                    if let Some(url) = Url::parse_user_input(nav.address.snapshot().as_str()) {
                        nav.address.set(Str::from(url.as_str().to_owned()));
                        proxy.go_to(url);
                    } else {
                        nav.status.set(Str::from_static("Invalid URL"));
                    }
                })
                .state(&NavigateState {
                    address: address.clone(),
                    status: status.clone(),
                }),
        ))
        .spacing(8.0),
        bridge,
        vstack((
            text("Status:").caption().foreground(MutedForeground),
            text!("{status}").body().foreground(Foreground),
        ))
        .spacing(8.0),
        progress(web.progress()).label(text("Load progress").caption()),
    ))
    .spacing(5.0)
    .max_width(420.0)
}

/// History, the redirect policy and JavaScript evaluation: what a native web
/// view offers beyond navigation.
#[cfg(not(target_arch = "wasm32"))]
fn bridge_controls(allow_redirects: &Binding<bool>, web: &WebViewProxy) -> impl View + use<> {
    let js_result: Binding<Str> = binding(Str::from(""));
    vstack((
        hstack((
            button("Back")
                .action(|proxy: WebViewProxy| proxy.go_back())
                .disabled(web.can_go_back().map(|can| !can)),
            button("Forward")
                .action(|proxy: WebViewProxy| proxy.go_forward())
                .disabled(web.can_go_forward().map(|can| !can)),
            button("Reload").action(|proxy: WebViewProxy| proxy.refresh()),
            button("Stop").action(|proxy: WebViewProxy| proxy.stop()),
        ))
        .spacing(8.0),
        Toggle::new(text("Allow redirects"), allow_redirects),
        button("Get Title (JS)")
            .style(ButtonStyle::Bordered)
            .action_async(
                |proxy: WebViewProxy, State(result): State<Binding<Str>>| async move {
                    match proxy.run_javascript("document.title").await {
                        // The raw path answers JSON, so the title arrives as a
                        // quoted string.
                        Ok(title) => result.set(Str::from(
                            serde_json::from_str::<String>(&title)
                                .expect("document.title answers a JSON string"),
                        )),
                        Err(err) => result.set(Str::from(format!("JS error: {err}"))),
                    }
                },
            )
            .state(&js_result),
        hstack((
            text("JS Result:").caption().foreground(MutedForeground),
            text!("{js_result}").body().foreground(Foreground),
        ))
        .spacing(8.0),
    ))
    .spacing(5.0)
}

fn webview_playground() -> impl View {
    let status: Binding<Str> = binding(Str::from("Idle"));
    let address: Binding<Str> = binding(Str::from("https://waterui.dev"));
    let allow_redirects = Binding::new(false);

    let open = WebView::open("https://waterui.dev");
    #[cfg(not(target_arch = "wasm32"))]
    let open = open
        .redirects_enabled(allow_redirects.clone())
        // Runs on every page load; the page can call back with
        // `waterui.invoke("logTitle", document.title)`.
        .inject(
            "ready-marker",
            "document.documentElement.dataset.waterui = 'ready';",
            ScriptInjectionTime::DocumentEnd,
        )
        // Hands the page the whole surface at once: `greet` as a handler,
        // `address` and `greetings` as mirrored state. `.handler(...)` and
        // `.expose(...)` still exist for one-off cases — this is what they
        // look like collected onto a type.
        .serve(PageApi {
            address: address.clone(),
            greetings: binding(0_u32),
        });
    let open = open.on_event({
        let status = status.clone();
        let address = address.clone();
        let allow_redirects = allow_redirects.clone();
        move |event| {
            handle_webview_event(event, &status, &address, &allow_redirects);
        }
    });

    open.proxy_scope(move |page, web| {
        #[cfg(not(target_arch = "wasm32"))]
        let bridge = bridge_controls(&allow_redirects, &web);
        #[cfg(target_arch = "wasm32")]
        let bridge = ();
        vstack((toolbar(&status, &address, &web, bridge), page))
    })
}

/// The web view demo: a playground page with navigation, history and JavaScript controls.
#[preview]
pub fn demo() -> impl View {
    webview_playground()
}

#[cfg(all(target_os = "linux", feature = "wpe"))]
use waterui_browser_wpe as _;

/// Builds the example with whichever engine this platform's `WebView` needs.
///
/// Selecting the browser engine is the application's dependency choice, not
/// view code's: Apple, Android, and GTK bridge their platform web engine, and
/// Linux without `WebKitGTK` enables the `wpe` feature linking
/// `waterui-browser-wpe` so the backend selects WPE at compile time.
#[must_use]
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(env).window(Window::new("", demo))
}
