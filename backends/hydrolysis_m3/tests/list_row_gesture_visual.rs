//! Visual acceptance for the Material 3 row drag affordances.
//!
//! The PNG-producing tests are ignored by default and reviewed by eye: a
//! mid-swipe frame should show the error-container background with its delete
//! glyph, and a lifted row should read as raised above its neighbours.

use waterui::Identifiable;
use waterui::component::list::{List, ListDelete, ListItem, ListMove};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::reactive::collection::List as ReactiveList;
use waterui_testing::{OffscreenApp, Role, UiBuilder};

const ROW_COUNT: usize = 6;

#[derive(Clone, Copy, Identifiable)]
struct Row {
    #[id]
    id: u64,
}

#[derive(Clone)]
struct Fixture {
    rows: ReactiveList<Row>,
    editing: Binding<bool>,
}

fn delete_row(ListDelete(index): ListDelete, State(fixture): State<Fixture>) {
    let _ = fixture.rows.remove(index);
}

fn move_row(ListMove(movement): ListMove, State(fixture): State<Fixture>) {
    let mut rows = fixture.rows.snapshot();
    let row = rows.remove(movement.from());
    rows.insert(movement.to(), row);
    let _ = fixture.rows.replace(rows);
}

fn list_view(editing: bool) -> impl View {
    let fixture = Fixture {
        rows: ReactiveList::from(
            (0..ROW_COUNT)
                .map(|index| Row { id: index as u64 })
                .collect::<Vec<_>>(),
        ),
        editing: binding(editing),
    };
    let list = List::for_each(fixture.rows.clone(), |row: Row| {
        ListItem::new(text(format!("Row {}", row.id)).padding())
    })
    .editing(fixture.editing.clone())
    .on_delete(delete_row)
    .on_move(move_row);
    vstack((list,)).state(&fixture)
}

fn save(app: &mut OffscreenApp, case: &str, stage: &str) {
    let _ = app.capture_snapshot("material3-preview", case, stage);
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn swipe_reveals_the_dismiss_background(ui: UiBuilder) {
    let mut app = ui.mount_offscreen(|| list_view(false));
    save(&mut app, "list-swipe", "rest");

    let bounds = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Row 1")
        .single()
        .bounds();
    let (center_x, center_y) = bounds.center();

    // Hold the row mid-swipe so the revealed background is on screen when the
    // frame is captured; a completed drag would already have committed.
    app.queue_pointer_down(center_x, center_y);
    app.queue_pointer_move(center_x - 40.0, center_y);
    save(&mut app, "list-swipe", "swipe-40px");
    app.queue_pointer_move(center_x - 120.0, center_y);
    save(&mut app, "list-swipe", "swipe-120px");
    app.queue_pointer_up(center_x - 120.0, center_y);
    save(&mut app, "list-swipe", "released");
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn reorder_drag_lifts_the_row(ui: UiBuilder) {
    let mut app = ui.mount_offscreen(|| list_view(true));
    save(&mut app, "list-reorder", "rest");

    let bounds = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Row 0")
        .single()
        .bounds();
    let row_height = bounds.height();
    let handle_x = bounds.x() + bounds.width() - 14.0;
    let start_y = bounds.y() + row_height / 2.0;

    app.queue_pointer_down(handle_x, start_y);
    app.queue_pointer_move(handle_x, row_height.mul_add(0.6, start_y));
    save(&mut app, "list-reorder", "lifted");
    app.queue_pointer_move(handle_x, row_height.mul_add(2.0, start_y));
    save(&mut app, "list-reorder", "two-rows-down");
    app.queue_pointer_up(handle_x, row_height.mul_add(2.0, start_y));
    save(&mut app, "list-reorder", "dropped");
}
