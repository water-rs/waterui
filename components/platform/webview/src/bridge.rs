//! The JavaScript bridge shared by every `WebView` backend.
//!
//! Four backends each carried a near-identical copy of the bridge script with a
//! subtly different wire format, and two more (CEF, WPE) exposed a different
//! page-facing API entirely. A snippet written against one backend could not run
//! on another, which defeats the purpose of having one `WebView` component.
//!
//! This module owns the single script and the single envelope. A backend supplies
//! only a transport: deliver [`SCRIPT`] at document start, hand
//! [`Request::parse`] whatever `__wateruiSend` produced, and complete the call
//! with [`Reply::resolve_script`], [`Reply::to_json`] or [`Reply::message`].

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use waterui_core::contract_violation;

use crate::message::JsReply;

/// What the page's promise rejects with when a [`Reply::Json`] holds bytes
/// that are not JSON.
const INVALID_JSON_REPLY: &str = "the handler's reply was not valid JSON";

/// The bridge script, injected at document start by every backend.
pub const SCRIPT: &str = include_str!("js/bridge.js");

/// The global the bridge calls to hand a request to the native side.
///
/// A backend installs a function under this name — a `WebKit` message handler, an
/// Android `@JavascriptInterface` method, a CDP binding — taking one string.
pub const SEND_FUNCTION: &str = "__wateruiSend";

/// The [`SEND_FUNCTION`] transport for a `WebKit` web view, injected at
/// document start ahead of [`SCRIPT`], which calls it.
///
/// It posts each envelope to the `__wateruiSend` handler a backend registers
/// with `addScriptMessageHandlerWithReply:contentWorld:name:` in the page
/// world, and settles the call from the promise `postMessage` returns. That
/// promise is `WebKit`'s reply channel, bound to the sending document, so the
/// backend answers with [`Reply::to_json`] and never evaluates a reply.
pub const WEBKIT_TRANSPORT: &str = include_str!("js/webkit_transport.js");

/// One `waterui.invoke(...)` call, as parsed from the page.
///
/// Every field is validated: this arrives from page script, which may be
/// untrusted, so a malformed request is rejected rather than fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Request {
    /// Correlates the reply with the pending promise in the page.
    id: u64,
    /// The handler name the page asked for.
    name: String,
    /// The payload, already decoded to bytes.
    payload: Vec<u8>,
    /// Whether the payload arrived as binary rather than JSON.
    is_binary: bool,
}

impl Request {
    /// Correlates the reply with the pending promise in the page.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The handler name the page asked for.
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The payload, already decoded to bytes.
    #[must_use]
    pub const fn payload(&self) -> &[u8] {
        self.payload.as_slice()
    }

    /// Whether the payload arrived as binary rather than JSON.
    #[must_use]
    pub const fn is_binary(&self) -> bool {
        self.is_binary
    }
}

#[derive(Debug, Deserialize)]
struct RawRequest {
    id: u64,
    name: String,
    #[serde(default)]
    json: Option<serde_json::Value>,
    #[serde(default)]
    b64: Option<String>,
}

/// Why a request from the page could not be understood.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RequestError {
    /// The envelope was not the JSON object the bridge produces.
    #[error("malformed WaterUI bridge envelope: {0}")]
    Envelope(#[from] serde_json::Error),
    /// The binary payload was not valid base64.
    #[error("malformed WaterUI bridge payload: {0}")]
    Payload(#[from] base64::DecodeError),
}

impl Request {
    /// Parses the envelope a backend received from [`SEND_FUNCTION`].
    ///
    /// # Errors
    ///
    /// Returns an error when the envelope is not well-formed. Reject the call
    /// with [`Reply::failure`] rather than treating it as fatal: page script can
    /// reach the transport directly.
    pub fn parse(envelope: &str) -> Result<Self, RequestError> {
        let raw: RawRequest = serde_json::from_str(envelope)?;
        let (payload, is_binary) = match (raw.json, raw.b64) {
            (_, Some(encoded)) => (STANDARD.decode(encoded)?, true),
            (Some(value), None) => (serde_json::to_vec(&value)?, false),
            (None, None) => (Vec::new(), false),
        };
        Ok(Self {
            id: raw.id,
            name: raw.name,
            payload,
            is_binary,
        })
    }
}

/// The result of a handler, ready to be delivered back to the page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reply {
    /// A serialized JSON value, delivered as JSON so the page's promise
    /// resolves with the value the handler returned.
    Json(Vec<u8>),
    /// Bytes the handler produced, delivered as base64.
    Bytes(Vec<u8>),
    /// The call failed; the page's promise rejects with this message.
    Failure(String),
}

