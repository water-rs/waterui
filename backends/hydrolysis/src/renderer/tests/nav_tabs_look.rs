//! The navigation bar and tabs looks: a pushed page's back button follows
//! its pointer through the chrome the look gave it, and a tab selection moves
//! the tab container's live operands without recording anything again.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::InteractionState;
use waterui_core::views::{SharedAnyViews, ViewSnapshot as _, Views as _};
use waterui_layout::stack::vstack;
use waterui_navigation::tab::{Tab, TabsLayout};
use waterui_navigation::{NavigationLink, NavigationStack, NavigationView};
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
        let view = RefCell::new(Some(view));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the test view is built once")
        });
        Self {
            runtime: HeadlessRuntime::new_for_tests(
                test_environment(),
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
    let tabs = SharedAnyViews::new(
        (1..=2)
            .map(|i| {
                Tab::new(format!("Tab {i}"), move || {
                    NavigationView::new(format!("Pane {i}"), text("pane"))
                })
            })
            .collect::<Vec<_>>(),
    );
    let selection = nami::Binding::container(
        tabs.snapshot()
            .get_id(0)
            .expect("a non-empty tab set reports an id"),
    );
    AnyView::new(TabsLayout::new(selection, tabs))
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
