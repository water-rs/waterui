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
