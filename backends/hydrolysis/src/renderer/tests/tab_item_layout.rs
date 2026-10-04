//! Tab item layout tests: the renderer asks the theme's `tabs_item_layout`
//! hook with the bar's width and item count, and the highlight covers the
//! whole item when the theme answers `Horizontal`.

use super::*;
use waterui_backend_core::widget::TabItemLayout;
use waterui_core::id::Id;

use waterui_navigation::tab::{Tab, TabsLayout, tab_style};

fn tabs_view(count: usize) -> TabsLayout {
    let selection = Binding::container(Id::try_from(1).expect("non-zero tab id"));
    let tabs = (1..=count)
        .map(|i| {
            let id = Id::try_from(crate::num_cast::usize_as_i32(i)).expect("non-zero tab id");
            Tab::new(id, format!("Tab {i}"), move || {
                NavigationView::new(format!("Pane {i}"), text!("pane"))
            })
        })
        .collect();
    TabsLayout::new(selection, tabs)
}

/// Tabs whose items carry a text icon beside the title, so the item's
/// content extent is icon + spacing + label exactly.
fn icon_tabs_view(count: usize, icon: &str) -> TabsLayout {
    let selection = Binding::container(Id::try_from(1).expect("non-zero tab id"));
    let icon = icon.to_owned();
    let tabs = (1..=count)
        .map(|i| {
            let id = Id::try_from(crate::num_cast::usize_as_i32(i)).expect("non-zero tab id");
            Tab::new(
                id,
                label(format!("Tab {i}")).icon(text(icon.clone())),
                move || NavigationView::new(format!("Pane {i}"), text!("pane")),
            )
        })
        .collect();
    TabsLayout::new(selection, tabs)
}

fn sidebar_tabs_view(count: usize) -> TabsLayout {
    tabs_view(count).style(tab_style::Sidebar)
}

/// Intrinsic width of a plain-text view, measured the same way the
/// renderer measures a tab's icon or label.
fn measure_plain(renderer: &mut HydrolysisRenderer, env: &Environment, s: &str) -> f64 {
    f64::from(
        HydrolysisRenderer::measure_text_intrinsic_size(
            renderer.state_mut(),
            waterui_text::styled::StyledStr::plain(s.to_owned()),
            env,
        )
        .width,
    )
}

fn theme_with_layout(layout: Option<TabItemLayout>) -> MinimalTestTheme {
    MinimalTestTheme {
        forced_tab_item_layout: layout,
        ..Default::default()
    }
}

#[test]
fn renderer_asks_the_layout_hook_with_bar_width_and_item_count() {
    let env = test_environment();
    let queries = Rc::new(RefCell::new(Vec::new()));
    let mut theme = theme_with_layout(None);
    theme.tabs_layout_queries = Rc::clone(&queries);
    let mut renderer = test_renderer_with_theme(theme);

    capture_root_window(
        &mut renderer,
        tabs_view(3),
        &env,
        Rect::new(0.0, 0.0, 480.0, 320.0),
    );

    let queries = queries.borrow();
    assert!(
        queries.contains(&(480.0, 3)),
        "tabs_item_layout must see the bar width and item count, got {queries:?}"
    );
}

#[test]
#[expect(
    clippy::float_cmp,
    reason = "the highlight bounds are exact layout constants"
)]
fn vertical_layout_draws_the_label_strip_highlight() {
    let env = test_environment();
    let draws = Rc::new(RefCell::new(Vec::new()));
    let mut theme = theme_with_layout(None);
    theme.tabs_highlight_draws = Rc::clone(&draws);
    let mut renderer = test_renderer_with_theme(theme);

    capture_root_window(
        &mut renderer,
        tabs_view(2),
        &env,
        Rect::new(0.0, 0.0, 400.0, 300.0),
    );

    // The first tab is selected: bar (0,252)-(400,300), its item the left
    // half, and the highlight is the 3-high strip at the item's top.
    let draws = draws.borrow();
    let [(bounds, layout)] = draws.as_slice() else {
        panic!("expected one highlight draw, got {draws:?}");
    };
    assert_eq!(*layout, TabItemLayout::Vertical);
    assert_eq!(bounds.y0, 252.0);
    assert_eq!(bounds.y1, 255.0);
    assert!(bounds.x0 > 0.0 && bounds.x1 < 200.0);
}

