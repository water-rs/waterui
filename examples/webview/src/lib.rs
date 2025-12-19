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
    let user_agent: Binding<Str> = binding("");

    let can_go_back = webview.can_go_back();
    let can_go_forward = webview.can_go_forward();

    webview.set_redirects_enabled(allow_redirects.get());

    let event_signal = WebView::event(&webview);
    let event_guard = {
        let status = status.clone();
        let progress_value = progress_value.clone();
        let address = address.clone();
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
            }
            WebViewEvent::Redirect { from, to } => {
                address.set(Str::from(to.to_string()));
                status.set(Str::from(format!("Redirect: {from} -> {to}")));
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
            button("Reload").action({
                let webview = webview.clone();
                move || webview.refresh()
            }),
            button("Stop").action({
                let webview = webview.clone();
                move || webview.stop()
            }),
        ))
        .spacing(8.0),
        hstack((
            Toggle::new(&allow_redirects).label(text("Allow redirects")),
            spacer(),
        )),
        hstack((
            TextField::new(&user_agent).prompt("Custom user agent"),
            button("Apply UA").style(ButtonStyle::Bordered).action({
                let webview = webview.clone();
                let user_agent = user_agent.clone();
                move || {
                    let ua = user_agent.get();
                    webview.set_user_agent(ua.as_str());
                }
            }),
        ))
        .spacing(8.0),
        hstack((
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
        ))
        .spacing(8.0),
        hstack((
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
    .spacing(10.0)
    .padding_with(12.0);

    vstack((toolbar, Divider, webview.clone()))
        .spacing(8.0)
        .on_change(&allow_redirects, move |enabled| {
            webview.set_redirects_enabled(enabled)
        })
        .retain(event_guard)
}

pub fn app(env: Environment) -> App {
    let controller: &WebViewController = env.get().expect("WebViewController not installed");
    let handle = controller.open();
    let webview = WebView::new(handle);

    App::new(main(webview), env)
}

waterui_ffi::export!();
