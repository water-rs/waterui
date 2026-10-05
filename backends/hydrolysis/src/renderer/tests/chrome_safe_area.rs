//! §7.1 "Chrome" (water-rs/waterui#1905): a chrome container extends each bar
//! it draws under the regions of the edge the bar touches, to the window
//! edge — while the bar's title, items, search field and tab items keep
//! their laid-out frames inside the safe area.
//!
//! The surface's geometry is the bounds the renderer hands
//! `WidgetTheme::draw_navigation_bar` / `draw_tabs_bar`, which
//! [`MinimalTestTheme`] records verbatim — the recorded paint geometry, not
//! pixels. The bounds sit in the bar widget's own draw space (origin at the
//! widget's frame), so the window's edges land at `-top_inset` and
//! `content_height + bottom_inset` there; a surface that reaches them has
//! painted through the inset to the window edge. The content side is the
//! registered hit targets and the painted glyph positions, both in window
//! space.

use kurbo::{Affine, Point, Rect};
use nami::Binding;
use std::rc::Rc;
use std::time::Duration;
use waterui::ViewExt as _;
use waterui::component::text;
use waterui_controls::button::button;
use waterui_core::{AnyView, Environment, Str};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::vstack;
use waterui_navigation::tab::{Tab, Tabs};
use waterui_navigation::{
    NavigationPath, NavigationSplitView, NavigationStack, NavigationToolbar, NavigationToolbarItem,
    NavigationToolbarPlacement, NavigationView,
};

use super::{MinimalTestTheme, test_environment, test_renderer_with_theme};
use crate::platform::{WindowKeyboardArea, WindowSafeArea};
use crate::renderer::HydrolysisRenderer;

const WINDOW: Rect = Rect::new(0.0, 0.0, 390.0, 844.0);
const TOP_INSET: f32 = 48.0;
const BOTTOM_INSET: f32 = 34.0;
const KEYBOARD_INSET: f32 = 336.0;
/// The bars' theme height under [`MinimalTestTheme`]: `inline_bar_height`
/// for the bottom tool bar, `bar_height` for a bottom-docked tab bar.
const BAR_HEIGHT: f64 = 64.0;
const TAB_BAR_HEIGHT: f64 = 48.0;
/// Edge coordinates come through f32 layout arithmetic, so a position lands
/// within a fraction of the boundary it was computed from — the same
/// half-pixel resolution the touch test resolves at.
const EDGE_EPS: f64 = 1.0;

fn env_with_insets(container: EdgeInsets, keyboard: EdgeInsets) -> Environment {
    let mut env = test_environment();
    env.insert(WindowSafeArea(nami::binding(container)));
    env.insert(WindowKeyboardArea(nami::binding(keyboard)));
    env
}

/// The container insets every case here mounts under — a status-bar band on
/// top, a navigation-bar band on the bottom.
fn edge_insets() -> EdgeInsets {
    EdgeInsets::new(TOP_INSET, BOTTOM_INSET, 0.0, 0.0)
}

/// Renders one frame of the window tree in `window`; the tree is built by
/// the first frame and later frames reuse it, so `view` matters only the
/// first time.
fn render_frame_in(
    renderer: &mut HydrolysisRenderer,
    view: AnyView,
    env: &Environment,
    window: Rect,
) {
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(view, env, window, Affine::IDENTITY, Affine::IDENTITY);
    renderer.finish_rebuild_frame();
}

fn render_frame(renderer: &mut HydrolysisRenderer, view: AnyView, env: &Environment) {
    render_frame_in(renderer, view, env, WINDOW);
}

fn capture(
    view: impl waterui_core::View,
    env: &Environment,
    theme: MinimalTestTheme,
) -> HydrolysisRenderer {
    let mut renderer = test_renderer_with_theme(theme);
    render_frame(&mut renderer, AnyView::new(view), env);
    renderer
}

/// `capture` on a wider window — a `NavigationSplitView` stays in its
/// compact single-column layout below 680pt (`split_compact_threshold`),
/// which would hide the detail column these assertions read.
fn capture_in(
    view: impl waterui_core::View,
    env: &Environment,
    theme: MinimalTestTheme,
    window: Rect,
) -> HydrolysisRenderer {
    let mut renderer = test_renderer_with_theme(theme);
    render_frame_in(&mut renderer, AnyView::new(view), env, window);
    renderer
}