impl From<JsReply> for Reply {
    fn from(reply: JsReply) -> Self {
        match reply {
            JsReply::Json(json) => Self::Json(json),
            JsReply::Bytes(bytes) => Self::Bytes(bytes),
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum ReplyPayload<'a> {
    /// Held as a `RawValue` so the handler's own bytes go into the envelope
    /// unchanged, rather than being parsed into a `Value` and printed again.
    Json {
        json: &'a RawValue,
    },
    Bytes {
        b64: String,
    },
    Failure {
        message: &'a str,
    },
}

#[derive(Serialize)]
struct ReplyEnvelope<'a> {
    /// The request id, for a channel that does not correlate the reply with
    /// its request itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    ok: bool,
    payload: ReplyPayload<'a>,
}

impl Reply {
    /// Builds a failure reply.
    pub fn failure(message: &(impl ToString + ?Sized)) -> Self {
        Self::Failure(message.to_string())
    }

    /// Renders the JavaScript that completes the pending call.
    ///
    /// Both arguments are serialized rather than interpolated, so a handler's
    /// bytes or an error message can never be read as source.
    ///
    /// # Panics
    ///
    /// Panics if the reply cannot be serialized, which would mean a bug in this
    /// module rather than anything the page did.
    ///
    /// A [`Json`](Self::Json) reply that is not valid JSON is a contract
    /// violation of whoever built it, never the page's doing: a debug build
    /// panics, a release build logs it and the page's call rejects.
    #[must_use]
    pub fn resolve_script(&self, id: u64) -> String {
        let (ok, payload) = self.serialized_payload();
        let payload = serde_json::to_string(&payload)
            .expect("a reply payload is a `RawValue`, a string or base64, which always serialize");
        format!("globalThis.__wateruiResolve({id},{ok},{payload});")
    }

    /// Serializes the reply envelope without binding it to a JavaScript realm.
    ///
    /// A backend whose engine offers a reply channel bound to the sending
    /// document returns this value through it — `WebKit`'s
    /// `WKScriptMessageHandlerWithReply`, WPE's script-message reply token,
    /// and Android's `JavaScriptReplyProxy`, which answers on the requesting
    /// message's own proxy — so the engine itself drops a reply whose
    /// document has gone. A backend without such a channel evaluates
    /// [`Self::resolve_script`] in the calling context instead.
    ///
    /// # Panics
    /// Panics if the reply envelope cannot be serialized.
    #[must_use]
    pub fn to_json(&self) -> String {
        self.envelope(None)
    }

    /// Serializes the reply envelope with the request `id` it answers:
    /// `{"id":…,"ok":…,"payload":…}`.
    ///
    /// For a channel bound to the sending document that does not correlate
    /// replies with requests itself — Android's `JavaScriptReplyProxy` — so
    /// the page's transport hands the three fields to `__wateruiResolve`.
    ///
    /// # Panics
    /// Panics if the reply envelope cannot be serialized.
    #[must_use]
    pub fn message(&self, id: u64) -> String {
        self.envelope(Some(id))
    }

    fn envelope(&self, id: Option<u64>) -> String {
        let (ok, payload) = self.serialized_payload();
        serde_json::to_string(&ReplyEnvelope { id, ok, payload })
            .expect("a reply envelope holds an id, a flag and a payload that always serialize")
    }

