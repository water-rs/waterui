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
use waterui::reactive::binding;
use waterui::task::spawn_local;
use waterui::webview::{ScriptInjectionTime, WebView, WebViewController, WebViewEvent};

fn main(webview: WebView) -> impl View {
    let address: Binding<Str> = binding("https://waterui.dev");
    let status: Binding<Str> = binding("Idle");
    let progress_value: Binding<f64> = binding(0.0);
    let js_result: Binding<Str> = binding("");
    let allow_redirects: Binding<bool> = binding(false);
    let system_user_agent: Binding<Str> = binding("");
    let custom_user_agent: Binding<Str> = binding("");

    let can_go_back = webview.can_go_back();
    let can_go_forward = webview.can_go_forward();

    webview.set_redirects_enabled(allow_redirects.get());

    let event_signal = WebView::event(&webview);
    let event_guard = {
        let status = status.clone();
        let progress_value = progress_value.clone();
        let address = address.clone();
        let allow_redirects = allow_redirects.clone();
        let system_user_agent = system_user_agent.clone();
        let webview = webview.clone();
        event_signal.watch(move |ctx| match ctx.into_value() {
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
                    if let Ok(url) = webview.run_javascript("location.href").await {
                        if !url.as_str().is_empty() && url.as_str() != "null" {
                            address.set(url);
                        }
                    }
                    if system_user_agent.get().as_str().is_empty() {
                        if let Ok(ua) = webview.run_javascript("navigator.userAgent").await {
                            if !ua.as_str().is_empty() && ua.as_str() != "null" {
                                system_user_agent.set(ua);
                            }
                        }
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
        })
    };

    webview.go_to(address.get().as_str());

    let toolbar = vstack((
        text("WebView Playground").size(22.0).bold(),
        hstack((
            TextField::new(&address),
            button("Go").style(ButtonStyle::Bordered).action({
                let webview = webview.clone();
                let address = address.clone();
                move || {
                    let url = address.get();
                    webview.go_to(url.as_str());
                }
            }),
        ))
        .spacing(8.0),
        hstack((
            button("Back").action({
                let webview = webview.clone();
                let can_go_back = can_go_back.clone();
                move || {
                    if can_go_back.get() {
                        webview.go_back();
                    }
                }
            }),
            button("Forward").action({
                let webview = webview.clone();
                let can_go_forward = can_go_forward.clone();
                move || {
                    if can_go_forward.get() {
                        webview.go_forward();
                    }
                }
            }),
        )),
        hstack((
            button("Reload").action({
                let webview = webview.clone();
                move || webview.refresh()
            }),
            button("Stop").action({
                let webview = webview.clone();
                move || webview.stop()
            }),
        )),
        Toggle::new(&allow_redirects).label(text("Allow redirects")),
        text!(
            "System User Agent: {system_user_agent}",
            system_user_agent = system_user_agent.get() // Wrong use of macro, but works for display
        ),
        vstack((
            TextField::new(&custom_user_agent).prompt("Custom user agent (optional)"),
            button("Apply Custom UA")
                .style(ButtonStyle::Bordered)
                .action({
                    let webview = webview.clone();
                    let custom_user_agent = custom_user_agent.clone();
                    move || {
                        let ua = custom_user_agent.get();
                        if ua.as_str().trim().is_empty() {
                            webview.set_user_agent("");
                        } else {
                            webview.set_user_agent(ua.as_str());
                        }
                    }
                }),
            button("Reset UA").style(ButtonStyle::Bordered).action({
                let webview = webview.clone();
                move || webview.set_user_agent("")
            }),
        ))
        .spacing(8.0),
        vstack((
            button("Inject JS").action({
                let webview = webview.clone();
                move || {
                    webview.inject_script(
                        r#"document.documentElement.style.outline = "3px solid #22c55e";"#,
                        ScriptInjectionTime::DocumentEnd,
                    );
                }
            }),
            button("Get Title (JS)")
                .style(ButtonStyle::Bordered)
                .action({
                    let webview = webview.clone();
                    let js_result = js_result.clone();
                    move || {
                        let webview = webview.clone();
                        let js_result = js_result.clone();
                        spawn_local(async move {
                            match webview.run_javascript("document.title").await {
                                Ok(result) => js_result.set(result),
                                Err(err) => js_result.set(Str::from(format!("JS error: {err}"))),
                            }
                        })
                        .detach();
                    }
                }),
        )),
        vstack((
            text("Status:"),
            Text::new(status.clone()),
            spacer(),
            text("Back:"),
            Text::display(can_go_back.clone()),
            text("Forward:"),
            Text::display(can_go_forward.clone()),
        ))
        .spacing(8.0),
        progress(progress_value.clone()).label(text("Load progress")),
        hstack((text("JS Result:"), Text::new(js_result.clone()))).spacing(8.0),
    ))
    .spacing(5.0)
    .width(250.0);

    hstack((webview.clone(), toolbar))
        .on_change(&allow_redirects, move |enabled| {
            webview.set_redirects_enabled(enabled)
        })
        .retain(event_guard)
}

fn missing_controller_view() -> impl View {
    vstack((
        text("WebView not available on this backend.").size(18.0).bold(),
        text("The native runtime did not install a WebViewController."),
        text("Run this example on a backend with WebView support."),
    ))
    .spacing(8.0)
    .padding()
}

pub fn app(env: Environment) -> App {
    let Some(controller) = env.get::<WebViewController>().cloned() else {
        return App::new(missing_controller_view(), env);
    };
    let handle = controller.open();
    let webview = WebView::new(handle);

    App::new(main(webview), env)
}

waterui_ffi::export!();