/// The y positions of every painted glyph in the window, plus the
/// window-space bounds of every registered pointer and text-input target —
/// the frames the bar's title, items, search field and tab items actually
/// got.
fn content_extents(renderer: &HydrolysisRenderer) -> (Vec<f64>, Vec<Rect>) {
    let mut ys = Vec::new();
    for recording in renderer
        .painted_recordings()
        .chain(std::iter::once(renderer.scene()))
    {
        for (transform, glyphs) in recording.glyph_runs() {
            for glyph in glyphs {
                let point = transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
                ys.push(point.y);
            }
        }
    }
    let mut bounds: Vec<Rect> = renderer
        .hit_test
        .pointer_targets
        .iter()
        .map(|target| target.bounds)
        .collect();
    bounds.extend(
        renderer
            .text_editing
            .text_input_targets
            .iter()
            .map(|target| target.frame),
    );
    (ys, bounds)
}

/// Asserts every painted glyph and interactive target stays vertically inside
/// `[top, bottom]` — the safe-area band the bar's content must not leave.
fn assert_content_inside(renderer: &HydrolysisRenderer, top: f64, bottom: f64, context: &str) {
    let (glyph_ys, targets) = content_extents(renderer);
    assert!(
        !glyph_ys.is_empty(),
        "{context}: expected painted text — nothing recorded any glyphs"
    );
    for y in glyph_ys {
        assert!(
            y >= top - EDGE_EPS && y <= bottom + EDGE_EPS,
            "{context}: a glyph painted at y={y}, outside the safe area {top}..={bottom}"
        );
    }
    for bounds in targets {
        assert!(
            bounds.y0 >= top - EDGE_EPS && bounds.y1 <= bottom + EDGE_EPS,
            "{context}: a hit target at {bounds:?} escaped the safe area {top}..={bottom}"
        );
    }
}

/// The navigation case: a `NavigationStack` whose view carries a trailing
/// toolbar item, a bottom tool-bar item and a search field.
fn navigation_view() -> impl waterui_core::View {
    let query = Binding::container(Str::from(""));
    NavigationStack::new(
        NavigationView::new("Inbox", text("mail body"))
            .navigation_toolbar(
                NavigationToolbar::default()
                    .item(NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarTrailing,
                        button("Add").action(|| {}),
                    ))
                    .item(NavigationToolbarItem::new(
                        NavigationToolbarPlacement::BottomBar,
                        button("Mark All").action(|| {}),
                    )),
            )
            .searchable(&query, "Search mail"),
    )
}

/// The top navigation bar's surface reaches the window's top edge through
/// the status-band inset and the bottom tool bar's reaches the bottom edge
/// through the navigation-band inset, while every painted glyph and hit
/// target stays inside the safe area.
#[test]
fn navigation_bar_surfaces_extend_through_the_insets_they_touch() {
    let theme = MinimalTestTheme::default();
    let bar_draws = Rc::clone(&theme.navigation_bar_draws);
    let separator_draws = Rc::clone(&theme.navigation_bar_separator_draws);
    let renderer = capture(
        navigation_view(),
        &env_with_insets(edge_insets(), EdgeInsets::default()),
        theme,
    );

    let draws = bar_draws.borrow();
    assert_eq!(
        draws.len(),
        2,
        "a searchable top bar and a bottom tool bar each draw once: {draws:?}"
    );
    // The bars draw in their widget's own space: the window's top edge sits
    // at `-TOP_INSET` and its bottom edge at `content_height + BOTTOM_INSET`.
    let content_height = WINDOW.height() - f64::from(TOP_INSET) - f64::from(BOTTOM_INSET);
    let window_bottom = content_height + f64::from(BOTTOM_INSET);
    let top = draws[0];
    assert!(
        (top.y0 + f64::from(TOP_INSET)).abs() <= EDGE_EPS,
        "the top bar's surface must reach the window's top edge through the \
         status band: {top:?}"
    );
    let bottom = draws[1];
    assert!(
        (bottom.y1 - window_bottom).abs() <= EDGE_EPS,
        "the bottom bar's surface must reach the window's bottom edge through \
         the navigation band: {bottom:?}"
    );
    assert!(
        (bottom.y0 - (content_height - BAR_HEIGHT)).abs() <= EDGE_EPS,
        "the bottom bar's inner edge stays at its laid-out frame: {bottom:?}"
    );

    // The separator is the bar's divider: it sits on the *inner* edge of the
    // bar's own frame, so the surface's inner edge equals it exactly — the
    // extension moved the painted edge, not the frame.
    let separator = separator_draws.borrow()[0];
    assert!(
        (separator.y1 - top.y1).abs() <= EDGE_EPS,
        "the separator stays on the bar's inner edge: {separator:?} vs {top:?}"
    );

    assert_content_inside(
        &renderer,
        f64::from(TOP_INSET),
        WINDOW.y1 - f64::from(BOTTOM_INSET),
        "navigation stack under insets",
    );
}

