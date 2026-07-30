//! Advanced Chromium Example
//!
//! This example is intentionally separate from the standard `WebView` example.
//! It demonstrates the advanced Chromium library with:
//! - A visible GPU-composited Chromium page
//! - Browser lifecycle and navigation commands
//! - Raw and strongly typed Chrome `DevTools` Protocol commands
//! - Chromium screenshots
//! - A headless Chromium page controlled through CDP

use serde_json::{Value, json};
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui_chromium::cdp::browser_protocol::page::GetNavigationHistoryParams;
use waterui_chromium::{
    CdpError, ChromiumConfiguration, ChromiumController, ChromiumEvent, ChromiumPage,
    ScreenshotFormat,
};

const START_URL: &str = "https://waterui.dev";
const INPUT_PROBE_SCRIPT: &str = include_str!("input_probe.js");

fn evaluation_text(response: &Value) -> Option<String> {
    let value = response.get("result")?.get("value")?;
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Null => Some(String::from("null")),
        value => Some(value.to_string()),
    }
}

fn history_summary(current_index: i64, entry_count: usize) -> String {
    format!("Typed CDP: {entry_count} entries, current index {current_index}")
}

fn install_event_observer(
    page: &ChromiumPage,
    status: &Binding<Str>,
    progress_value: &Binding<f64>,
) {
    let status = status.clone();
    let progress_value = progress_value.clone();
    page.watch(move |event| match event {
        ChromiumEvent::Ready => {
            status.set(Str::from_static("Chromium ready; CDP attached"));
        }
        ChromiumEvent::NavigationStarted(url) => {
            status.set(Str::from(format!("Navigating to {url}")));
            progress_value.set(0.0);
        }
        ChromiumEvent::Loading(progress) => {
            progress_value.set(f64::from(progress));
        }
        ChromiumEvent::Loaded(url) => {
            status.set(Str::from(format!("Loaded {url}")));
            progress_value.set(1.0);
        }
        ChromiumEvent::RendererTerminated(reason) => {
            status.set(Str::from(format!("Renderer terminated: {reason}")));
            progress_value.set(0.0);
        }
        ChromiumEvent::Closed => {
            status.set(Str::from_static("Chromium page closed"));
            progress_value.set(0.0);
        }
    });
}

#[expect(
    clippy::future_not_send,
    reason = "Chromium pages and CDP sessions are confined to the WaterUI thread"
)]
async fn evaluate_page(
    page: &ChromiumPage,
    expression: &'static str,
) -> Result<Option<String>, CdpError> {
    page.cdp()
        .execute_raw(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true
            }),
        )
        .await
        .map(|response| evaluation_text(&response))
}

fn navigation_controls(page: &ChromiumPage) -> impl View + use<> {
    vstack((
        hstack((
            button("Back")
                .action(|State(page): State<ChromiumPage>| page.go_back())
                .state(page),
            button("Forward")
                .action(|State(page): State<ChromiumPage>| page.go_forward())
                .state(page),
        ))
        .spacing(8.0),
        hstack((
            button("Reload")
                .action(|State(page): State<ChromiumPage>| page.reload())
                .state(page),
            button("Stop")
                .action(|State(page): State<ChromiumPage>| page.stop())
                .state(page),
        ))
        .spacing(8.0),
    ))
    .spacing(8.0)
}

fn lifecycle_controls(status: &Binding<Str>, progress_value: &Binding<f64>) -> impl View + use<> {
    vstack((
        text("Lifecycle").headline(),
        text!("{status}").body(),
        progress(progress_value.clone()).label(text("Load progress").caption()),
    ))
    .spacing(6.0)
}

