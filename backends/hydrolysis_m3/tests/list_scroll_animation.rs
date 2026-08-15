//! Programmatic `List` jumps scroll rather than teleport.
//!
//! The list example's Top / Middle / Last buttons drive
//! `ScrollController::scroll_to`. Landing on the target in a single frame is
//! the bug this guards: the viewport must visibly travel there.

use core::time::Duration;

use waterui::Identifiable;
use waterui::component::list::{List, ListItem};
use waterui::prelude::*;

use waterui::reactive::collection::List as ReactiveList;
use waterui_testing::{OffscreenApp, Role, UiBuilder};

const ROWS: usize = 500;
const TARGET_ROW: usize = 300;
/// One frame at 60Hz.
const FRAME: Duration = Duration::from_millis(16);
/// Comfortably past the ~540ms the approach takes to settle.
const FRAMES: usize = 60;

#[derive(Clone, Copy, Identifiable)]
struct Row {
    #[id]
    id: u64,
}

#[derive(Clone)]
struct Fixture {
    rows: ReactiveList<Row>,
    scroll: ScrollController<usize>,
}

fn fixture() -> Fixture {
    Fixture {
        rows: ReactiveList::from(
            (0..ROWS)
                .map(|index| Row { id: index as u64 })
                .collect::<Vec<_>>(),
        ),
        scroll: ScrollController::new(0),
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the view outlives the call, so it cannot borrow the fixture"
)]
fn list_view(fixture: Fixture) -> impl View {
    let list = List::for_each(fixture.rows.clone(), |row: Row| {
        ListItem::new(text(format!("Row {}", row.id)).padding())
    })
    .scroll_controller(&fixture.scroll);
    vstack((list,))
}

/// Index of the topmost row the viewport is showing, read off the a11y tree.
fn top_visible_row(app: &mut OffscreenApp) -> Option<usize> {
    app.query()
        .role(Role::LIST_ITEM)
        .all()
        .iter()
        .filter_map(|element| {
            element
                .node()
                .label()
                .and_then(|label| label.strip_prefix("Row ").map(str::to_owned))
        })
        .filter_map(|index| index.parse::<usize>().ok())
        .min()
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn an_indexed_jump_scrolls_instead_of_teleporting(ui: UiBuilder) {
    let fixture = fixture();
    let scroll = fixture.scroll.clone();
    let mut app = ui.mount_offscreen(move || list_view(fixture.clone()));

    assert_eq!(
        top_visible_row(&mut app),
        Some(0),
        "the list should start at the top"
    );

    scroll.scroll_to(TARGET_ROW);

    let mut positions = Vec::new();
    for _ in 0..FRAMES {
        app.pump_for(FRAME);
        if let Some(top) = top_visible_row(&mut app) {
            positions.push(top);
        }
    }

    let arrived = positions
        .last()
        .copied()
        .expect("the list should report a visible row after the jump");
    assert!(
        arrived.abs_diff(TARGET_ROW) <= 1,
        "the jump should settle on row {TARGET_ROW}, ended at {arrived}"
    );

    // The point of the test: the viewport passed through positions on its way
    // rather than appearing at the destination on the first frame.
    let distinct = positions
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    assert!(
        distinct >= 3,
        "an animated jump must move through intermediate rows; saw only {distinct} \
         distinct positions: {positions:?}"
    );
    assert_ne!(
        positions.first().copied(),
        Some(TARGET_ROW),
        "the first frame after the jump must not already be at the target"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn a_short_jump_is_animated_end_to_end(ui: UiBuilder) {
    const NEARBY_ROW: usize = 12;

    let fixture = fixture();
    let scroll = fixture.scroll.clone();
    let mut app = ui.mount_offscreen(move || list_view(fixture.clone()));

    scroll.scroll_to(NEARBY_ROW);

    let mut positions = Vec::new();
    for _ in 0..FRAMES {
        app.pump_for(FRAME);
        if let Some(top) = top_visible_row(&mut app) {
            positions.push(top);
        }
    }

    // A target inside the teleport bound is never snapped to, so every row
    // between here and there is crossed.
    assert!(
        positions.iter().any(|top| (1..NEARBY_ROW).contains(top)),
        "a nearby jump should scroll through the rows in between: {positions:?}"
    );
    // The row above the target keeps a sliver inside the viewport once the
    // target is aligned to the top edge, so it can still be the topmost row the
    // a11y tree reports.
    let arrived = positions
        .last()
        .copied()
        .expect("the list should report a visible row after the jump");
    assert!(
        arrived.abs_diff(NEARBY_ROW) <= 1,
        "the jump should settle on row {NEARBY_ROW}, ended at {arrived}"
    );
}