/// A `Tabs` view docked on the bottom, holding `NavigationView` pages: the
/// tab bar extends under the bottom inset, and the nested navigation bar
/// still extends under the top inset through the boundary the tab content
/// inherited.
#[test]
fn tab_bar_and_nested_navigation_bar_extend() {
    let theme = MinimalTestTheme::default();
    let tab_draws = Rc::clone(&theme.tabs_bar_draws);
    let nav_draws = Rc::clone(&theme.navigation_bar_draws);
    let selection = Binding::container(0i32);
    let renderer = capture(
        Tabs::new(
            &selection,
            vec![
                Tab::new(0, "First", || NavigationView::new("One", text("one"))),
                Tab::new(1, "Second", || NavigationView::new("Two", text("two"))),
            ],
        ),
        &env_with_insets(edge_insets(), EdgeInsets::default()),
        theme,
    );

    let content_height = WINDOW.height() - f64::from(TOP_INSET) - f64::from(BOTTOM_INSET);
    let window_bottom = content_height + f64::from(BOTTOM_INSET);
    let tab_draws = tab_draws.borrow();
    assert_eq!(tab_draws.len(), 1, "the tab bar draws once: {tab_draws:?}");
    let bar = tab_draws[0];
    assert!(
        (bar.y1 - window_bottom).abs() <= EDGE_EPS,
        "the tab bar's surface must reach the window's bottom edge: {bar:?}"
    );
    assert!(
        (bar.y0 - (content_height - TAB_BAR_HEIGHT)).abs() <= EDGE_EPS,
        "the tab bar's inner edge stays at its laid-out frame: {bar:?}"
    );

    // Chrome-hosted content inherits the boundary it touches: the selected
    // page's navigation bar still extends through the top inset (§7.1).
    let nav_draws = nav_draws.borrow();
    assert_eq!(
        nav_draws.len(),
        1,
        "the selected page's navigation bar draws once: {nav_draws:?}"
    );
    assert!(
        (nav_draws[0].y0 + f64::from(TOP_INSET)).abs() <= EDGE_EPS,
        "the page bar's surface must reach the window's top edge: {:?}",
        nav_draws[0]
    );

    assert_content_inside(
        &renderer,
        f64::from(TOP_INSET),
        WINDOW.y1 - f64::from(BOTTOM_INSET),
        "tabs under insets",
    );
}

/// The extension reaches through every region the edge carries: with the
/// keyboard region deeper than the container inset on the bottom edge, the
/// bottom tool bar's surface runs to the window edge through both. Where
/// the bar's frame sits under the keyboard is a separate question — the
/// surface is asserted against the window edge measured off the top
/// surface's own painted edge, not off keyboard arithmetic, so the test
/// survives whichever docking the bar takes.
#[test]
fn the_bottom_bar_extends_through_the_keyboard_region() {
    let theme = MinimalTestTheme::default();
    let bar_draws = Rc::clone(&theme.navigation_bar_draws);
    capture(
        navigation_view(),
        &env_with_insets(
            edge_insets(),
            EdgeInsets::new(0.0, KEYBOARD_INSET, 0.0, 0.0),
        ),
        theme,
    );

    let draws = bar_draws.borrow();
    let (top, bottom) = (draws[0], draws[1]);
    // The window's bottom edge in the bars' draw space is one window
    // height below the top surface's own painted edge — recorded data, not
    // a number derived from the keyboard inset.
    let window_bottom = top.y0 + WINDOW.height();
    assert!(
        (bottom.y1 - window_bottom).abs() <= EDGE_EPS,
        "the bottom bar's surface must reach the window edge through the \
         keyboard region: {bottom:?}"
    );
}

