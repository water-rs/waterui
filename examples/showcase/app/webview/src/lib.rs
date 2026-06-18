//! WebView Example - Demonstrates WaterUI's WebView component
//!
//! This example showcases:
//! - Opening a WebView via the controller injected into the Environment
//! - Navigation controls (back/forward/refresh/stop)
//! - URL bar updates from WebView events
//! - Loading progress and status
//! - JavaScript injection and execution
//! - Redirect toggle (opt-in)

use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::task::spawn_local;
use waterui::webview::{ScriptInjectionTime, WebView, WebViewController, WebViewEvent};

fn normalize_address_input(input: &str) -> Option<Str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if waterui::webview::Url::parse(trimmed).is_some() {
        return Some(Str::from(trimmed.to_owned()));
    }

    if !trimmed.contains("://") {
        let with_https = format!("https://{trimmed}");
        if waterui::webview::Url::parse(&with_https).is_some() {
            return Some(Str::from(with_https));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::normalize_address_input;

    #[test]
    fn accepts_absolute_url() {
        let normalized = normalize_address_input("https://waterui.dev");
        assert_eq!(
            normalized.as_ref().map(|s| s.as_str()),
            Some("https://waterui.dev")
        );
    }

    #[test]
    fn prefixes_https_for_host_only_input() {
        let normalized = normalize_address_input("waterui.dev/docs");
        assert_eq!(
            normalized.as_ref().map(|s| s.as_str()),
            Some("https://waterui.dev/docs")
        );
    }

    #[test]
    fn rejects_empty_or_whitespace() {
        assert!(normalize_address_input("").is_none());
        assert!(normalize_address_input("   ").is_none());
    }
}

/// Handles WebView events and updates UI state accordingly.
fn handle_webview_event(
    event: WebViewEvent,
    webview: &WebView,
    status: &Binding<Str>,
    progress_value: &Binding<f64>,
    address: &Binding<Str>,
    allow_redirects: &Binding<bool>,
    system_user_agent: &Binding<Str>,
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
            progress_value.set(progress as f64);
            status.set(Str::from(format!("Loading {:.0}%", progress * 100.0)));
        }
        WebViewEvent::Loaded => {
            status.set(Str::from_static("Loaded"));
            progress_value.set(1.0);
            let webview = webview.clone();
            let system_user_agent = system_user_agent.clone();
            let address = address.clone();
            spawn_local(async move {
                if let Ok(url) = webview.run_javascript("location.href").await
                    && !url.as_str().is_empty() && url.as_str() != "null" {
                        address.set(url);
                    }
                if system_user_agent.get().as_str().is_empty()
                    && let Ok(ua) = webview.run_javascript("navigator.userAgent").await
                        && !ua.as_str().is_empty() && ua.as_str() != "null" {
                            system_user_agent.set(ua);
                        }
            })
            .detach();
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

fn scene(webview: WebView) -> impl View {
    let address: Binding<Str> = binding("https://waterui.dev");
    let status: Binding<Str> = binding("Idle");
    let progress_value = Binding::f64(0.0);
    let js_result: Binding<Str> = binding("");
    let allow_redirects = Binding::bool(false);
    let system_user_agent: Binding<Str> = binding("");
    let custom_user_agent: Binding<Str> = binding("");

    // Configure webview with reactive redirect setting
    let webview = webview.redirects_enabled(allow_redirects.clone());

    let can_go_back = webview.can_go_back();
    let can_go_forward = webview.can_go_forward();

    if let Some(normalized) = normalize_address_input(address.get().as_str()) {
        address.set(normalized.clone());
        webview.go_to(normalized.as_str());
    } else {
        status.set(Str::from_static("Invalid URL"));
    }

    let toolbar = vstack((
        text("WebView Playground")
            .title()
            .foreground(theme_color::Foreground),
        hstack((
            TextField::new(&address),
            button("Go")
                .style(ButtonStyle::Bordered)
                .action(
                    |State(wv): State<WebView>,
                     State(addr): State<Binding<Str>>,
                     State(status): State<Binding<Str>>| {
                        if let Some(normalized) = normalize_address_input(addr.get().as_str()) {
                            addr.set(normalized.clone());
                            wv.go_to(normalized.as_str());
                        } else {
                            status.set(Str::from_static("Invalid URL"));
                        }
                    },
                )
                .state(&webview)
                .state(&address)
                .state(&status),
        ))
        .spacing(8.0),
        hstack((
            button("Back")
                .action(|State(wv): State<WebView>| wv.go_back())
                .state(&webview)
                .disabled(can_go_back.clone().map(|b| !b)),
            button("Forward")
                .action(|State(wv): State<WebView>| wv.go_forward())
                .state(&webview)
                .disabled(can_go_forward.clone().map(|b| !b)),
        )),
        hstack((
            button("Reload")
                .action(|State(wv): State<WebView>| wv.refresh())
                .state(&webview),
            button("Stop")
                .action(|State(wv): State<WebView>| wv.stop())
                .state(&webview),
        )),
        Toggle::new(&allow_redirects).label(text("Allow redirects")),
        text!("System User Agent: {system_user_agent}")
            .font(font::Caption)
            .foreground(theme_color::MutedForeground),
        vstack((
            TextField::new(&custom_user_agent).prompt("Custom user agent (optional)"),
            button("Apply Custom UA")
                .style(ButtonStyle::Bordered)
                .action(
                    |State(wv): State<WebView>, State(ua): State<Binding<Str>>| {
                        let user_agent = ua.get();
                        if user_agent.as_str().trim().is_empty() {
                            wv.set_user_agent("");
                        } else {
                            wv.set_user_agent(user_agent.as_str());
                        }
                    },
                )
                .state(&webview)
                .state(&custom_user_agent),
            button("Reset UA")
                .style(ButtonStyle::Bordered)
                .action(|State(wv): State<WebView>| wv.set_user_agent(""))
                .state(&webview),
        ))
        .spacing(8.0),
        vstack((
            button("Inject JS")
                .action(|State(wv): State<WebView>| {
                    wv.inject_script(
                        r#"document.documentElement.style.outline = "3px solid #22c55e";"#,
                        ScriptInjectionTime::DocumentEnd,
                    );
                })
                .state(&webview),
            button("Get Title (JS)")
                .style(ButtonStyle::Bordered)
                .action_async(
                    |State(wv): State<WebView>, State(result): State<Binding<Str>>| async move {
                        match wv.run_javascript("document.title").await {
                            Ok(title) => result.set(title),
                            Err(err) => result.set(Str::from(format!("JS error: {err}"))),
                        }
                    },
                )
                .state(&webview)
                .state(&js_result),
        )),
        vstack((
            text("Status:")
                .caption()
                .foreground(theme_color::MutedForeground),
            text!("{status}").body().foreground(theme_color::Foreground),
            spacer(),
            text("Back:")
                .caption()
                .foreground(theme_color::MutedForeground),
            text!("{can_go_back}")
                .body()
                .foreground(theme_color::Foreground),
            text("Forward:")
                .caption()
                .foreground(theme_color::MutedForeground),
            text!("{can_go_forward}")
                .body()
                .foreground(theme_color::Foreground),
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
    .width(250.0);

    let event_signal = WebView::event(&webview);
    let event_guard = {
        let webview = webview.clone();
        event_signal.watch(move |ctx| {
            handle_webview_event(
                ctx.into_value(),
                &webview,
                &status,
                &progress_value,
                &address,
                &allow_redirects,
                &system_user_agent,
            );
        })
    };

    hstack((webview, toolbar)).retain(event_guard)
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

/// Self-contained, environment-aware entry. Resolves the [`WebViewController`]
/// from the environment at body time: opens a live WebView when a native
/// controller is installed, otherwise renders the unavailable placeholder. This
/// lets the example embed anywhere (gallery, `water preview`) without an
/// app-level controller hand-off.
#[derive(Debug)]
struct WebViewDemo;

impl View for WebViewDemo {
    fn body(self, env: &Environment) -> impl View {
        match env.get::<WebViewController>().cloned() {
            Some(controller) => AnyView::new(scene(controller.open())),
            None => AnyView::new(missing_controller_view()),
        }
    }
}

#[preview]
pub fn demo() -> impl View {
    WebViewDemo
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
