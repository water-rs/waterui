//! The navigation bar and tabs looks: a pushed page's back button follows
//! its pointer through the chrome the look gave it, and a tab selection moves
//! the tab container's live operands without recording anything again.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::LayoutDirection;
use waterui_interaction::interaction::InteractionState;
use waterui_layout::stack::vstack;
use waterui_navigation::tab::{Tab, Tabs};
use waterui_navigation::{
    NavigationLink, NavigationStack, NavigationToolbar, NavigationToolbarItem,
    NavigationToolbarPlacement, NavigationView,
};
use waterui_text::text;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::live_tabs::LiveTabs;
use crate::renderer::mount::layers::LayerVisitor;
use crate::renderer::recording::Recording;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 300;
const FRAME: Duration = Duration::from_millis(16);
const POINTER: u64 = 3;

struct Rig {
    runtime: HeadlessRuntime,
    now: Instant,
}

impl Rig {
    fn mount(view: AnyView) -> Self {
        Self::mount_with_env(view, test_environment())
    }

    fn mount_with_env(view: AnyView, env: Environment) -> Self {
        let view = RefCell::new(Some(view));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the test view is built once")
        });
        Self {
            runtime: HeadlessRuntime::new_for_tests(
                env,
                builder,
                WINDOW_WIDTH,
                WINDOW_HEIGHT,
                MinimalTestTheme::default(),
            ),
            now: Instant::now(),
        }
    }

    /// Frames past every motion the test theme declares.
    fn settle(&mut self) {
        for _ in 0..60 {
            self.now += FRAME;
            let _ = self.runtime.pump_at(false, self.now);
        }
    }

    fn pointer(&mut self, event: InputEvent) {
        self.runtime.push_input_event(event);
        self.settle();
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.pointer(InputEvent::PointerMove {
            id: POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
        });
    }

    fn down(&mut self, x: f32, y: f32) {
        self.pointer(InputEvent::PointerDown {
            id: POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    fn up(&mut self, x: f32, y: f32) {
        self.pointer(InputEvent::PointerUp {
            id: POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    /// The one live tab container the mount binds.
    fn tabs(&self) -> Rc<LiveTabs> {
        let mut tabs = self.runtime.renderer().hit_test.interaction.live_tabs();
        tabs.dedup_by(|a, b| Rc::ptr_eq(a, b));
        let [tabs] = <[_; 1]>::try_from(tabs).expect("one container binds one live tab set");
        tabs
    }

    /// The states every live chrome the mount binds shows.
    fn chrome_states(&self) -> Vec<InteractionState> {
        self.runtime
            .renderer()
            .hit_test
            .interaction
            .live_chromes()
            .iter()
            .map(|chrome| chrome.state())
            .collect()
    }
}

/// A pushed page's back button answers a hover and a press through the
/// chrome of the navigation look: the live chrome's state follows the
/// pointer.
#[test]
fn the_back_button_shows_interaction_feedback() {
    let mut rig = Rig::mount(AnyView::new(NavigationStack::new(NavigationView::new(
        "Root",
        vstack((NavigationLink::new("Open Detail", || {
            NavigationView::new("Detail", text("detail content"))
        }),)),
    ))));
    rig.settle();
    let update = rig
        .runtime
        .accessibility_tree()
        .expect("the rendered frame produced a tree");
    let (open, _) = find_by_label(&update, Role::Button, "Open Detail").expect("the link is shown");
    assert!(
        rig.runtime.perform_accessibility_action(ActionRequest {
            action: Action::Click,
            target_node: open,
            target_tree: TreeId::ROOT,
            data: None,
        }),
        "the link activates"
    );
    rig.settle();
    assert!(
        rig.chrome_states()
            .iter()
            .all(|state| !state.contains(InteractionState::HOVERED)),
        "nothing is hovered before the pointer arrives"
    );

    // The pushed page's back button is the test theme's 40 point square at
    // (4, 12).
    rig.move_to(24.0, 32.0);
    assert!(
        rig.chrome_states()
            .iter()
            .any(|state| state.contains(InteractionState::HOVERED)),
        "the back button is hovered under the pointer, got {:?}",
        rig.chrome_states()
    );

    rig.down(24.0, 32.0);
    assert!(
        rig.chrome_states()
            .iter()
            .any(|state| state.contains(InteractionState::PRESSED)),
        "the back button is pressed under the pointer, got {:?}",
        rig.chrome_states()
    );
}

fn two_tabs() -> AnyView {
    let tabs = (1..=2)
        .map(|i| {
            Tab::new(format!("Tab {i}"), move || {
                NavigationView::new(format!("Pane {i}"), text("pane"))
            })
        })
        .collect::<Vec<_>>();
    AnyView::new(Tabs::new(&nami::Binding::container(0usize), tabs))
}

/// Selecting a tab moves the container's operands to the new item and
/// records nothing: the highlight animates from live values.
#[test]
fn a_tab_selection_moves_the_highlight_and_records_nothing() {
    let mut rig = Rig::mount(two_tabs());
    rig.settle();

    let before = rig.tabs();
    assert_eq!(before.selected(), Some(0), "the first tab starts selected");
    let recorded = before.recordings();

    // The second item is the bar's right half: (200, 252)-(400, 300).
    rig.move_to(300.0, 276.0);
    rig.down(300.0, 276.0);
    rig.up(300.0, 276.0);
    rig.settle();

    let after = rig.tabs();
    assert!(Rc::ptr_eq(&before, &after), "the selection keeps the parts");
    assert_eq!(after.selected(), Some(1), "the second tab is selected");
    assert_eq!(
        after.recordings(),
        recorded,
        "a selection change records nothing"
    );
}

/// In RTL, a pushed page's back button sits on the leading (right) edge,
/// leading toolbar items sit beside it on the right, trailing toolbar items
/// sit on the left, the title sits between them, and tapping the right-side
/// back button pops the page.
#[test]
fn rtl_navigation_bar_mirrors_back_button_and_slots() {
    let mut env = test_environment();
    env.insert(LayoutDirection::RightToLeft);
    let mut rig = Rig::mount_with_env(
        AnyView::new(NavigationStack::new(NavigationView::new(
            "Root",
            vstack((NavigationLink::new("Open Detail", || {
                NavigationView::new("Detail", text("detail content")).navigation_toolbar(
                    NavigationToolbar::default()
                        .item(NavigationToolbarItem::action(
                            NavigationToolbarPlacement::TopBarLeading,
                            "Edit",
                            || {},
                        ))
                        .item(NavigationToolbarItem::action(
                            NavigationToolbarPlacement::TopBarTrailing,
                            "Done",
                            || {},
                        )),
                )
            }),)),
        ))),
        env,
    );
    rig.settle();
    let update = rig
        .runtime
        .accessibility_tree()
        .expect("the rendered frame produced a tree");
    let (open, _) = find_by_label(&update, Role::Button, "Open Detail").expect("the link is shown");
    assert!(
        rig.runtime.perform_accessibility_action(ActionRequest {
            action: Action::Click,
            target_node: open,
            target_tree: TreeId::ROOT,
            data: None,
        }),
        "the link activates"
    );
    rig.settle();

    let update = rig
        .runtime
        .accessibility_tree()
        .expect("the pushed frame produced a tree");
    let (_, back_node) = find_by_label(&update, Role::Button, "Back")
        .expect("the pushed page exposes a Back button");
    let back_bounds = back_node
        .bounds()
        .expect("the Back button has rendered bounds");
    // 400pt viewport, back_button_leading_inset=4, back_button_size=40 -> [356, 396].
    assert!(
        (back_bounds.x0 - 356.0).abs() < 1.0 && (back_bounds.x1 - 396.0).abs() < 1.0,
        "in RTL the back button must lie at [356, 396], got {back_bounds:?}"
    );

    let (_, leading_node) = find_by_label(&update, Role::Button, "Edit").expect("leading item");
    let (_, trailing_node) = find_by_label(&update, Role::Button, "Done").expect("trailing item");
    let (_, title_node) = find_by_label(&update, Role::Header, "Detail").expect("title header");
    let leading_bounds = leading_node.bounds().expect("leading item bounds");
    let trailing_bounds = trailing_node.bounds().expect("trailing item bounds");
    let title_bounds = title_node.bounds().expect("title bounds");

    assert!(
        leading_bounds.x1 <= back_bounds.x0,
        "in RTL the leading toolbar item ({leading_bounds:?}) must sit to the left of the right-side back button ({back_bounds:?})"
    );
    assert!(
        trailing_bounds.x1 <= title_bounds.x0 && title_bounds.x1 <= leading_bounds.x0,
        "in RTL the horizontal order must be trailing ({trailing_bounds:?}) <= title ({title_bounds:?}) <= leading ({leading_bounds:?})"
    );

    // Tapping the mirrored right-side back button pops back to Root.
    rig.move_to(376.0, 32.0);
    assert!(
        rig.chrome_states()
            .iter()
            .any(|state| state.contains(InteractionState::HOVERED)),
        "the RTL back button hovers under the right-side pointer"
    );
    rig.down(376.0, 32.0);
    rig.up(376.0, 32.0);
    let after_pop = rig
        .runtime
        .accessibility_tree()
        .expect("the popped frame produced a tree");
    assert!(
        find_by_label(&after_pop, Role::Button, "Open Detail").is_some(),
        "tapping the RTL back button on the right edge pops back to Root"
    );
}

#[derive(Default)]
struct GlyphColours(Vec<(f64, f64, Vec<[u8; 4]>)>);

impl<E: crate::text::TextEngine> LayerVisitor<E> for GlyphColours {
    fn run(&mut self, world: kurbo::Affine, recording: &Recording) {
        if let Some((transform, _)) = recording.glyph_runs().next() {
            let point = (world * transform).translation();
            self.0
                .push((point.x, point.y, recording.glyph_brush_colours()));
        }
    }
}

#[derive(Default)]
struct GlyphSpans(Vec<(f64, f64)>);

impl<E: crate::text::TextEngine> LayerVisitor<E> for GlyphSpans {
    fn run(&mut self, world: kurbo::Affine, recording: &Recording) {
        for (transform, glyphs) in recording.glyph_runs() {
            let y = (world * transform).translation().y;
            let span = match (glyphs.first(), glyphs.last()) {
                (Some(first), Some(last)) => f64::from(last.x - first.x),
                _ => 0.0,
            };
            self.0.push((y, span));
        }
    }
}

#[test]
fn navigation_bar_title_font_applies_to_inline_titles_and_keeps_large_titles() {
    use super::test_renderer_with_theme;
    use crate::renderer::RetainedSubview;
    use crate::renderer::mount::layers::visit;
    use waterui_core::layout::ProposalSize;
    use waterui_navigation::NavigationTitleDisplayMode;
    use waterui_text::font::{Body, Font, FontWeight};

    let env = test_environment();
    let max_bar_glyph_span = |theme: MinimalTestTheme, mode: NavigationTitleDisplayMode| -> f64 {
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(
                NavigationView::new("Dashboard", text::text("body"))
                    .navigation_title_display_mode(mode),
            )
        });
        let mut rig = Rig {
            runtime: HeadlessRuntime::new_for_tests(
                test_environment(),
                builder,
                WINDOW_WIDTH,
                WINDOW_HEIGHT,
                theme,
            ),
            now: Instant::now(),
        };
        rig.settle();
        let mut spans = GlyphSpans::default();
        visit(&rig.runtime.renderer().mount_roots(), &mut spans);
        spans
            .0
            .into_iter()
            .filter(|(y, _)| *y < 52.0)
            .map(|(_, span)| span)
            .fold(0.0, f64::max)
    };

    let custom_font = Font::new(Body).size(28.0).weight(FontWeight::SemiBold);
    let default_inline_span = max_bar_glyph_span(
        MinimalTestTheme::default(),
        NavigationTitleDisplayMode::Inline,
    );
    let themed_inline_span = max_bar_glyph_span(
        MinimalTestTheme {
            navigation_title_font: Some(custom_font.clone()),
            ..Default::default()
        },
        NavigationTitleDisplayMode::Inline,
    );
    assert!(
        themed_inline_span > default_inline_span * 1.4,
        "title_font must scale inline title glyphs (default {default_inline_span}, themed {themed_inline_span})"
    );

    let default_large_span = max_bar_glyph_span(
        MinimalTestTheme::default(),
        NavigationTitleDisplayMode::Large,
    );
    let themed_large_span = max_bar_glyph_span(
        MinimalTestTheme {
            navigation_title_font: Some(custom_font.clone()),
            ..Default::default()
        },
        NavigationTitleDisplayMode::Large,
    );
    approx::assert_relative_eq!(themed_large_span, default_large_span, epsilon = 1e-3);

    // Intrinsic measurement also reflects the title font override.
    let mut default_renderer = test_renderer_with_theme(MinimalTestTheme::default());
    let default_dims = RetainedSubview::new(
        NavigationView::new("Dashboard", text::text("body"))
            .navigation_title_display_mode(NavigationTitleDisplayMode::Inline),
    )
    .measure(&mut default_renderer, &env, ProposalSize::UNSPECIFIED);
    let mut themed_renderer = test_renderer_with_theme(MinimalTestTheme {
        navigation_title_font: Some(custom_font),
        ..Default::default()
    });
    let themed_dims = RetainedSubview::new(
        NavigationView::new("Dashboard", text::text("body"))
            .navigation_title_display_mode(NavigationTitleDisplayMode::Inline),
    )
    .measure(&mut themed_renderer, &env, ProposalSize::UNSPECIFIED);
    assert!(
        themed_dims.width > default_dims.width,
        "measured inline navigation width must reflect title_font ({} vs {})",
        themed_dims.width,
        default_dims.width
    );
}

fn compact_floating_tabs_look(
    bar_draws: &Rc<RefCell<Vec<kurbo::Rect>>>,
    divider_draws: &Rc<RefCell<Vec<kurbo::Rect>>>,
) -> crate::theme::TabsLook {
    use super::invisible_chrome;
    use crate::theme::{Custom, TabBarWidth, TabsLook};

    let bar_draws = Rc::clone(bar_draws);
    let divider_draws = Rc::clone(divider_draws);
    TabsLook::new(
        crate::testing::plain_tabs_metrics(),
        Custom::new((), move |_recorder, frame, ()| {
            bar_draws.borrow_mut().push(frame);
        }),
        Custom::new((), move |_recorder, frame, ()| {
            divider_draws.borrow_mut().push(frame);
        }),
        crate::testing::plain_tabs_motion(),
        |_item, _selected| invisible_chrome(),
    )
    .bar_width(TabBarWidth::Compact {
        item_min_width: 80.0,
        max_width: Some(320.0),
        horizontal_padding: 8.0,
    })
    .floating_margin(12.0)
}

#[test]
fn tabs_compact_floating_bar_centers_pill_and_reserves_bottom_margin() {
    use super::{capture_root_window, test_renderer_with_theme};
    use waterui_core::Binding;

    let env = test_environment();
    let window = kurbo::Rect::new(0.0, 0.0, 400.0, 300.0);
    let metrics = crate::testing::plain_tabs_metrics();
    let bar_draws = Rc::new(RefCell::new(Vec::new()));
    let divider_draws = Rc::new(RefCell::new(Vec::new()));
    let theme = MinimalTestTheme {
        tabs_look: Some(compact_floating_tabs_look(&bar_draws, &divider_draws)),
        ..Default::default()
    };
    let mut renderer = test_renderer_with_theme(theme);

    let tabs = Tabs::new(
        &Binding::container(0usize),
        ["Home", "Work"]
            .into_iter()
            .map(|name| Tab::new(name, move || NavigationView::new(name, text::text("pane"))))
            .collect::<Vec<_>>(),
    );
    capture_root_window(&mut renderer, tabs, &env, window);

    let bar_frame = {
        let bar_draws_ref = bar_draws.borrow();
        let [frame] = bar_draws_ref.as_slice() else {
            panic!("expected one bar draw, got {:?}", *bar_draws_ref);
        };
        *frame
    };
    // Bottom edge is inset by floating_margin (12.0) and height is bar_height (48.0).
    approx::assert_relative_eq!(bar_frame.y1, 288.0, epsilon = 1e-6);
    approx::assert_relative_eq!(bar_frame.height(), metrics.bar_height, epsilon = 1e-6);
    assert!(
        bar_frame.width() >= 176.0 && bar_frame.width() <= 320.0,
        "compact pill width out of range: {bar_frame:?}"
    );
    approx::assert_relative_eq!(bar_frame.center().x, 200.0, epsilon = 1e-6);

    let divider_frame = {
        let divider_draws_ref = divider_draws.borrow();
        let [frame] = divider_draws_ref.as_slice() else {
            panic!("expected one divider draw, got {:?}", *divider_draws_ref);
        };
        *frame
    };
    approx::assert_relative_eq!(divider_frame.x0, bar_frame.x0, epsilon = 1e-6);
    approx::assert_relative_eq!(divider_frame.x1, bar_frame.x1, epsilon = 1e-6);

    let update = renderer
        .take_accessibility_tree_update()
        .expect("accessibility update");
    let (_, tab_list) = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::TabList)
        .expect("tab list node");
    let list_bounds = tab_list.bounds().expect("tab list bounds");
    approx::assert_relative_eq!(list_bounds.x0, bar_frame.x0, epsilon = 1e-6);
    approx::assert_relative_eq!(list_bounds.x1, bar_frame.x1, epsilon = 1e-6);
    approx::assert_relative_eq!(list_bounds.y0, bar_frame.y0, epsilon = 1e-6);
    approx::assert_relative_eq!(list_bounds.y1, bar_frame.y1, epsilon = 1e-6);

    let (_, home) = find_by_label(&update, Role::Tab, "Home").expect("Home tab");
    let (_, work) = find_by_label(&update, Role::Tab, "Work").expect("Work tab");
    let home_bounds = home.bounds().expect("Home bounds");
    let work_bounds = work.bounds().expect("Work bounds");
    approx::assert_relative_eq!(home_bounds.x0, bar_frame.x0 + 8.0, epsilon = 1e-6);
    approx::assert_relative_eq!(work_bounds.x1, bar_frame.x1 - 8.0, epsilon = 1e-6);
}

#[test]
fn tabs_sidebar_ignores_compact_and_floating_margin() {
    use super::{capture_root_window, test_renderer_with_theme};
    use waterui_core::Binding;
    use waterui_navigation::tab::tab_style;

    let env = test_environment();
    let window = kurbo::Rect::new(0.0, 0.0, 400.0, 300.0);
    let bar_draws = Rc::new(RefCell::new(Vec::new()));
    let divider_draws = Rc::new(RefCell::new(Vec::new()));
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        tabs_look: Some(compact_floating_tabs_look(&bar_draws, &divider_draws)),
        ..Default::default()
    });
    let sidebar = Tabs::new(
        &Binding::container(0usize),
        ["Home", "Work"]
            .into_iter()
            .map(|name| Tab::new(name, move || NavigationView::new(name, text::text("pane"))))
            .collect::<Vec<_>>(),
    )
    .style(tab_style::Sidebar);
    capture_root_window(&mut renderer, sidebar, &env, window);
    let sidebar_draws = bar_draws.borrow();
    let [sidebar_frame] = sidebar_draws.as_slice() else {
        panic!("expected one sidebar draw, got {:?}", *sidebar_draws);
    };
    approx::assert_relative_eq!(sidebar_frame.x0, 0.0, epsilon = 1e-6);
    approx::assert_relative_eq!(sidebar_frame.y0, 0.0, epsilon = 1e-6);
    approx::assert_relative_eq!(sidebar_frame.y1, 300.0, epsilon = 1e-6);
}

