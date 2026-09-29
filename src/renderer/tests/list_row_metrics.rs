//! Row metrics tests for water-rs/hydrolysis#201 (water-rs/waterui#1249).
//!
//! `ListItem::insets` replaces the theme's row insets for one row and
//! `ListConfig::min_row_height` (via `.list_min_row_height`) replaces the
//! theme's one-line floor for every row under it. Both unset keeps the
//! theme's metrics exactly.

use core::time::Duration;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Rect, Role, TreeUpdate};
use nami::Binding;
use nami::collection::SignalCollection;
use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::padding::EdgeInsets;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 700;
/// `MinimalTestTheme`'s one-line row height — what every row measures with the
/// metrics unset.
const ROW_HEIGHT: f64 = 56.0;
/// Its symmetric row insets: 10pt vertical each edge, 16pt horizontal.
const THEME_VERTICAL_INSETS: f64 = 20.0;
const THEME_HORIZONTAL_INSET: f64 = 16.0;
/// One-line label text size.
const LABEL_SIZE: f64 = 20.0;

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// Pumps until the runtime reports quiet — never fewer than `min_frames` —
/// then returns the settled merged tree (as the vec `node_bounds` reads).
fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant, min_frames: u32) -> Vec<TreeUpdate> {
    let mut frame = 0;
    loop {
        frame += 1;
        *at += Duration::from_millis(16);
        let _ = runtime.pump_at(false, *at);
        if frame >= min_frames
            && (frame >= 300 || (runtime.is_settled() && !runtime.has_pending_semantic_update()))
        {
            break;
        }
    }
    runtime.accessibility_tree().into_iter().collect()
}

/// The bounds the settled tree carries for the node with `role`
/// derived-labelled `label` — `None` when the node is absent.
fn node_bounds(updates: &[TreeUpdate], role: Role, label: &str) -> Option<Rect> {
    updates.iter().rev().find_map(|update| {
        update.nodes.iter().find_map(|(_, node)| {
            if node.role() == role && node.label() == Some(label) {
                node.bounds()
            } else {
                None
            }
        })
    })
}

/// A list of `count` 20pt one-line rows; `insets_of` selects per-row insets,
/// `floor` the list-wide minimum row height.
fn metrics_list(
    count: u64,
    insets_of: impl Fn(u64) -> Option<EdgeInsets> + 'static,
    floor: Option<f32>,
) -> AnyViewBuilder<AnyView> {
    let rows = Binding::container((0..count).map(SelfId::new).collect::<Vec<_>>());
    let insets_of: Rc<dyn Fn(u64) -> Option<EdgeInsets>> = Rc::new(insets_of);
    AnyViewBuilder::<AnyView>::new(move || {
        let insets_of = insets_of.clone();
        let list = List::for_each(SignalCollection::new(rows.clone()), move |id| {
            let index = id.into_inner();
            let item = ListItem::new(text(format!("Row {index}")).size(LABEL_SIZE));
            match insets_of(index) {
                Some(insets) => item.insets(insets),
                None => item,
            }
        });
        match floor {
            Some(floor) => AnyView::new(list.list_min_row_height(floor)),
            None => AnyView::new(list),
        }
    })
}

/// `.list_min_row_height(0.0)` drops the floor entirely: a one-line row with
/// 4pt insets measures its content plus the insets — 20 + 8, not the theme's
/// 56 (water-rs/hydrolysis#201).
#[test]
fn zero_min_row_height_sizes_row_to_content_plus_insets() {
    let mut runtime = runtime(metrics_list(1, |_| Some(EdgeInsets::all(4.0)), Some(0.0)));
    let mut at = Instant::now();
    let updates = settle(&mut runtime, &mut at, 4);

    let row = node_bounds(&updates, Role::ListItem, "Row 0").expect("the row must publish bounds");
    let label =
        node_bounds(&updates, Role::Label, "Row 0").expect("the row's label must publish bounds");
    assert!(
        (row.height() - (label.height() + 8.0)).abs() < 1.0,
        "floor 0 must size the row to content + insets (row {:.1}, label {:.1})",
        row.height(),
        label.height()
    );
    assert!(
        row.height() < ROW_HEIGHT - 8.0,
        "the one-line floor must not apply (row {:.1}, floor {ROW_HEIGHT})",
        row.height()
    );
}