/// The realistic overlap case: a `NavigationView` with a bottom tool-bar
/// item inside `Tabs` content — the nested bottom bar ends on the tab bar's
/// inner edge but touches no boundary itself, so its surface keeps its own
/// frame exactly while the tab bar's surface reaches the window edge.
#[test]
fn a_nested_bottom_bar_inside_tab_content_does_not_extend() {
    let theme = MinimalTestTheme::default();
    let bar_draws = Rc::clone(&theme.navigation_bar_draws);
    let tab_draws = Rc::clone(&theme.tabs_bar_draws);
    let selection = Binding::container(0i32);
    let renderer = capture(
        Tabs::new(
            &selection,
            vec![
                Tab::new(0, "First", || {
                    NavigationView::new("Nested", text("nested page")).navigation_toolbar(
                        NavigationToolbar::default().item(NavigationToolbarItem::new(
                            NavigationToolbarPlacement::BottomBar,
                            button("Open").action(|| {}),
                        )),
                    )
                }),
                Tab::new(1, "Second", || NavigationView::new("Other", text("other"))),
            ],
        ),
        &env_with_insets(edge_insets(), EdgeInsets::default()),
        theme,
    );

    let content_height = WINDOW.height() - f64::from(TOP_INSET) - f64::from(BOTTOM_INSET);
    let tab_draws = tab_draws.borrow();
    assert_eq!(tab_draws.len(), 1, "the tab bar draws once: {tab_draws:?}");
    let tab = tab_draws[0];
    assert!(
        (tab.y1 - (content_height + f64::from(BOTTOM_INSET))).abs() <= EDGE_EPS,
        "the tab bar's surface must reach the window's bottom edge: {tab:?}"
    );

    // The nested page's bottom bar ends exactly where the tab bar begins —
    // the two draw in the same vertical space (both widgets' frames start
    // at the window's content top), so the numbers are directly comparable.
    let bar_draws = bar_draws.borrow();
    assert_eq!(
        bar_draws.len(),
        2,
        "the nested page's top bar and bottom bar draw once each: {bar_draws:?}"
    );
    let bottom = bar_draws[1];
    assert!(
        (bottom.y1 - tab.y0).abs() <= EDGE_EPS,
        "the nested bottom surface keeps its own frame — it ends at the tab \
         bar's inner edge instead of extending to the window edge: \
         {bottom:?} vs {tab:?}"
    );

    assert_content_inside(
        &renderer,
        f64::from(TOP_INSET),
        WINDOW.y1 - f64::from(BOTTOM_INSET),
        "nested navigation inside tabs",
    );
}

