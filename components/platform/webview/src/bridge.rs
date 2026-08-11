//! The JavaScript bridge shared by every `WebView` backend.
//!
//! Four backends each carried a near-identical copy of the bridge script with a
//! subtly different wire format, and two more (CEF, WPE) exposed a different
//! page-facing API entirely. A snippet written against one backend could not run
//! on another, which defeats the purpose of having one `WebView` component.
//!
//! This module owns the single script and the single envelope. A backend supplies
//! only a transport: deliver [`SCRIPT`] at document start, hand
//! [`Request::parse`] whatever `__wateruiSend` produced, and evaluate
//! [`Reply::resolve_script`] to complete the call.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// The bridge script, injected at document start by every backend.
pub const SCRIPT: &str = include_str!("js/bridge.js");

/// The global the bridge calls to hand a request to the native side.
///
/// A backend installs a function under this name — a `WebKit` message handler, an
/// Android `@JavascriptInterface` method, a CDP binding — taking one string.
pub const SEND_FUNCTION: &str = "__wateruiSend";

/// One `waterui.invoke(...)` call, as parsed from the page.
///
/// Every field is validated: this arrives from page script, which may be
/// untrusted, so a malformed request is rejected rather than fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Correlates the reply with the pending promise in the page.
    pub id: u64,
    /// The handler name the page asked for.
    pub name: String,
    /// The payload, already decoded to bytes.
    pub payload: Vec<u8>,
    /// Whether the payload arrived as binary rather than JSON.
    pub is_binary: bool,
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
pub enum Reply {
    /// Bytes the handler produced, delivered as base64.
    Bytes(Vec<u8>),
    /// The call failed; the page's promise rejects with this message.
    Failure(String),
}

#[derive(Serialize)]
#[serde(untagged)]
enum ReplyPayload<'a> {
    Bytes { b64: String },
    Failure { message: &'a str },
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
    #[must_use]
    pub fn resolve_script(&self, id: u64) -> String {
        let (ok, payload) = match self {
            Self::Bytes(bytes) => (
                true,
                ReplyPayload::Bytes {
                    b64: STANDARD.encode(bytes),
                },
            ),
            Self::Failure(message) => (false, ReplyPayload::Failure { message }),
        };
        let payload =
            serde_json::to_string(&payload).expect("WaterUI bridge reply must serialize");
        format!("globalThis.__wateruiResolve({id},{ok},{payload});")
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
        assert!(request.payload.is_empty());
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
}