/// A row's insets position its content edge for edge: `leading`/`trailing`
/// move the content's left/right in from the row's sides (water-rs/hydrolysis
/// #201) instead of the theme's symmetric horizontal inset.
#[test]
fn row_insets_position_the_content() {
    let mut runtime = runtime(metrics_list(
        1,
        |_| Some(EdgeInsets::new(6.0, 6.0, 24.0, 4.0)),
        None,
    ));
    let mut at = Instant::now();
    let updates = settle(&mut runtime, &mut at, 4);

    let row = node_bounds(&updates, Role::ListItem, "Row 0").expect("the row must publish bounds");
    let label =
        node_bounds(&updates, Role::Label, "Row 0").expect("the row's label must publish bounds");
    assert!(
        (label.x0 - row.x0 - 24.0).abs() < 1.5,
        "the leading inset must move the content to row x0 + 24 (got {:.1})",
        label.x0 - row.x0
    );
    // Text keeps a small glyph-side margin inside its proposal, so the right
    // gap is pinned as a range: at least the row's 4pt trailing inset and
    // provably less than the theme's 16pt one it replaced.
    let trailing_gap = row.x1 - label.x1;
    assert!(
        (4.0..THEME_HORIZONTAL_INSET).contains(&trailing_gap),
        "the trailing inset must bound the content right of x1 - 16 (got {trailing_gap:.1})"
    );
}

/// The configured floor bounds every row while a row's insets stay its own:
/// under `.list_min_row_height(64.0)` the two plain rows measure 64 and only
/// the inset row's 30pt insets push it past the floor (water-rs/hydrolysis
/// #201).
#[test]
fn min_row_height_floors_every_row_while_insets_stay_per_row() {
    let mut runtime = runtime(metrics_list(
        3,
        |index| (index == 1).then(|| EdgeInsets::all(30.0)),
        Some(64.0),
    ));
    let mut at = Instant::now();
    let updates = settle(&mut runtime, &mut at, 4);

    let first = node_bounds(&updates, Role::ListItem, "Row 0").expect("row 0 must publish bounds");
    let inset = node_bounds(&updates, Role::ListItem, "Row 1").expect("row 1 must publish bounds");
    let last = node_bounds(&updates, Role::ListItem, "Row 2").expect("row 2 must publish bounds");
    assert!(
        (first.height() - 64.0).abs() < 1.0,
        "a plain row must sit on the configured floor (got {:.1})",
        first.height()
    );
    assert!(
        (last.height() - 64.0).abs() < 1.0,
        "the floor must cover every row, not just the first (got {:.1})",
        last.height()
    );
    let expected_inset_height = (LABEL_SIZE + 60.0).max(64.0);
    assert!(
        (inset.height() - expected_inset_height).abs() < 4.0,
        "only the inset row may pay for its insets (got {:.1}, want ~{expected_inset_height})",
        inset.height()
    );
    // Rows still stack contiguously on their measured extents.
    assert!(
        (inset.y0 - first.y1).abs() < 1.0 && (last.y0 - inset.y1).abs() < 1.0,
        "rows must stack without gaps (y1 {:.1} -> y0 {:.1} -> y1 {:.1} -> y0 {:.1})",
        first.y1,
        inset.y0,
        inset.y1,
        last.y0
    );
}

/// With neither metric set the theme's row height and insets apply exactly:
/// 56pt rows with the 16pt horizontal inset (water-rs/hydrolysis#201
/// regression pin).
#[test]
fn unset_metrics_keep_the_theme_row_geometry() {
    let mut runtime = runtime(metrics_list(3, |_| None, None));
    let mut at = Instant::now();
    let updates = settle(&mut runtime, &mut at, 4);

    for index in 0..3 {
        let label_text = format!("Row {index}");
        let row = node_bounds(&updates, Role::ListItem, &label_text)
            .expect("every row must publish bounds");
        assert!(
            (row.height() - ROW_HEIGHT).abs() < 1.0,
            "row {index} must keep the theme's one-line height (got {:.1})",
            row.height()
        );
        let label = node_bounds(&updates, Role::Label, &label_text)
            .expect("every row's label must publish bounds");
        assert!(
            (label.x0 - row.x0 - THEME_HORIZONTAL_INSET).abs() < 1.5,
            "row {index} must keep the theme's horizontal inset (got {:.1})",
            label.x0 - row.x0
        );
        assert!(
            (row.height() - label.height() - THEME_VERTICAL_INSETS) > 10.0,
            "row {index} must keep the theme's vertical insets (row {:.1}, label {:.1})",
            row.height(),
            label.height()
        );
    }
}
