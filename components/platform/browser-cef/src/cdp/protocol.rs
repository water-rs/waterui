//! Typed Chrome `DevTools` Protocol commands.
//!
//! Commands used to be `json!({...})` literals and their responses were read
//! field by field with `get("x").and_then(Value::as_str).expect(...)`. Every one
//! of those was a panic waiting for Chromium to rename or omit something, and a
//! misspelled method name was only discovered at runtime.
//!
//! A command is now a type: the method name is an associated constant, the
//! parameters are its fields, and the response is a struct serde fills in.

use serde::{Deserialize, Serialize};

/// One CDP command, with the response it produces.
pub trait CdpCommand: Serialize {
    /// The protocol method, such as [`Evaluate::METHOD`].
    const METHOD: &'static str;

    /// What Chromium sends back.
    type Response: for<'de> Deserialize<'de>;
}

/// Enables the runtime domain, which [`AddBinding`] needs.
#[derive(Debug, Serialize)]
pub struct RuntimeEnable {}

impl CdpCommand for RuntimeEnable {
    const METHOD: &'static str = "Runtime.enable";
    type Response = Empty;
}

/// Installs a function the page can call to reach native code.
#[derive(Debug, Serialize)]
pub struct AddBinding {
    /// The global the page calls.
    pub name: &'static str,
}

impl CdpCommand for AddBinding {
    const METHOD: &'static str = "Runtime.addBinding";
    type Response = Empty;
}

/// Enables the page domain, which [`AddScriptToEvaluateOnNewDocument`] needs.
///
/// Chromium accepts a document-start script whether or not the domain is
/// enabled, and only *runs* the registered scripts while it is, so leaving this
/// out installed the bridge into no document at all and reported success doing
/// it.
#[derive(Debug, Serialize)]
pub struct PageEnable {}

impl CdpCommand for PageEnable {
    const METHOD: &'static str = "Page.enable";
    type Response = Empty;
}

/// Registers a script that runs before anything else in each new document.
#[derive(Debug, Serialize)]
pub struct AddScriptToEvaluateOnNewDocument<'a> {
    /// The script source.
    pub source: &'a str,
}

impl CdpCommand for AddScriptToEvaluateOnNewDocument<'_> {
    const METHOD: &'static str = "Page.addScriptToEvaluateOnNewDocument";
    type Response = ScriptIdentifier;
}

/// Identifies a registered document-start script.
///
/// Kept so the response is decoded rather than ignored; removing a script is not
/// something `WaterUI` does yet.
#[derive(Debug, Deserialize)]
pub struct ScriptIdentifier {
    /// Chromium's handle for the script.
    #[expect(
        dead_code,
        reason = "decoded for completeness; no caller removes a script yet"
    )]
    pub identifier: String,
}

/// Evaluates an expression in the page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evaluate<'a> {
    /// The source to evaluate.
    pub expression: &'a str,
    /// Wait for a promise result rather than returning the promise.
    pub await_promise: bool,
    /// Return the value itself rather than a remote handle.
    pub return_by_value: bool,
}

impl CdpCommand for Evaluate<'_> {
    const METHOD: &'static str = "Runtime.evaluate";
    type Response = EvaluateResponse;
}

/// The outcome of an evaluation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateResponse {
    /// The value, or a description of it.
    pub result: RemoteObject,
    /// Present when the expression threw.
    #[serde(default)]
    pub exception_details: Option<ExceptionDetails>,
}

/// A value the page produced.
#[derive(Debug, Deserialize)]
pub struct RemoteObject {
    /// The value, when it could be returned by value.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// A human-readable description, for values that cannot be.
    #[serde(default)]
    pub description: Option<String>,
}

/// Why an evaluation failed.
#[derive(Debug, Deserialize)]
pub struct ExceptionDetails {
    /// The message Chromium reports.
    pub text: String,
    /// The thrown value, when there was one.
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "`text` carries the message; the value is decoded for completeness"
    )]
    pub exception: Option<RemoteObject>,
}

/// Overrides the user agent for subsequent requests.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUserAgentOverride<'a> {
    /// The user agent to send. Empty clears the override.
    pub user_agent: &'a str,
}

impl CdpCommand for SetUserAgentOverride<'_> {
    const METHOD: &'static str = "Network.setUserAgentOverride";
    type Response = Empty;
}

/// Stores one cookie.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCookie<'a> {
    /// Cookie name.
    pub name: &'a str,
    /// Cookie value.
    pub value: &'a str,
    /// The domain it belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<&'a str>,
    /// The URL it belongs to, used when no domain is given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<&'a str>,
    /// Path scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<&'a str>,
    /// Whether it is HTTPS-only.
    pub secure: bool,
    /// Whether script can read it.
    pub http_only: bool,
    /// Cross-site policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_site: Option<&'static str>,
    /// Expiry as a Unix timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
}

impl CdpCommand for SetCookie<'_> {
    const METHOD: &'static str = "Network.setCookie";
    type Response = Empty;
}

/// Reads the cookies visible to the given URLs.
#[derive(Debug, Serialize)]
pub struct GetCookies<'a> {
    /// The URLs whose cookies to read. Empty means the current document's.
    pub urls: Vec<&'a str>,
}