/// A split view's detail column touches the top boundary but not the
/// leading one — the detail bar's surface reaches the window's top edge
/// while its leading edge keeps the column's own boundary, never extending
/// across the sidebar.
#[test]
fn a_detail_columns_bar_does_not_extend_across_the_leading_edge() {
    const LEADING_INSET: f32 = 8.0;
    /// Wide enough to leave the compact single-column layout.
    const SPLIT_WINDOW: Rect = Rect::new(0.0, 0.0, 800.0, 844.0);

    let theme = MinimalTestTheme::default();
    let bar_draws = Rc::clone(&theme.navigation_bar_draws);
    let selection = Binding::container(Some(0i32));
    let _renderer = capture_in(
        NavigationSplitView::new(
            &selection,
            || NavigationView::new("Master", text("master")),
            |id| NavigationView::new(format!("Detail {id}"), text("detail")),
        ),
        &env_with_insets(
            EdgeInsets::new(TOP_INSET, BOTTOM_INSET, LEADING_INSET, 0.0),
            EdgeInsets::default(),
        ),
        theme,
        SPLIT_WINDOW,
    );

    let draws = bar_draws.borrow();
    assert_eq!(
        draws.len(),
        2,
        "master and detail columns each draw a top bar: {draws:?}"
    );
    // The master column's bar touches the leading boundary and extends
    // under the leading inset; the detail column's does not — its leading
    // edge is covered inside the window.
    let master = *draws
        .iter()
        .find(|bar| bar.x0 < -EDGE_EPS)
        .expect("the master column's bar extends through the leading inset");
    let detail = *draws
        .iter()
        .find(|bar| bar.x0 >= -EDGE_EPS)
        .expect("the detail column's bar draws");
    assert!(
        (master.x0 + f64::from(LEADING_INSET)).abs() <= EDGE_EPS,
        "the master column's bar reaches the window's leading edge: {master:?}"
    );
    assert!(
        detail.x0.abs() <= EDGE_EPS,
        "the detail column's bar must not extend across the leading edge — \
         its surface stays on its own frame: {detail:?}"
    );
    assert!(
        (detail.y0 + f64::from(TOP_INSET)).abs() <= EDGE_EPS,
        "the detail bar still extends through the top inset: {detail:?}"
    );
}

/// A top bar extends only through the edges a top bar can touch: on a
/// `NavigationView` shorter than the bar, `bar_rect.y1` clamps to the
/// view's bottom boundary — a boundary the view touches — and without the
/// docking mask the bar's colour would paint through the bottom inset.
#[test]
fn a_clamped_top_bar_never_extends_through_the_bottom_edge() {
    let theme = MinimalTestTheme::default();
    let bar_draws = Rc::clone(&theme.navigation_bar_draws);
    let _renderer = capture(
        vstack((
            spacer(),
            NavigationView::new("Docked", text("short")).size(390.0, 40.0),
        )),
        &env_with_insets(edge_insets(), EdgeInsets::default()),
        theme,
    );

    let draws = bar_draws.borrow();
    assert_eq!(draws.len(), 1, "one top bar draws: {draws:?}");
    let bar = draws[0];
    assert!(
        bar.height() <= 40.0 + EDGE_EPS,
        "a top bar on a view shorter than the bar never extends through the \
         bottom edge — the surface keeps its clamped frame: {bar:?}"
    );
}

