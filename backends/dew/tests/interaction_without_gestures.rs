//! The firmware contract for interaction metadata: without the `gestures`
//! feature, a view that asks for pointer semantics dew cannot provide must say
//! so, naming the feature, rather than rendering as if the handler were wired.
//!
//! Everything here is compiled out of a default build, and no default test run
//! executes it. It runs in the shape it is about:
//!
//! ```text
//! cargo test -p waterui-dew --no-default-features --features host,system-fonts \
//!     --test interaction_without_gestures
//! ```
#![cfg(not(feature = "gestures"))]

use waterui_core::event::{Event, OnEvent};
use waterui_core::gesture::{GestureObserver, TapGesture};
use waterui_core::{AnyView, Environment, Metadata, Native};
use waterui_dew::DewRenderer;
use waterui_graphics::color::ResolvedColor;

fn render(view: AnyView) {
    let mut renderer = DewRenderer::default();
    let _ = renderer.render_tree(view, &Environment::new(), 32.0, 32.0);
}

#[test]
#[should_panic(expected = "waterui-dew/gestures")]
fn hover_metadata_names_the_feature_it_needs() {
    render(AnyView::new(Metadata::new(
        Native::new(ResolvedColor::default()),
        OnEvent::new(Event::HoverMove, |_: Environment| {}),
    )));
}

#[test]
#[should_panic(expected = "waterui-dew/gestures")]
fn gesture_metadata_names_the_feature_it_needs() {
    render(AnyView::new(Metadata::new(
        Native::new(ResolvedColor::default()),
        GestureObserver::new(TapGesture::new(), || {}),
    )));
}
