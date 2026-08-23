//! A zoom transition whose matched geometry is not on screen.
//!
//! A matched element registers while it is *painted*, so a page that never
//! drew its half of the pair has none to offer — a list that scrolled the row
//! away, or, as the navigation example does, a stack that applies one zoom to
//! every destination while only one of six tiles is marked as its source.
//! That used to panic inside the renderer and take the application down for
//! what is ordinary use.

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

/// Only the first link is a matched source; the stack zooms on every push.
fn gallery() -> impl View {
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

/// Pushing from a tile that was never marked must transition, not crash.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 400))]
fn a_zoom_without_matched_geometry_falls_back(ui: UiBuilder) {
    let mut app = ui.mount(gallery);
    app.settle();

    let tile = app.query().label("Unmatched tile").single().bounds();
    let (x, y) = tile.center();
    app.tap_at(x, y);
    app.settle();

    let _ = app.query().label("page 1 content").single();
}
