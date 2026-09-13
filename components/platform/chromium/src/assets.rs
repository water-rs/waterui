//! The local asset origin a Chromium page serves over CDP `Fetch` interception.
//!
//! Chromium answers a secure context only for `https`, so the asset origin is
//! `https://waterui.localhost`: the `.localhost` name is RFC 6761 — it never
//! resolves off-box, so a missed interception cannot leak a request to a
//! network. Every request the pattern pauses is answered by
//! [`waterui_webview::assets::dispatch`], the same entry point every engine
//! routes through, so the GET/HEAD/traversal rules are enforced once.

use base64::Engine as _;
use serde_json::{Value, json};
use waterui_url::Url;
use waterui_webview::AssetServer;
use waterui_webview::assets::{self, ASSET_HTTPS_ORIGIN};

use crate::{CdpSession, ChromiumPage};

/// The origin a Chromium page serves bundled assets under —
/// `https://waterui.localhost`.
#[must_use]
pub const fn asset_origin() -> Url {
    Url::new(ASSET_HTTPS_ORIGIN)
}

/// Arms `Fetch` interception so `server` answers the page's asset origin.
///
/// The subscription is opened before `Fetch.enable` goes out so a paused
/// request can never arrive before anyone is listening. The task lives as long
/// as the page's `DevTools` session: when the page closes the receiver's
/// channel goes away and the loop ends.
pub fn intercept(page: &ChromiumPage, server: AssetServer) {
    let cdp = page.cdp().clone();
    executor_core::spawn_local(serve(cdp, server)).detach();
}

#[expect(
    clippy::future_not_send,
    reason = "CEF DevTools sessions are bound to the browser UI thread"
)]
async fn serve(cdp: CdpSession, server: AssetServer) {
    let paused = cdp.events_raw("Fetch.requestPaused");
    // `requestStage: Request` pauses before the network stack runs, so the
    // server sees the request rather than its response.
    if let Err(error) = cdp
        .execute_raw(
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": format!("{ASSET_HTTPS_ORIGIN}/*"),
                    "requestStage": "Request",
                }],
            }),
        )
        .await
    {
        tracing::error!(%error, "Chromium refused Fetch.enable; the asset origin cannot resolve");
        return;
    }
    while let Ok(event) = paused.recv().await {
        answer(&cdp, &server, &event.params).await;
    }
}

/// Answers one paused request: the server through the asset origin, the network
/// stack for anything else the pattern let through.
#[expect(
    clippy::future_not_send,
    reason = "CEF DevTools sessions are bound to the browser UI thread"
)]
async fn answer(cdp: &CdpSession, server: &AssetServer, params: &Value) {
    let Some(request_id) = params.get("requestId").and_then(Value::as_str) else {
        tracing::warn!("Chromium emitted Fetch.requestPaused without a requestId; ignoring");
        return;
    };
    let request = params.get("request").unwrap_or(&Value::Null);
    let url = request.get("url").and_then(Value::as_str).unwrap_or_default();
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some((path, query)) = assets::asset_target(url, ASSET_HTTPS_ORIGIN) else {
        continue_request(cdp, request_id).await;
        return;
    };
    let response = assets::dispatch(server, method, path, query);
    let headers: Vec<Value> = response
        .headers
        .iter()
        .map(|(name, value)| json!({"name": name.as_str(), "value": value.as_str()}))
        .collect();
    let result = cdp
        .execute_raw(
            "Fetch.fulfillRequest",
            json!({
                "requestId": request_id,
                "responseCode": response.status,
                "responseHeaders": headers,
                "body": base64::engine::general_purpose::STANDARD.encode(&response.body),
            }),
        )
        .await;
    if let Err(error) = result {
        tracing::warn!(%error, url, "Chromium refused Fetch.fulfillRequest for an asset request");
    }
}

#[expect(
    clippy::future_not_send,
    reason = "CEF DevTools sessions are bound to the browser UI thread"
)]
async fn continue_request(cdp: &CdpSession, request_id: &str) {
    let result = cdp
        .execute_raw(
            "Fetch.continueRequest",
            json!({ "requestId": request_id }),
        )
        .await;
    if let Err(error) = result {
        tracing::warn!(%error, "Chromium refused Fetch.continueRequest for a paused request");
    }
}
