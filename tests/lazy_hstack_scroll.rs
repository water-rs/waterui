//! `scroll_horizontal(HStack::for_each(...))` virtualizes horizontally:
//! wheel-scrolling along x brings later rows into view.

use core::array;
use waterui::component::{HStack, text};
use waterui::layout::scroll::scroll_horizontal;
use waterui_testing::{OffscreenApp, TestStyle, ui};

fn rows(app: &OffscreenApp) -> Vec<(String, f32)> {
    let mut rows: Vec<(String, f32)> = app
        .tree()
        .nodes()
        .values()
        .filter_map(|n| {
            let label = n.label()?;
            label.starts_with("Item ").then_some(())?;
            Some((label.to_owned(), n.bounds()?.x()))
        })
        .collect();
    rows.sort_by(|a, b| a.1.total_cmp(&b.1));
    rows
}

fn item_indices() -> [usize; 30] {
    array::from_fn(|i| i)
}

#[test]
fn lazy_hstack_scrolls_horizontally() {
    let mut app = ui()
        .viewport(400, 300)
        .theme(TestStyle::new())
        .mount_offscreen(|| {
            scroll_horizontal(HStack::for_each(item_indices(), |i| {
                text(format!("Item {i}"))
            }))
        });
    app.settle();
    let before = rows(&app);
    app.scroll_at(200.0, 150.0, -600.0, 0.0, false);
    app.settle();
    let after = rows(&app);
    assert_ne!(
        before, after,
        "horizontal wheel scroll must move the row of scroll_horizontal(HStack::for_each)"
    );
}

#[test]
fn lazy_hstack_spaced_scrolls_horizontally() {
    let mut app = ui()
        .viewport(400, 300)
        .theme(TestStyle::new())
        .mount_offscreen(|| {
            scroll_horizontal(
                HStack::for_each(item_indices(), |i| text(format!("Item {i}"))).spacing(8.0),
            )
        });
    app.settle();
    let before = rows(&app);
    app.scroll_at(200.0, 150.0, -600.0, 0.0, false);
    app.settle();
    let after = rows(&app);
    assert_ne!(
        before, after,
        "horizontal wheel scroll must move the row of spaced scroll_horizontal(HStack::for_each)"
    );
}
