//! Row-level drag gestures on `List`: swipe-to-dismiss and reorder-by-handle.
//!
//! Both are driven through the accessibility tree rather than by inspecting
//! pixels, so a failure points at the interaction rather than at rendering.

use std::cell::RefCell;
use std::rc::Rc;

use waterui::Identifiable;
use waterui::component::list::{List, ListDelete, ListItem, ListMove};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::reactive::collection::List as ReactiveList;
use waterui_testing::{DragOptions, Role, UiBuilder};

const ROW_COUNT: usize = 6;
const VIEWPORT_WIDTH: f32 = 400.0;

#[derive(Clone, Copy, Identifiable)]
struct Row {
    #[id]
    id: u64,
}

#[derive(Clone)]
struct Fixture {
    rows: ReactiveList<Row>,
    editing: Binding<bool>,
    /// Every reorder the backend committed, oldest first.
    moves: Rc<RefCell<Vec<(usize, usize)>>>,
    /// Every deletion the backend committed, oldest first.
    deletes: Rc<RefCell<Vec<usize>>>,
}

impl Fixture {
    fn new(editing: bool) -> Self {
        let rows = (0..ROW_COUNT)
            .map(|index| Row { id: index as u64 })
            .collect::<Vec<_>>();
        Self {
            rows: ReactiveList::from(rows),
            editing: binding(editing),
            moves: Rc::new(RefCell::new(Vec::new())),
            deletes: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Row labels in their current order, e.g. `["Row 0", "Row 1", ...]`.
    fn order(&self) -> Vec<String> {
        self.rows
            .snapshot()
            .into_iter()
            .map(|row| format!("Row {}", row.id))
            .collect()
    }
}

fn delete_row(ListDelete(index): ListDelete, State(fixture): State<Fixture>) {
    fixture.deletes.borrow_mut().push(index);
    let _ = fixture.rows.remove(index);
}

fn move_row(ListMove(movement): ListMove, State(fixture): State<Fixture>) {
    fixture
        .moves
        .borrow_mut()
        .push((movement.from(), movement.to()));
    let mut rows = fixture.rows.snapshot();
    let row = rows.remove(movement.from());
    rows.insert(movement.to(), row);
    let _ = fixture.rows.replace(rows);
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the fixture is installed into the view environment, which must own it"
)]
fn list_view(fixture: Fixture) -> impl View {
    let list = List::for_each(fixture.rows.clone(), |row: Row| {
        ListItem::new(text(format!("Row {}", row.id)).padding())
    })
    .editing(fixture.editing.clone())
    .on_delete(delete_row)
    .on_move(move_row);
    vstack((list,)).state(&fixture)
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn swiping_a_row_past_the_threshold_deletes_it(ui: UiBuilder) {
    let fixture = Fixture::new(false);
    let probe = fixture.clone();
    let mut app = ui.mount(move || list_view(fixture.clone()));

    app.query()
        .role(Role::LIST_ITEM)
        .label("Row 1")
        .assert_exists();

    // Past Material's positional threshold (half the row width), so release
    // commits the dismissal.
    app.query()
        .role(Role::LIST_ITEM)
        .label("Row 1")
        .drag_by(-VIEWPORT_WIDTH * 0.75, 0.0);

    assert_eq!(
        probe.deletes.borrow().as_slice(),
        &[1],
        "swipe past the threshold should delete exactly the swiped row"
    );
    assert_eq!(
        probe.order(),
        vec!["Row 0", "Row 2", "Row 3", "Row 4", "Row 5"],
        "the swiped row should be gone and the rest should keep their order"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn swiping_a_row_short_of_the_threshold_keeps_it(ui: UiBuilder) {
    let fixture = Fixture::new(false);
    let probe = fixture.clone();
    let mut app = ui.mount(move || list_view(fixture.clone()));

    // Beyond the touch slop so a drag is recognized, but nowhere near half the
    // row: the row must spring back instead of committing.
    app.query()
        .role(Role::LIST_ITEM)
        .label("Row 1")
        .drag_by(-40.0, 0.0);

    assert!(
        probe.deletes.borrow().is_empty(),
        "a swipe short of the threshold must not delete: {:?}",
        probe.deletes.borrow()
    );
    assert_eq!(probe.order().len(), ROW_COUNT);
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn a_press_inside_the_touch_slop_is_not_a_swipe(ui: UiBuilder) {
    let fixture = Fixture::new(false);
    let probe = fixture.clone();
    let mut app = ui.mount(move || list_view(fixture.clone()));

    app.query()
        .role(Role::LIST_ITEM)
        .label("Row 2")
        .drag_by(-2.0, 0.0);

    assert!(
        probe.deletes.borrow().is_empty(),
        "movement inside the touch slop must not start a swipe"
    );
    assert_eq!(probe.order().len(), ROW_COUNT);
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn dragging_the_move_handle_reorders_the_row(ui: UiBuilder) {
    let fixture = Fixture::new(true);
    let probe = fixture.clone();
    let mut app = ui.mount(move || list_view(fixture.clone()));

    let bounds = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Row 0")
        .single()
        .bounds();
    let row_height = bounds.height();
    // The move handle sits just inside the row's trailing edge.
    let handle_x = bounds.x() + bounds.width() - 14.0;
    let start_y = bounds.y() + row_height / 2.0;

    app.drag_from_to_with(
        handle_x,
        start_y,
        handle_x,
        row_height.mul_add(2.0, start_y),
        DragOptions::default(),
    );

    assert_eq!(
        probe.moves.borrow().as_slice(),
        &[(0, 2)],
        "dragging down two rows should commit a single 0 -> 2 move"
    );
    assert_eq!(
        probe.order(),
        vec!["Row 1", "Row 2", "Row 0", "Row 3", "Row 4", "Row 5"],
        "the dragged row should land two slots down"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 420))]
fn rows_stay_reachable_in_the_accessibility_tree(ui: UiBuilder) {
    let fixture = Fixture::new(true);
    let mut app = ui.mount(move || list_view(fixture.clone()));

    app.query().role(Role::LIST).assert_exists();
    for index in 0..ROW_COUNT {
        app.query()
            .role(Role::LIST_ITEM)
            .label(format!("Row {index}"))
            .assert_exists();
    }
}