    fn serialized_payload(&self) -> (bool, ReplyPayload<'_>) {
        match self {
            Self::Json(bytes) => {
                match str::from_utf8(bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|text| serde_json::from_str(text).map_err(|error| error.to_string()))
                {
                    Ok(json) => (true, ReplyPayload::Json { json }),
                    Err(error) => {
                        contract_violation!(
                            "a JSON reply must be UTF-8 JSON, got an invalid one: {error}; \
                             the page's call is rejected instead"
                        );
                        (
                            false,
                            ReplyPayload::Failure {
                                message: INVALID_JSON_REPLY,
                            },
                        )
                    }
                }
            }
            Self::Bytes(bytes) => (
                true,
                ReplyPayload::Bytes {
                    b64: STANDARD.encode(bytes),
                },
            ),
            Self::Failure(message) => (false, ReplyPayload::Failure { message }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Reply, Request};

    #[test]
    fn a_json_payload_arrives_without_base64() {
        let request = Request::parse(r#"{"id":7,"name":"greet","json":{"name":"Lexo"}}"#)
            .expect("well-formed envelope");

        assert_eq!(request.id, 7);
        assert_eq!(request.name, "greet");
        assert!(!request.is_binary);
        assert_eq!(request.payload, br#"{"name":"Lexo"}"#);
    }

    #[test]
    fn a_binary_payload_round_trips_through_base64() {
        let request =
            Request::parse(r#"{"id":1,"name":"upload","b64":"AAEC"}"#).expect("well-formed");

        assert!(request.is_binary);
        assert_eq!(request.payload, vec![0, 1, 2]);
    }

    #[test]
    fn a_call_with_no_payload_is_accepted() {
        let request = Request::parse(r#"{"id":2,"name":"ping"}"#).expect("well-formed");
        assert_eq!(request.payload, Vec::<u8>::new());
    }

    /// Page script can reach the transport directly, so these must be errors
    /// rather than panics.
    #[test]
    fn malformed_envelopes_are_rejected_not_fatal() {
        assert!(Request::parse("garbage").is_err());
        assert!(Request::parse(r#"{"id":"not a number","name":"x"}"#).is_err());
        assert!(Request::parse(r#"{"name":"missing id"}"#).is_err());
        assert!(Request::parse(r#"{"id":1,"name":"x","b64":"not base64!"}"#).is_err());
    }

    /// A JSON reply travels as JSON. Sending it as base64 like any other bytes
    /// is what made `await waterui.invoke(...)` resolve to base64 text.
    #[test]
    fn a_json_reply_is_not_base64() {
        let script = Reply::Json(br#"{"text":"Hi Lexo"}"#.to_vec()).resolve_script(3);
        assert_eq!(
            script,
            r#"globalThis.__wateruiResolve(3,true,{"json":{"text":"Hi Lexo"}});"#
        );
    }

    /// Bytes that are not JSON never reach the page as source: the call rejects.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a JSON reply must be UTF-8 JSON")
    )]
    fn an_invalid_json_reply_rejects_the_call() {
        let reply = Reply::Json(b"{not json".to_vec());
        assert_eq!(
            reply.resolve_script(6),
            r#"globalThis.__wateruiResolve(6,false,{"message":"the handler's reply was not valid JSON"});"#
        );
        assert_eq!(
            reply.message(6),
            r#"{"id":6,"ok":false,"payload":{"message":"the handler's reply was not valid JSON"}}"#
        );
    }

    #[test]
    fn replies_serialize_their_payload_rather_than_interpolating_it() {
        let script = Reply::Bytes(vec![1, 2, 3]).resolve_script(4);
        assert_eq!(
            script,
            r#"globalThis.__wateruiResolve(4,true,{"b64":"AQID"});"#
        );

        // A message containing quotes and a closing brace must not escape the call.
        let script = Reply::failure(r#"he said "}"; alert(1)"#).resolve_script(5);
        assert_eq!(
            script,
            r#"globalThis.__wateruiResolve(5,false,{"message":"he said \"}\"; alert(1)"});"#
        );
    }

    #[test]
    fn replies_serialize_as_json_for_realm_bound_transports() {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &Reply::Json(br#"{"answer":42}"#.to_vec()).to_json()
            )
            .unwrap(),
            serde_json::json!({"ok": true, "payload": {"json": {"answer": 42}}})
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&Reply::Bytes(vec![0, 1, 2]).to_json())
                .unwrap(),
            serde_json::json!({"ok": true, "payload": {"b64": "AAEC"}})
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &Reply::failure(r#"failure "message""#).to_json()
            )
            .unwrap(),
            serde_json::json!({"ok": false, "payload": {"message": "failure \"message\""}})
        );
    }

    /// The message form carries the request id beside the envelope `to_json`
    /// renders, as one JSON object the page parses rather than runs.
    #[test]
    fn a_reply_message_carries_its_request_id() {
        assert_eq!(
            Reply::Json(br#"{"text":"Hi Lexo"}"#.to_vec()).message(3),
            r#"{"id":3,"ok":true,"payload":{"json":{"text":"Hi Lexo"}}}"#
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&Reply::failure(r#"he said "}""#).message(5))
                .unwrap(),
            serde_json::json!({"id": 5, "ok": false, "payload": {"message": "he said \"}\""}})
        );
    }
}
