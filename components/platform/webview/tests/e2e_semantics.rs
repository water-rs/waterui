//! What a `WebView` looks like from the accessibility tree.
//!
//! The tree is `WaterUI`'s real observation channel, and it is all a screen reader
//! gets here: page content lives in a texture or a native subview that the host
//! tree cannot see into, so the component's own node is the whole surface.
//!
//! These tests run in the configuration the harness actually has — a renderer
//! with no web engine compiled in, driving
//! [`WebViewController::without_engine`]. That is a real configuration rather
//! than a stand-in for a browser: it is what an embedded target, a headless
//! renderer, or any build without a `webview-*` feature gets. Consequently the
//! properties asserted here are the ones that do not need a page: that the
//! component publishes exactly one meaningful node, and that writing a new URL
//! into a bound `Binding` navigates the web view it already has instead of
//! building another one.

use std::time::Duration;

use hydrolysis_m3::install as install_m3;
use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::text;
use waterui::{Environment, SignalExt as _, Str};
use waterui_core::binding;
use waterui_testing::{Role, UiBuilder};
use waterui_webview::{Url, WebView, WebViewController};

const DOCS_URL: &str = "https://waterui.dev/docs";
const API_URL: &str = "https://waterui.dev/api";

const WEBVIEW_WIDTH: f32 = 320.0;
const WEBVIEW_HEIGHT: f32 = 260.0;

/// A `WebView` owes the accessibility tree one node, and it has to be a usable
/// one: the role a container gets by default, the label the view was given, and
/// the bounds it was laid out at. Exactly one, too — a component that registers
/// its node twice reads to a screen reader as two web views sitting on top of
/// each other.
#[waterui::test(viewport = (420, 420))]
fn a_webview_publishes_one_labelled_node(ui: UiBuilder) {
    let mut env = Environment::new();
    env.insert(WebViewController::without_engine());
    let mut app = ui.theme(install_m3).environment(env).mount(|| {
        WebView::open(DOCS_URL)
            .a11y_label("Docs WebView")
            .size(WEBVIEW_WIDTH, WEBVIEW_HEIGHT)
    });

    let nodes = app.query().label("Docs WebView").all();
    assert_eq!(
        nodes.len(),
        1,
        "a WebView publishes exactly one accessibility node, found: {nodes:?}"
    );

    let webview = app.query().label("Docs WebView").single();
    assert_eq!(
        webview.node().role(),
        Role::GROUP,
        "a WebView reads as a container unless `a11y_role` says otherwise"
    );
    let bounds = webview.bounds();
    assert!(
        (bounds.width() - WEBVIEW_WIDTH).abs() < 0.5
            && (bounds.height() - WEBVIEW_HEIGHT).abs() < 0.5,
        "the published node covers the web view's own bounds, got {}x{}",
        bounds.width(),
        bounds.height(),
    );
}

/// Writing a new URL into the binding a web view opened with navigates that web
/// view. The give-away for the other outcome — the view being torn down and a
/// fresh one built — is structural, so that is what this reads: the node the
/// component published before the write is the same node afterwards, while the
/// value derived from the same binding does change.
#[waterui::test(viewport = (420, 420))]
fn changing_the_url_binding_keeps_the_same_webview(ui: UiBuilder) {
    let url = binding(Url::new(DOCS_URL));
    let url_for_view = url.clone();

    let mut env = Environment::new();
    env.insert(WebViewController::without_engine());
    let mut app = ui.theme(install_m3).environment(env).mount(move || {
        let address = url_for_view
            .clone()
            .map(|url: Url| Str::from(url.as_str().to_owned()));
        vstack((
            text!("Address: {address}"),
            WebView::open(url_for_view.clone())
                .a11y_label("Docs WebView")
                .size(WEBVIEW_WIDTH, WEBVIEW_HEIGHT),
        ))
    });

    app.query()
        .label(format!("Address: {DOCS_URL}"))
        .assert_exists();
    let before = app.query().label("Docs WebView").single().id();

    url.set(Url::new(API_URL));

    assert!(
        app.query()
            .label(format!("Address: {API_URL}"))
            .wait_for_existence(Duration::from_millis(250)),
        "the URL binding drives the view, so the derived address has to follow it"
    );
    let after = app.query().label("Docs WebView").single().id();
    assert_eq!(
        before, after,
        "a new URL navigates the existing web view; a different node id means the \
         component was rebuilt instead"
    );
}