/// A navigation transition's page clips must cover what the pages paint:
/// the bars' surfaces reach the window edge, so a clip cut to the stack's
/// own bounds drops the status band to the window background for the
/// animation's duration. One pumped frame at each side of the fade-through
/// — the outgoing page's clip early, the incoming's late — both reach the
/// window edges.
#[test]
fn a_transitioning_page_clip_covers_the_extended_surfaces() {
    let theme = MinimalTestTheme::default();
    let bar_draws = Rc::clone(&theme.navigation_bar_draws);
    let path = NavigationPath::<u8>::new();
    path.push(1);
    let mut renderer = test_renderer_with_theme(theme);
    let env = env_with_insets(edge_insets(), EdgeInsets::default());
    render_frame(
        &mut renderer,
        AnyView::new(
            NavigationStack::with_path(path, NavigationView::new("Root", text("root")))
                .destination(|_| NavigationView::new("Detail", text("detail"))),
        ),
        &env,
    );

    // Pumped clock, never slept: +32ms shows the outgoing page mid-flight,
    // +200ms more the incoming one.
    for elapsed in [32_u64, 200] {
        let start = renderer.frame_instant();
        renderer.set_frame_instant(
            start
                .checked_add(Duration::from_millis(elapsed))
                .expect("test frame instant overflow"),
        );
        render_frame(&mut renderer, AnyView::new(()), &env);

        // Every clip scope the frame pushed, in window space — the page
        // scopes among them are stack-sized and must cover the edges their
        // surfaces paint to.
        let clips: Vec<Rect> = renderer
            .painted_recordings()
            .chain(std::iter::once(renderer.scene()))
            .flat_map(crate::renderer::recording::Recording::clip_scopes)
            .map(|(transform, clip)| transform.transform_rect_bbox(clip))
            .collect();
        assert!(
            clips.iter().any(|clip| {
                clip.width() >= WINDOW.width() / 2.0
                    && clip.height() >= WINDOW.height() / 2.0
                    && clip.y0 <= WINDOW.y0 + EDGE_EPS
                    && clip.y1 >= WINDOW.y1 - EDGE_EPS
            }),
            "a transitioning page's clip must reach the window edges its bar \
             surfaces paint to — clip rects at +{elapsed}ms: {clips:?}"
        );
    }

    // The stack's backdrop fill covers the same reach the clips do — a
    // stack-sized fill that reaches the window edges, so an extended bar
    // surface never lands on the window background.
    let fills: Vec<Rect> = renderer
        .painted_recordings()
        .chain(std::iter::once(renderer.scene()))
        .flat_map(crate::renderer::recording::Recording::fill_bounds)
        .map(|(transform, shape)| transform.transform_rect_bbox(shape))
        .collect();
    assert!(
        fills.iter().any(|fill| {
            fill.width() >= WINDOW.width() / 2.0
                && fill.height() >= WINDOW.height() / 2.0
                && fill.y0 <= WINDOW.y0 + EDGE_EPS
                && fill.y1 >= WINDOW.y1 - EDGE_EPS
        }),
        "the stack's background fill must reach the window edges its pages' \
         surfaces paint to: {fills:?}"
    );

    // And the pages' bars keep drawing the extended surface inside those
    // clips — recorded in each page's own space, where the window's top
    // edge sits at `-TOP_INSET`.
    assert!(
        bar_draws
            .borrow()
            .iter()
            .all(|bar| bar.y0 <= -f64::from(TOP_INSET) + EDGE_EPS),
        "every page's top bar keeps its extension mid-transition: {:?}",
        bar_draws.borrow()
    );

    // The offset space is the painted reach too: a custom transition
    // reports offsets as fractions of the viewport, so `offset_y = 1` must
    // move the page's whole painted reach — extended surfaces included —
    // off the window. Counted against `bounds` instead, the incoming
    // page's extended top band would stay visible below the bottom edge.
    let theme = MinimalTestTheme::default();
    let path = NavigationPath::<u8>::new();
    path.push(1);
    let mut renderer = test_renderer_with_theme(theme);
    render_frame(
        &mut renderer,
        AnyView::new(
            NavigationStack::with_path(path, NavigationView::new("Root", text("root")))
                .destination(|_| NavigationView::new("Detail", text("detail")))
                .transition(SlideUpTransition),
        ),
        &env,
    );
    let start = renderer.frame_instant();
    renderer.set_frame_instant(
        start
            .checked_add(Duration::from_millis(32))
            .expect("test frame instant overflow"),
    );
    render_frame(&mut renderer, AnyView::new(()), &env);
    let clips: Vec<Rect> = renderer
        .painted_recordings()
        .chain(std::iter::once(renderer.scene()))
        .flat_map(crate::renderer::recording::Recording::clip_scopes)
        .map(|(transform, clip)| transform.transform_rect_bbox(clip))
        .collect();
    assert!(
        clips.iter().all(|clip| {
            (clip.y0 <= WINDOW.y0 + EDGE_EPS && clip.y1 >= WINDOW.y1 - EDGE_EPS)
                || clip.y0 >= WINDOW.y1 - EDGE_EPS
        }),
        "a page slid one painted viewport off-screen leaves nothing inside \
         the window — clip rects: {clips:?}"
    );
}

/// A custom transition whose incoming page arrives from one viewport below:
/// the offset-1 case — an `offset_y` counted against `bounds` instead of
/// the painted reach would leave the page's extended top band on screen.
#[derive(Debug)]
struct SlideUpTransition;

impl waterui_navigation::NavigationTransition for SlideUpTransition {
    fn frame(
        &self,
        _progress: f32,
        _direction: waterui_navigation::NavigationTransitionDirection,
    ) -> waterui_navigation::NavigationTransitionFrame {
        waterui_navigation::NavigationTransitionFrame {
            outgoing: waterui_navigation::NavigationTransitionLayer::IDENTITY,
            incoming: waterui_navigation::NavigationTransitionLayer {
                offset_y: 1.0,
                ..waterui_navigation::NavigationTransitionLayer::IDENTITY
            },
        }
    }
}
