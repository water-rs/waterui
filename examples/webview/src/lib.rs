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
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::webview::{
    Json, ScriptInjectionTime, Url, WebView, WebViewController, WebViewEvent, WebViewProxy,
};

/// The payload the page exchanges with the `greet` handler.
#[derive(serde::Serialize, serde::Deserialize)]
struct Greeting {
    name: String,
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
        WebViewEvent::None => {
            status.set(Str::from_static("Idle"));
            progress_value.set(0.0);
        }
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
        WebViewEvent::StateChanged { .. } => {}
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

        let open = WebView::open("https://waterui.dev")
            .redirects_enabled(allow_redirects.clone())
            // Runs on every page load; the page can call back with
            // `waterui.invoke("logTitle", document.title)`.
            .inject(
                "document.documentElement.dataset.waterui = 'ready';",
                ScriptInjectionTime::DocumentEnd,
            )
            // `await waterui.invoke("greet", { name: "Lexo" })` in the page.
            // The closure reads exactly like a `Button::action`: take the
            // extractors you need, return anything `IntoJsReply`, and it may be
            // async.
            .handler("greet", |Json(request): Json<Greeting>| async move {
                Json(Greeting {
                    name: format!("Hi {}", request.name),
                })
            })
            .on_event({
                let status = status.clone();
                let progress_value = progress_value.clone();
                let address = address.clone();
                let allow_redirects = allow_redirects.clone();
                move |event| {
                    handle_webview_event(event, &status, &progress_value, &address, &allow_redirects)
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
