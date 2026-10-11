//! Anchored overlay example — every `AnchorEdge`, edge alignments, and the
//! flip that fires when the anchor sits against the window edge it names.
//!
//! Tap any anchor to present its overlay; tap outside it (or the anchor
//! again) to dismiss. The top row asks for `AnchorEdge::Top` where no room
//! remains above, so each overlay renders below its anchor; the bottom row
//! asks for `AnchorEdge::Bottom` and flips above; the middle row shows the
//! same edges where they fit without flipping.

use waterui::app::App;
use waterui::border::Border;
use waterui::metadata::anchored_overlay::{
    AnchorEdge, AnchoredOverlay, Clamp, Dismissal, EdgeAlignment,
};
use waterui::prelude::theme_color::{Border as BorderColor, Surface};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;

/// The small card every demo presents as overlay content. The label names
/// the edge the placement asked for.
fn overlay_card(note: &'static str) -> impl View {
    text(note)
        .caption()
        .padding_with(12.0)
        .background(Surface)
        .border_with(Border::new(BorderColor, 1.0).corner_radius(8.0))
}

/// A taller overlay card for the flip demos — too tall to fit in the band
/// between the anchor and the window edge, so the placement flips.
fn tall_card(note: &'static str) -> impl View {
    overlay_card(note).height(96.0)
}

/// A button that presents an `AnchoredOverlay` against `edge`.
fn edge_anchor(title: &'static str, note: &'static str, edge: AnchorEdge, tall: bool) -> impl View {
    let open = binding(false);
    let card = if tall {
        AnyView::new(tall_card(note))
    } else {
        AnyView::new(overlay_card(note))
    };
    button(title)
        .action({
            let open = open.clone();
            move || open.toggle()
        })
        .anchored_overlay(
            AnchoredOverlay::new(&open, card)
                .edge(edge)
                .gap(6.0)
                .clamp(Clamp::Window { margin: 8.0 }),
        )
}

/// An anchor that varies both the edge and the alignment along it.
fn aligned_anchor(title: &'static str, edge: AnchorEdge, alignment: EdgeAlignment) -> impl View {
    let open = binding(false);
    let note = match (edge, alignment) {
        (AnchorEdge::Top, EdgeAlignment::Start) => "Top edge, start",
        (AnchorEdge::Top, EdgeAlignment::End) => "Top edge, end",
        (AnchorEdge::Bottom, EdgeAlignment::Start) => "Bottom edge, start",
        (AnchorEdge::Bottom, EdgeAlignment::End) => "Bottom edge, end",
        _ => "Aligned",
    };
    button(title)
        .action({
            let open = open.clone();
            move || open.toggle()
        })
        .anchored_overlay(
            AnchoredOverlay::new(&open, overlay_card(note))
                .edge(edge)
                .alignment(alignment)
                .gap(6.0)
                .clamp(Clamp::Window { margin: 8.0 }),
        )
}

/// A `Dismissal::Manual` demo: the overlay ignores outside interaction and
/// closes only through the binding.
fn manual_anchor() -> impl View {
    let open = binding(false);
    button("Manual")
        .action({
            let open = open.clone();
            move || open.toggle()
        })
        .anchored_overlay(
            AnchoredOverlay::new(&open, overlay_card("Trailing edge, manual"))
                .edge(AnchorEdge::Trailing)
                .gap(6.0)
                .clamp(Clamp::Window { margin: 8.0 })
                .dismissal(Dismissal::Manual),
        )
}

#[preview]
pub fn demo() -> impl View {
    vstack((
        text("Anchored Overlay").title().bold(),
        text("Tap an anchor; the overlay tracks it and stays inside the window.")
            .caption()
            .muted(),
        // `AnchorEdge::Top` on the topmost row: the tall card has no room
        // above these anchors, so each overlay flips and renders below.
        hstack((
            edge_anchor("Top (flips)", "Top edge", AnchorEdge::Top, true),
            aligned_anchor("Top Start", AnchorEdge::Top, EdgeAlignment::Start),
            aligned_anchor("Top End", AnchorEdge::Top, EdgeAlignment::End),
        ))
        .spacing(12.0),
        spacer(),
        // The four edges at their natural orientation — including a `Top`
        // anchor mid-window that fits above without flipping.
        hstack((
            edge_anchor("Leading", "Leading edge", AnchorEdge::Leading, false),
            edge_anchor("Bottom", "Bottom edge", AnchorEdge::Bottom, false),
            edge_anchor("Trailing", "Trailing edge", AnchorEdge::Trailing, false),
            edge_anchor("Top (fits)", "Top edge", AnchorEdge::Top, false),
            manual_anchor(),
        ))
        .spacing(12.0),
        spacer(),
        // `AnchorEdge::Bottom` has no room below these anchors, so the
        // overlay flips above them.
        hstack((
            edge_anchor("Bottom (flips)", "Bottom edge", AnchorEdge::Bottom, true),
            aligned_anchor("Bottom Start", AnchorEdge::Bottom, EdgeAlignment::Start),
            aligned_anchor("Bottom End", AnchorEdge::Bottom, EdgeAlignment::End),
        ))
        .spacing(12.0),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