fn input_probe_controls(page: &ChromiumPage, result: &Binding<Str>) -> impl View + use<> {
    vstack((
        button("Prepare browser input probe")
            .style(ButtonStyle::Bordered)
            .action_async(
                |State(page): State<ChromiumPage>, State(result): State<Binding<Str>>| async move {
                    match evaluate_page(&page, INPUT_PROBE_SCRIPT).await {
                        Ok(Some(value)) if value == "ready" => {
                            result.set(Str::from_static("Browser input probe ready"));
                        }
                        Ok(Some(value)) => {
                            result.set(Str::from(format!("Unexpected input probe result: {value}")));
                        }
                        Ok(None) => result
                            .set(Str::from_static("Input probe response omitted result.value")),
                        Err(error) => result.set(Str::from(format!("Input probe error: {error}"))),
                    }
                },
            )
            .state(page)
            .state(result),
        button("Inspect focused browser element")
            .style(ButtonStyle::Bordered)
            .action_async(
                |State(page): State<ChromiumPage>, State(result): State<Binding<Str>>| async move {
                    match evaluate_page(
                        &page,
                        "JSON.stringify({tag:document.activeElement?.tagName??null,value:document.activeElement?.value??null,selectionStart:document.activeElement?.selectionStart??null,selectionEnd:document.activeElement?.selectionEnd??null,events:window.wateruiInputProbeEvents??[]})",
                    )
                    .await
                    {
                        Ok(Some(focus)) => {
                            result.set(Str::from(format!("Raw CDP focus: {focus}")));
                        }
                        Ok(None) => result
                            .set(Str::from_static("Raw CDP response omitted result.value")),
                        Err(error) => result.set(Str::from(format!("Raw CDP error: {error}"))),
                    }
                },
            )
            .state(page)
            .state(result),
    ))
    .spacing(8.0)
}

fn visible_cdp_controls(page: &ChromiumPage) -> impl View + use<> {
    let raw_result: Binding<Str> = binding("Raw CDP has not run");
    let typed_result: Binding<Str> = binding("Typed CDP has not run");
    let screenshot_result: Binding<Str> = binding("No screenshot captured");

    vstack((
        text("Visible page CDP").headline(),
        button("Read title with raw CDP")
            .style(ButtonStyle::Bordered)
            .action_async(
                |State(page): State<ChromiumPage>, State(result): State<Binding<Str>>| async move {
                    match evaluate_page(&page, "document.title").await {
                        Ok(title) => match title {
                            Some(title) => result.set(Str::from(format!("Raw CDP title: {title}"))),
                            None => result
                                .set(Str::from_static("Raw CDP response omitted result.value")),
                        },
                        Err(error) => result.set(Str::from(format!("Raw CDP error: {error}"))),
                    }
                },
            )
            .state(page)
            .state(&raw_result),
        input_probe_controls(page, &raw_result),
        text!("{raw_result}")
            .caption()
            .foreground(theme_color::MutedForeground),
        button("Read typed navigation history")
            .style(ButtonStyle::Bordered)
            .action_async(
                |State(page): State<ChromiumPage>, State(result): State<Binding<Str>>| async move {
                    match page
                        .cdp()
                        .execute(GetNavigationHistoryParams::default())
                        .await
                    {
                        Ok(history) => result.set(Str::from(history_summary(
                            history.current_index,
                            history.entries.len(),
                        ))),
                        Err(error) => result.set(Str::from(format!("Typed CDP error: {error}"))),
                    }
                },
            )
            .state(page)
            .state(&typed_result),
        text!("{typed_result}")
            .caption()
            .foreground(theme_color::MutedForeground),
        button("Capture Chromium PNG")
            .style(ButtonStyle::Bordered)
            .action_async(
                |State(page): State<ChromiumPage>, State(result): State<Binding<Str>>| async move {
                    match page.screenshot(ScreenshotFormat::Png).await {
                        Ok(bytes) => result.set(Str::from(format!(
                            "Chromium returned {} PNG bytes",
                            bytes.len()
                        ))),
                        Err(error) => result.set(Str::from(format!("Screenshot error: {error}"))),
                    }
                },
            )
            .state(page)
            .state(&screenshot_result),
        text!("{screenshot_result}")
            .caption()
            .foreground(theme_color::MutedForeground),
    ))
    .spacing(8.0)
}

#[expect(
    clippy::future_not_send,
    reason = "Chromium pages and CDP sessions are confined to the WaterUI thread"
)]
async fn load_headless_title(page: &ChromiumPage) -> Result<Value, CdpError> {
    let cdp = page.cdp();
    let lifecycle_events = cdp.events_raw("Page.lifecycleEvent");
    cdp.execute_raw("Page.enable", json!({})).await?;
    cdp.execute_raw("Page.setLifecycleEventsEnabled", json!({"enabled": true}))
        .await?;
    let navigation = cdp
        .execute_raw("Page.navigate", json!({"url": START_URL}))
        .await?;
    let loader_id = navigation
        .get("loaderId")
        .and_then(Value::as_str)
        .ok_or_else(|| CdpError::InvalidPayload {
            method: String::from("Page.navigate"),
            message: String::from("response is missing `loaderId`"),
        })?;

    loop {
        let event = lifecycle_events
            .recv()
            .await
            .map_err(|_| CdpError::Detached)?;
        let matches_loader =
            event.params.get("loaderId").and_then(Value::as_str) == Some(loader_id);
        let is_load = event.params.get("name").and_then(Value::as_str) == Some("load");
        if matches_loader && is_load {
            break;
        }
    }

    cdp.execute_raw(
        "Runtime.evaluate",
        json!({
            "expression": "document.title",
            "returnByValue": true
        }),
    )
    .await
}

