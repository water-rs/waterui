//! The renderer-process side of the web view bridge.
//!
//! `Page.addScriptToEvaluateOnNewDocument` runs its source in every document
//! of every frame, whatever the document's origin, so it cannot carry the
//! bridge: a cross-origin iframe or a refused document would get the bridge and
//! every application script along with it. A web view's creation-time bundle —
//! its admission rules and the final source of each script — travels to the
//! renderer as data instead, in the browser's `extra_info`, which CEF delivers
//! to every renderer process the browser is created in before that process
//! loads a document. The renderer's own process handler then decides per
//! document: the evaluation wrapper goes into the main frame's default world of
//! every document, and the bridge and the scripts only into one the rules
//! admit.
//!
//! The bundle carries no application value. A document receives mirrored state
//! only as the reply to its own first pull, which the browser process answers
//! after authenticating the calling context.
//!
//! Chromium runs this handler from `DidCreateScriptContext`, after the
//! inspector has installed `Runtime.addBinding` bindings into the new context
//! and before it evaluates `Page.addScriptToEvaluateOnNewDocument` sources, so
//! the bridge can send at once and a script injected later over `DevTools` finds
//! the bridge already defined.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cef::rc::Rc as _;
use cef::{
    Browser, CefString, DictionaryValue, Frame, ImplBrowser, ImplDictionaryValue, ImplFrame,
    ImplRenderProcessHandler, ImplV8Context, ImplV8Exception, RenderProcessHandler, V8Context,
    V8Exception, V8Value, WrapRenderProcessHandler, dictionary_value_create,
};
use serde::{Deserialize, Serialize};
use waterui_url::Url;
use waterui_webview::{OriginRule, ScriptInjectionTime, WebViewConfig};

/// The `extra_info` key a web view's bundle travels under. A browser whose
/// `extra_info` lacks it is not a web view — a Chromium component page — and
/// gets nothing injected.
const BUNDLE_KEY: &str = "waterui:webview";

/// What a renderer process needs to know about one web view before its first
/// document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RendererBundle {
    /// The admission rules in [`OriginPolicy::wire`] form; empty is deny-all.
    ///
    /// [`OriginPolicy::wire`]: waterui_webview::OriginPolicy::wire
    rules: String,
    /// The final source of every creation-time script, in run order.
    scripts: Vec<String>,
}

impl RendererBundle {
    /// The bundle a view created with `config` hands its renderers.
    pub fn new(config: &WebViewConfig) -> Self {
        Self {
            rules: config.bridge_origins.wire().to_string(),
            scripts: config
                .user_scripts()
                .iter()
                .map(|script| document_source(&script.source, script.time))
                .collect(),
        }
    }

    /// The `extra_info` dictionary carrying this bundle.
    pub fn to_extra_info(&self) -> DictionaryValue {
        let dictionary =
            dictionary_value_create().expect("CEF must allocate the web view's extra_info");
        let bundle: CefString = serde_json::to_string(self)
            .expect("a renderer bundle must serialize")
            .as_str()
            .into();
        dictionary.set_string(Some(&BUNDLE_KEY.into()), Some(&bundle));
        dictionary
    }

    /// Reads the bundle back from a browser's `extra_info`, or `None` for a
    /// browser that is not a web view.
    fn from_extra_info(extra_info: &DictionaryValue) -> Option<Self> {
        let key: CefString = BUNDLE_KEY.into();
        if extra_info.has_key(Some(&key)) != 1 {
            return None;
        }
        let bundle = CefString::from(&extra_info.string(Some(&key))).to_string();
        Some(
            serde_json::from_str(&bundle)
                .expect("the browser process wrote this bundle; it must parse"),
        )
    }

    /// Whether the rules admit the document at `url`.
    ///
    /// The URL is parsed with `FromStr`, not `Url::parse`: a `file:` document
    /// and an opaque one must reach the matcher, which decides them, rather
    /// than be filtered out as non-web first.
    fn admits(&self, url: &str) -> bool {
        url.parse::<Url>().is_ok_and(|url| {
            OriginRule::parse_wire(&self.rules)
                .iter()
                .any(|rule| rule.admits_document(&url))
        })
    }
}

/// The source a script runs as in a CEF document.
///
/// Both injection channels — the renderer's context hook and
/// `Page.addScriptToEvaluateOnNewDocument` — run at document start, so a
/// document-end script defers itself to `DOMContentLoaded`.
pub fn document_source(script: &str, time: ScriptInjectionTime) -> String {
    match time {
        ScriptInjectionTime::DocumentStart => script.to_string(),
        ScriptInjectionTime::DocumentEnd => format!(
            "globalThis.addEventListener('DOMContentLoaded',()=>globalThis.eval({}),{{once:true}});",
            serde_json::to_string(script).expect("a script must serialize")
        ),
    }
}