#[test]
fn horizontal_layout_highlights_the_whole_item() {
    let env = test_environment();
    let draws = Rc::new(RefCell::new(Vec::new()));
    let mut theme = theme_with_layout(Some(TabItemLayout::Horizontal));
    theme.tabs_highlight_draws = Rc::clone(&draws);
    let mut renderer = test_renderer_with_theme(theme);

    capture_root_window(
        &mut renderer,
        tabs_view(2),
        &env,
        Rect::new(0.0, 0.0, 400.0, 300.0),
    );

    // The highlight hugs the selected item's content — icon + spacing +
    // label grown by the button inset on each side — centered in the item,
    // with the theme's 40-high indicator metric.
    let draws = draws.borrow();
    let [(bounds, layout)] = draws.as_slice() else {
        panic!("expected one highlight draw, got {draws:?}");
    };
    assert_eq!(*layout, TabItemLayout::Horizontal);
    let button = Rect::new(0.0, 252.0, 200.0, 300.0);
    assert!(bounds.y0 > button.y0 && bounds.y1 < button.y1);
    assert!(bounds.x0 >= button.x0 && bounds.x1 <= button.x1);
    assert!(bounds.width() < button.width());
}

/// A horizontal item's indicator hugs its content: icon + icon-label
/// spacing + label, grown by `button_horizontal_inset` on each side and
/// centered in the button — never the button's full share of the bar. At
/// 1400pt with 4 items the button share is 350pt, far wider than the
/// content.
#[test]
fn horizontal_highlight_hugs_icon_spacing_label_not_the_button() {
    const ICON_TEXT: &str = "·";

    let env = test_environment();
    let draws = Rc::new(RefCell::new(Vec::new()));
    let mut theme = theme_with_layout(Some(TabItemLayout::Horizontal));
    theme.tabs_highlight_draws = Rc::clone(&draws);
    let mut renderer = test_renderer_with_theme(theme);

    capture_root_window(
        &mut renderer,
        icon_tabs_view(4, ICON_TEXT),
        &env,
        Rect::new(0.0, 0.0, 1400.0, 300.0),
    );

    let draws = draws.borrow();
    let [(bounds, layout)] = draws.as_slice() else {
        panic!("expected one highlight draw, got {draws:?}");
    };
    assert_eq!(*layout, TabItemLayout::Horizontal);
    // Button share: 1400/4 = 350 wide; the highlight must instead be the
    // content — icon + icon_label_spacing + label — grown by
    // button_horizontal_inset on each side.
    let metrics = MinimalTestTheme::default().tabs_metrics(TabItemLayout::Horizontal);
    let icon_width = measure_plain(&mut renderer, &env, ICON_TEXT);
    let label_width = measure_plain(&mut renderer, &env, "Tab 1");
    let expected = f64::mul_add(
        metrics.button_horizontal_inset,
        2.0,
        icon_width + metrics.icon_label_spacing + label_width,
    );
    assert!(
        (bounds.width() - expected).abs() < 0.5,
        "highlight must hug icon + spacing + label + 2×inset ({expected}), got {}",
        bounds.width()
    );
    assert!(
        bounds.width() < 1400.0 / 4.0,
        "highlight {} must be narrower than the 350-wide button share",
        bounds.width()
    );
}

/// A `Sidebar` bar is a strip `bar_height` wide along its item axis: a
/// width-class theme must see that thickness — not the window width — or it
/// would answer `Horizontal` for a wide window and break the strip's items
/// and highlight.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "the comparison is exact by design — the value originates from a literal fixture, not accumulated arithmetic"
)]
fn sidebar_tabs_query_the_layout_hook_with_their_strip_extent() {
    let env = test_environment();
    let queries = Rc::new(RefCell::new(Vec::new()));
    let draws = Rc::new(RefCell::new(Vec::new()));
    let mut theme = theme_with_layout(None);
    // A width-class theme like M3's: horizontal items from 600 up.
    theme.horizontal_from_width = Some(600.0);
    theme.tabs_layout_queries = Rc::clone(&queries);
    theme.tabs_highlight_draws = Rc::clone(&draws);
    let mut renderer = test_renderer_with_theme(theme);

    capture_root_window(
        &mut renderer,
        sidebar_tabs_view(3),
        &env,
        Rect::new(0.0, 0.0, 800.0, 320.0),
    );

    let queries = queries.borrow();
    assert!(
        !queries.is_empty() && queries.iter().all(|(width, _)| *width == 48.0),
        "the sidebar must report its strip width, got {queries:?}"
    );
    // The strip (0,0)-(48,320) keeps vertical items, so the highlight hugs
    // the item's trailing edge rather than spanning it.
    let draws = draws.borrow();
    let [(bounds, layout)] = draws.as_slice() else {
        panic!("expected one highlight draw, got {draws:?}");
    };
    assert_eq!(*layout, TabItemLayout::Vertical);
    assert_eq!(bounds.x0, 45.0);
    assert_eq!(bounds.x1, 48.0);
}
