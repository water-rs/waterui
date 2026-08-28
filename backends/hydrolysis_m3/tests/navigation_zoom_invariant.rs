//! A zoom transition whose declared matched pair is incomplete.
//!
//! Every matched transition names one source and one destination. Silently
//! substituting a different transition when either half is absent would hide
//! an invalid declaration, so the renderer fails at the point where it resolves
//! the pair.

use waterui::navigation::{NavigationLink, NavigationPath, NavigationStack, NavigationView};
use waterui::navigation::{NavigationTransitionViewExt as _, navigation_transition};
use waterui::prelude::*;
use waterui_core::id::Id;
use waterui_testing::UiBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Page(u8);

fn matched_id() -> Id {
    Id::try_from(1).expect("a matched transition id must be non-zero")
}

/// The transition declares an id whose destination half is absent.
fn invalid_gallery() -> impl View {
    let matched = NavigationLink::value(text("Matched tile"), Page(0))
        .navigation_transition_source(matched_id());
    let unmatched = NavigationLink::value(text("Unmatched tile"), Page(1));
    NavigationStack::with_path(
        NavigationPath::<Page>::new(),
        vstack((matched, unmatched)).title("Gallery"),
    )
    .destination(|Page(index)| {
        NavigationView::new(
            format!("Page {index}"),
            text(format!("page {index} content")),
        )
    })
    .transition(navigation_transition::zoom(matched_id()))
}

/// An incomplete matched pair is rejected instead of changing transition kind.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 400))]
#[should_panic(expected = "navigation zoom destination")]
fn a_zoom_without_matched_geometry_fails_fast(ui: UiBuilder) {
    let mut app = ui.mount(invalid_gallery);
    app.settle();

    let tile = app.query().label("Unmatched tile").single().bounds();
    let (x, y) = tile.center();
    app.tap_at(x, y);
    app.settle();

    let _ = app.query().label("page 1 content").single();
}