/// Evaluates `source` in the entered `context`.
///
/// A throwing script is reported, not fatal: the renderer cannot stop the
/// document from loading, and the scripts after it still run.
fn eval(context: &V8Context, source: &str) {
    let code: CefString = source.into();
    let mut value: Option<V8Value> = None;
    let mut exception: Option<V8Exception> = None;
    context.eval(Some(&code), None, 0, Some(&mut value), Some(&mut exception));
    if let Some(exception) = exception {
        tracing::error!(
            message = %CefString::from(&exception.message()),
            "a WaterUI document-start script threw"
        );
    }
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
pub fn new_render_process_handler() -> RenderProcessHandler {
    cef::wrap_render_process_handler! {
        struct WaterRenderProcessHandler {
            // The bundle of every web view this process renders, by browser
            // id. A renderer process serves one thread, so plain shared
            // ownership is enough.
            bundles: Rc<RefCell<HashMap<i32, RendererBundle>>>,
        }

        impl RenderProcessHandler {
            fn on_browser_created(
                &self,
                browser: Option<&mut Browser>,
                extra_info: Option<&mut DictionaryValue>,
            ) {
                let browser = browser.expect("CEF must name the browser it created");
                if let Some(bundle) = extra_info.and_then(|info| RendererBundle::from_extra_info(info)) {
                    self.bundles.borrow_mut().insert(browser.identifier(), bundle);
                }
            }

            fn on_browser_destroyed(&self, browser: Option<&mut Browser>) {
                let browser = browser.expect("CEF must name the browser it destroyed");
                self.bundles.borrow_mut().remove(&browser.identifier());
            }

            fn on_context_created(
                &self,
                browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                context: Option<&mut V8Context>,
            ) {
                let browser = browser.expect("CEF must name the browser of a new context");
                let frame = frame.expect("CEF must name the frame of a new context");
                let context = context.expect("CEF must hand over the context it created");
                // A subframe's document is never a bridge document, whatever
                // its origin.
                if frame.is_main() != 1 {
                    return;
                }
                // `Frame::v8_context` is the frame's default world; any other
                // context is an isolated world, which must not see the bridge.
                let default_world = frame
                    .v8_context()
                    .is_some_and(|mut main| context.is_same(Some(&mut main)) == 1);
                if !default_world {
                    return;
                }
                let bundles = self.bundles.borrow();
                let Some(bundle) = bundles.get(&browser.identifier()) else {
                    return;
                };
                let url = CefString::from(&frame.url()).to_string();
                assert_eq!(
                    context.enter(),
                    1,
                    "CEF refused to enter the context it just created"
                );
                // The evaluation wrapper reaches every main-frame document: the
                // browser's own evaluations run through it, and it opens no
                // path from the page to the backend.
                eval(context, waterui_webview::EVAL_SCRIPT);
                if bundle.admits(&url) {
                    // The bridge first: the scripts may call it as they run.
                    eval(context, waterui_webview::BRIDGE_SCRIPT);
                    for script in &bundle.scripts {
                        eval(context, script);
                    }
                }
                assert_eq!(context.exit(), 1, "CEF refused to exit an entered context");
            }
        }
    }

    WaterRenderProcessHandler::new(Rc::new(RefCell::new(HashMap::new())))
}

#[cfg(test)]
mod tests {
    use waterui_webview::{BridgeOrigins, OriginPolicy, UserScript, WebViewConfig};

    use super::RendererBundle;

    fn bundle(origins: BridgeOrigins) -> RendererBundle {
        let initial: waterui_url::Url = "https://app.example".parse().unwrap();
        RendererBundle::new(
            &WebViewConfig::default()
                .with_bridge_origins(OriginPolicy::new(origins, &initial))
                .with_scripts(vec![UserScript::new(
                    "app",
                    "globalThis.app = 1;",
                    waterui_webview::ScriptInjectionTime::DocumentEnd,
                )]),
        )
    }

    #[test]
    fn the_rules_admit_only_the_documents_the_policy_admits() {
        let bundle = bundle(BridgeOrigins::Initial);
        assert!(bundle.admits("https://app.example/page"));
        assert!(!bundle.admits("https://other.example/"));
        assert!(!bundle.admits("about:blank"));
        let deny_all = RendererBundle::new(&WebViewConfig::default());
        assert!(!deny_all.admits("https://app.example/"));
    }

    #[test]
    fn the_scripts_keep_their_order_and_defer_document_end() {
        let bundle = bundle(BridgeOrigins::Initial);
        assert_eq!(
            bundle.scripts.len(),
            2,
            "the state seed, then the app script"
        );
        assert!(bundle.scripts[1].contains("DOMContentLoaded"));
    }
}