fn headless_controls(controller: &ChromiumController) -> impl View + use<> {
    let headless_result: Binding<Str> = binding("Headless Chromium has not run");

    vstack((
        text("Headless page").headline(),
        button("Run headless CDP")
            .style(ButtonStyle::BorderedProminent)
            .action_async(
                |State(controller): State<ChromiumController>,
                 State(result): State<Binding<Str>>| async move {
                    result.set(Str::from_static("Starting headless Chromium"));
                    let page = match controller
                        .headless(ChromiumConfiguration::default().language("en-US"))
                        .await
                    {
                        Ok(page) => page,
                        Err(error) => {
                            result.set(Str::from(format!("Headless startup error: {error}")));
                            return;
                        }
                    };
                    result.set(Str::from_static(
                        "Headless Chromium attached; loading page with CDP",
                    ));

                    let response = load_headless_title(&page).await;
                    result.set(Str::from_static("Headless CDP complete; closing page"));
                    let close_result = page.close().await;

                    match (response, close_result) {
                        (Ok(response), Ok(())) => match evaluation_text(&response) {
                            Some(title) => result
                                .set(Str::from(format!("Headless title: {title}; page closed"))),
                            None => result.set(Str::from_static(
                                "Headless CDP response omitted result.value",
                            )),
                        },
                        (Err(error), _) => {
                            result.set(Str::from(format!("Headless CDP error: {error}")));
                        }
                        (_, Err(error)) => {
                            result.set(Str::from(format!("Headless close error: {error}")));
                        }
                    }
                },
            )
            .state(controller)
            .state(&headless_result),
        text!("{headless_result}")
            .caption()
            .foreground(theme_color::MutedForeground),
    ))
    .spacing(8.0)
}

fn controls(
    page: &ChromiumPage,
    controller: &ChromiumController,
    status: &Binding<Str>,
    progress_value: &Binding<f64>,
) -> impl View + use<> {
    let header = vstack((
        text("Chromium + CDP").title(),
        text("Independent advanced browser runtime")
            .caption()
            .foreground(theme_color::MutedForeground),
    ))
    .spacing(4.0);

    scroll(
        vstack((
            header,
            navigation_controls(page),
            lifecycle_controls(status, progress_value),
            Divider,
            visible_cdp_controls(page),
            Divider,
            headless_controls(controller),
        ))
        .spacing(10.0)
        .padding(),
    )
    .width(340.0)
}

fn scene(controller: &ChromiumController) -> impl View + use<> {
    let status: Binding<Str> = binding("Opening Chromium");
    let progress_value = Binding::f64(0.0);
    let chromium = controller.open(ChromiumConfiguration::new(START_URL).language("en-US"));
    let page = chromium.page().clone();
    install_event_observer(&page, &status, &progress_value);

    hstack((
        chromium,
        controls(&page, controller, &status, &progress_value),
    ))
}

#[derive(Debug)]
struct ChromiumDemo;

impl View for ChromiumDemo {
    fn body(self, env: &Environment) -> impl View {
        let controller = env
            .get::<ChromiumController>()
            .expect("the selected backend did not install the Chromium runtime");
        scene(controller)
    }
}

#[preview]
/// Builds the visible Chromium and CDP demonstration.
pub fn demo() -> impl View {
    ChromiumDemo
}

/// Creates the example application with the environment prepared by the backend.
pub fn app(env: Environment) -> App {
    App::new(demo, env)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{evaluation_text, history_summary};

    #[test]
    fn extracts_string_and_scalar_cdp_values() {
        assert_eq!(
            evaluation_text(&json!({"result": {"value": "WaterUI"}})).as_deref(),
            Some("WaterUI")
        );
        assert_eq!(
            evaluation_text(&json!({"result": {"value": 120}})).as_deref(),
            Some("120")
        );
    }

    #[test]
    fn rejects_cdp_response_without_returned_value() {
        assert_eq!(evaluation_text(&json!({"result": {}})), None);
    }

    #[test]
    fn formats_typed_navigation_history() {
        assert_eq!(
            history_summary(2, 4),
            "Typed CDP: 4 entries, current index 2"
        );
    }
}
