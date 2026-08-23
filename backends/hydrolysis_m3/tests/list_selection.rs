//! Selection chrome on `List` rows.
//!
//! The self-drawn renderer ignored `ListItem::selected` outright. The row's
//! own content already flipped to `SelectionForeground` — that part is the
//! list component's job and always worked — but nothing painted the container
//! behind it and nothing told the accessibility tree, so a sidebar had no way
//! to show which row was current.

use waterui::component::list::{List, ListItem};
use waterui::prelude::*;
use waterui::reactive::{SignalExt, binding};
use waterui_testing::UiBuilder;

const LABELS: [&str; 3] = ["First", "Second", "Third"];

#[expect(
    clippy::needless_pass_by_value,
    reason = "the binding is cloned into the mounted body, which must own it"
)]
fn selectable_list(current: Binding<usize>) -> impl View {
    let row = |index: usize| {
        let current = current.clone();
        move || {
            ListItem::new(text(LABELS[index]))
                .selected(current.clone().map(move |value| value == index))
        }
    };
    List::content((row(0), row(1), row(2)))
}

/// Exactly the selected row reports itself selected, and the flag follows the
/// signal without the list being rebuilt.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 300))]
fn only_the_selected_row_is_marked_selected(ui: UiBuilder) {
    let current = binding(1usize);
    let mut app = ui.mount({
        let current = current.clone();
        move || selectable_list(current.clone())
    });
    app.settle();

    let _ = app.query().label("First").selected(false).single();
    let _ = app.query().label("Second").selected(true).single();
    let _ = app.query().label("Third").selected(false).single();

    current.set(2);
    app.settle();

    let _ = app.query().label("Second").selected(false).single();
    let _ = app.query().label("Third").selected(true).single();
}

/// Visual acceptance: the selected row carries the theme's selection fill and
/// its content reads against that fill, while its neighbours are untouched.
#[ignore = "writes a visual acceptance PNG for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 300))]
fn the_selected_row_shows_its_selection_fill(ui: UiBuilder) {
    let current = binding(1usize);
    let mut app = ui.mount_offscreen(move || selectable_list(current.clone()));
    let _ = app.capture_snapshot("material3-preview", "list-selection", "second-selected");
}
