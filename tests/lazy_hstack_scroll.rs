//! `Lazy::hstack` and `Lazy::hstack_spaced` are horizontally scrolling lazy
//! rows: wheel-scrolling along x brings later rows into view.

use waterui::component::{Text, lazy::Lazy, text};
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

fn items() -> Vec<Text> {
    (0..30).map(|i| text(format!("Item {i}"))).collect()
}

#[test]
fn lazy_hstack_scrolls_horizontally() {
    let mut app = ui()
        .viewport(400, 300)
        .theme(TestStyle::new())
        .mount_offscreen(|| Lazy::hstack(items()));
    app.settle();
    let before = rows(&app);
    app.scroll_at(200.0, 150.0, -600.0, 0.0, false);
    app.settle();
    let after = rows(&app);
    assert_ne!(
        before, after,
        "horizontal wheel scroll must move the row of Lazy::hstack"
    );
}

#[test]
fn lazy_hstack_spaced_scrolls_horizontally() {
    let mut app = ui()
        .viewport(400, 300)
        .theme(TestStyle::new())
        .mount_offscreen(|| Lazy::hstack_spaced(8.0, items()));
    app.settle();
    let before = rows(&app);
    app.scroll_at(200.0, 150.0, -600.0, 0.0, false);
    app.settle();
    let after = rows(&app);
    assert_ne!(
        before, after,
        "horizontal wheel scroll must move the row of Lazy::hstack_spaced"
    );
}