impl CdpCommand for GetCookies<'_> {
    const METHOD: &'static str = "Network.getCookies";
    type Response = GetCookiesResponse;
}

/// The cookies Chromium returned.
#[derive(Debug, Deserialize)]
pub struct GetCookiesResponse {
    /// One entry per cookie.
    pub cookies: Vec<Cookie>,
}

/// One stored cookie.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// The domain it belongs to.
    pub domain: String,
    /// Path scope.
    pub path: String,
    /// Whether it is HTTPS-only.
    pub secure: bool,
    /// Whether script can read it.
    pub http_only: bool,
    /// Cross-site policy, when Chromium reports one.
    #[serde(default)]
    pub same_site: Option<String>,
    /// Expiry as a Unix timestamp; non-positive means a session cookie.
    #[serde(default)]
    pub expires: f64,
}

/// Captures the page as an image.
///
/// Screenshots are a `chromium` capability: the only caller,
/// `page::screenshot_command`, is gated the same way.
#[cfg(feature = "chromium")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureScreenshot {
    /// Image format.
    pub format: &'static str,
    /// Quality, for formats that have one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    /// Capture from the compositor surface.
    pub from_surface: bool,
    /// Restrict to the visible viewport.
    pub capture_beyond_viewport: bool,
}

#[cfg(feature = "chromium")]
impl CdpCommand for CaptureScreenshot {
    const METHOD: &'static str = "Page.captureScreenshot";
    type Response = CaptureScreenshotResponse;
}

/// The captured image.
#[cfg(feature = "chromium")]
#[derive(Debug, Deserialize)]
pub struct CaptureScreenshotResponse {
    /// Base64-encoded image bytes.
    pub data: String,
}

/// Chooses where downloads are written.
///
/// Redirecting downloads is a `chromium` capability: the only caller,
/// `page::set_download_directory`, is gated the same way.
#[cfg(feature = "chromium")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDownloadBehavior<'a> {
    /// What to do with a download.
    pub behavior: &'static str,
    /// Where to put it.
    pub download_path: &'a str,
}

#[cfg(feature = "chromium")]
impl CdpCommand for SetDownloadBehavior<'_> {
    const METHOD: &'static str = "Browser.setDownloadBehavior";
    type Response = Empty;
}

/// Reports the browser version; used to probe that the `DevTools` agent attached.
#[derive(Debug, Serialize)]
pub struct GetVersion {}

impl CdpCommand for GetVersion {
    const METHOD: &'static str = "Browser.getVersion";
    type Response = Empty;
}

/// A response with nothing worth reading.
#[derive(Debug, Deserialize)]
pub struct Empty {}

#[cfg(test)]
mod tests {
    #[cfg(feature = "chromium")]
    use super::CaptureScreenshot;
    use super::{CdpCommand, Cookie, Evaluate, EvaluateResponse, SetCookie};

    #[test]
    fn parameters_serialize_under_the_names_chromium_expects() {
        let evaluate = Evaluate {
            expression: "1 + 1",
            await_promise: true,
            return_by_value: true,
        };
        let json = serde_json::to_value(&evaluate).expect("serializes");

        assert_eq!(Evaluate::METHOD, "Runtime.evaluate");
        assert_eq!(json["expression"], "1 + 1");
        // camelCase, because that is what the protocol uses.
        assert_eq!(json["awaitPromise"], true);
        assert_eq!(json["returnByValue"], true);
    }

    #[test]
    fn absent_optional_parameters_are_omitted_rather_than_sent_as_null() {
        let cookie = SetCookie {
            name: "session",
            value: "abc",
            domain: None,
            url: Some("https://waterui.dev"),
            path: None,
            secure: true,
            http_only: true,
            same_site: None,
            expires: None,
        };
        let json = serde_json::to_value(&cookie).expect("serializes");

        assert!(json.get("domain").is_none());
        assert!(json.get("sameSite").is_none());
        assert_eq!(json["httpOnly"], true);
    }

    #[test]
    #[cfg(feature = "chromium")]
    fn png_omits_the_quality_parameter_that_only_lossy_formats_take() {
        let png = CaptureScreenshot {
            format: "png",
            quality: None,
            from_surface: true,
            capture_beyond_viewport: false,
        };
        let json = serde_json::to_value(&png).expect("serializes");
        assert!(json.get("quality").is_none());
    }

    #[test]
    fn a_response_missing_its_optional_fields_still_parses() {
        // Previously each of these absent fields was an `expect` away from a panic.
        let response: EvaluateResponse = serde_json::from_str(r#"{"result":{}}"#).expect("parses");
        assert!(response.result.value.is_none());
        assert!(response.exception_details.is_none());

        let cookie: Cookie = serde_json::from_str(
            r#"{"name":"a","value":"b","domain":"waterui.dev","path":"/","secure":true,"httpOnly":false}"#,
        )
        .expect("parses");
        assert!(cookie.same_site.is_none());
        assert!(cookie.expires.abs() < f64::EPSILON);
    }
}