#[test]
fn tabs_foreground_overrides_paint_selected_and_unselected_items_on_selection() {
    use super::invisible_chrome;
    use crate::renderer::mount::layers::visit;
    use crate::theme::{Custom, TabsLook};
    use waterui::color::Srgb;
    use waterui_controls::label::label;
    use waterui_core::Binding;

    let metrics = crate::testing::plain_tabs_metrics();
    let look = TabsLook::new(
        metrics,
        Custom::new((), |_recorder, _frame, ()| {}),
        Custom::new((), |_recorder, _frame, ()| {}),
        crate::testing::plain_tabs_motion(),
        |_item, _selected| invisible_chrome(),
    )
    .selected_foreground(Srgb::from_hex("#FF0000"))
    .unselected_foreground(Srgb::from_hex("#0000FF"));

    let selection = Binding::container(0usize);
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(Tabs::new(
            &selection,
            [("Home", "H"), ("Work", "W")]
                .into_iter()
                .map(|(title, icon)| {
                    Tab::new(label(title).icon(text::text(icon)), move || {
                        NavigationView::new(title, text::text("pane"))
                    })
                })
                .collect::<Vec<_>>(),
        ))
    });
    let mut rig = Rig {
        runtime: HeadlessRuntime::new_for_tests(
            test_environment(),
            builder,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            MinimalTestTheme {
                tabs_look: Some(look),
                ..Default::default()
            },
        ),
        now: Instant::now(),
    };
    rig.settle();

    let tab_colours = |runtime: &HeadlessRuntime| -> (Vec<[u8; 4]>, Vec<[u8; 4]>) {
        let mut visitor = GlyphColours::default();
        visit(&runtime.renderer().mount_roots(), &mut visitor);
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (x, y, colours) in visitor.0 {
            if colours.is_empty() || y < 250.0 {
                continue;
            }
            if x < 200.0 {
                left.extend(colours);
            } else {
                right.extend(colours);
            }
        }
        (left, right)
    };

    let (left0, right0) = tab_colours(&rig.runtime);
    assert!(
        !left0.is_empty() && !right0.is_empty(),
        "both tabs must draw glyphs: left={left0:?}, right={right0:?}"
    );
    assert!(
        left0.iter().all(|c| c[0] > 200 && c[2] < 50),
        "selected left tab must use red foreground, got {left0:?}"
    );
    assert!(
        right0.iter().all(|c| c[2] > 200 && c[0] < 50),
        "unselected right tab must use blue foreground, got {right0:?}"
    );

    // Click the second tab and verify foreground overrides follow selection.
    let tree = rig.runtime.accessibility_tree().expect("tree");
    let (work_id, _) = find_by_label(&tree, Role::Tab, "Work").expect("Work tab");
    assert!(rig.runtime.perform_accessibility_action(ActionRequest {
        action: Action::Click,
        target_node: work_id,
        target_tree: TreeId::ROOT,
        data: None,
    }));
    rig.settle();

    let (left1, right1) = tab_colours(&rig.runtime);
    assert!(
        left1.iter().all(|c| c[2] > 200 && c[0] < 50),
        "deselected left tab must switch to blue foreground, got {left1:?}"
    );
    assert!(
        right1.iter().all(|c| c[0] > 200 && c[2] < 50),
        "newly selected right tab must switch to red foreground, got {right1:?}"
    );
}
