//! WebView example.
//!
//! The whole web view is described before it exists: `WebView::open` returns a
//! builder that records the URL it follows, its redirect policy, the user agent,
//! injected scripts, handlers the page can call, and what to do with its events.
//! Nothing here reaches into the environment for a controller, and no
//! subscription guard has to be retained by hand.
//!
//! Controls drive the page through [`WebViewProxy`], extracted from the
//! environment exactly the way `State<T>` is.

use waterui::app::App;
use waterui::js_api;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::webview::{
    Json, ScriptInjectionTime, Url, WebView, WebViewController, WebViewEvent, WebViewProxy,
};

/// What the `greet` handler answers with.
#[derive(serde::Serialize, serde::Deserialize)]
struct Greeting {
    text: String,
}

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

#[js_api]
impl PageApi {
    /// Mirrored state, writable: `waterui.state.address = "https://..."` in the
    /// page moves the toolbar, because this is one binding with two views onto
    /// it.
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
    async fn greet(&self, name: String) -> Json<Greeting> {
        self.greetings.set(self.greetings.get() + 1);
        Json(Greeting {
            text: format!("Hi {name}"),
        })
    }
}

/// Applies one web view event to the UI state.
fn handle_webview_event(
    event: WebViewEvent,
    status: &Binding<Str>,
    progress_value: &Binding<f64>,
    address: &Binding<Str>,
    allow_redirects: &Binding<bool>,
) {
    match event {
        WebViewEvent::WillNavigate { url } => {
            address.set(Str::from(url.to_string()));
            status.set(Str::from(format!("Navigating to {url}")));
            progress_value.set(0.0);
        }
        WebViewEvent::Loading { progress } => {
            progress_value.set(f64::from(progress));
            status.set(Str::from(format!("Loading {:.0}%", progress * 100.0)));
        }
        WebViewEvent::Loaded => {
            status.set(Str::from_static("Loaded"));
            progress_value.set(1.0);
        }
        WebViewEvent::Redirect { from, to } => {
            if allow_redirects.get() {
                address.set(Str::from(to.to_string()));
                status.set(Str::from(format!("Redirect: {from} -> {to}")));
            } else {
                progress_value.set(0.0);
                status.set(Str::from(format!("Redirect blocked: {from} -> {to}")));
            }
        }
        WebViewEvent::Error(err) => {
            status.set(Str::from(format!("Error: {err}")));
        }
    }
}

/// The controls beside the page. Every action takes a [`WebViewProxy`], which the
/// surrounding `with_proxy` scope supplies.
fn toolbar(
    status: Binding<Str>,
    progress_value: Binding<f64>,
    address: Binding<Str>,
    allow_redirects: Binding<bool>,
    js_result: Binding<Str>,
) -> impl View {
    vstack((
        text("WebView Playground")
            .title()
            .foreground(theme_color::Foreground),
        hstack((
            TextField::new(&address),
            button("Go")
                .style(ButtonStyle::Bordered)
                .action(
                    |proxy: WebViewProxy,
                     State(addr): State<Binding<Str>>,
                     State(status): State<Binding<Str>>| {
                        if let Some(url) = Url::parse_user_input(addr.get().as_str()) {
                            addr.set(Str::from(url.as_str().to_owned()));
                            proxy.go_to(url);
                        } else {
                            status.set(Str::from_static("Invalid URL"));
                        }
                    },
                )
                .state(&address)
                .state(&status),
        ))
        .spacing(8.0),
        hstack((
            button("Back").action(|proxy: WebViewProxy| proxy.go_back()),
            button("Forward").action(|proxy: WebViewProxy| proxy.go_forward()),
            button("Reload").action(|proxy: WebViewProxy| proxy.refresh()),
            button("Stop").action(|proxy: WebViewProxy| proxy.stop()),
        ))
        .spacing(8.0),
        Toggle::new(&allow_redirects).label(text("Allow redirects")),
        button("Get Title (JS)")
            .style(ButtonStyle::Bordered)
            .action_async(
                |proxy: WebViewProxy, State(result): State<Binding<Str>>| async move {
                    match proxy.run_javascript("document.title").await {
                        Ok(title) => result.set(title),
                        Err(err) => result.set(Str::from(format!("JS error: {err}"))),
                    }
                },
            )
            .state(&js_result),
        vstack((
            text("Status:")
                .caption()
                .foreground(theme_color::MutedForeground),
            text!("{status}").body().foreground(theme_color::Foreground),
        ))
        .spacing(8.0),
        progress(progress_value.clone()).label(text("Load progress").caption()),
        hstack((
            text("JS Result:")
                .caption()
                .foreground(theme_color::MutedForeground),
            text!("{js_result}")
                .body()
                .foreground(theme_color::Foreground),
        ))
        .spacing(8.0),
    ))
    .spacing(5.0)
    .width(250.0)
}

fn missing_controller_view() -> impl View {
    vstack((
        text("WebView not available on this backend.").title(),
        "The native runtime did not install a WebViewController.",
        "Run this example on a backend with WebView support.",
    ))
    .spacing(8.0)
    .padding()
}

#[derive(Debug)]
struct WebViewDemo;

impl View for WebViewDemo {
    fn body(self, env: &Environment) -> impl View {
        if env.get::<WebViewController>().is_none() {
            return AnyView::new(missing_controller_view());
        }

        let status: Binding<Str> = binding("Idle");
        let progress_value = Binding::f64(0.0);
        let address: Binding<Str> = binding("https://waterui.dev");
        let allow_redirects = Binding::bool(false);
        let js_result: Binding<Str> = binding("");
        let greetings: Binding<u32> = binding(0_u32);

        let open = WebView::open("https://waterui.dev")
            .redirects_enabled(allow_redirects.clone())
            // Runs on every page load; the page can call back with
            // `waterui.invoke("logTitle", document.title)`.
            .inject(
                "document.documentElement.dataset.waterui = 'ready';",
                ScriptInjectionTime::DocumentEnd,
            )
            // Hands the page the whole surface at once: `greet` as a handler,
            // `address` and `greetings` as mirrored state. `.handler(...)` and
            // `.expose(...)` still exist for one-off cases — this is what they
            // look like collected onto a type.
            .serve(PageApi {
                address: address.clone(),
                greetings: greetings.clone(),
            })
            .on_event({
                let status = status.clone();
                let progress_value = progress_value.clone();
                let address = address.clone();
                let allow_redirects = allow_redirects.clone();
                move |event| {
                    handle_webview_event(
                        event,
                        &status,
                        &progress_value,
                        &address,
                        &allow_redirects,
                    )
                }
            });

        AnyView::new(open.with_proxy(move || {
            toolbar(status, progress_value, address, allow_redirects, js_result)
        }))
    }
}

#[preview]
pub fn demo() -> impl View {
    WebViewDemo
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
