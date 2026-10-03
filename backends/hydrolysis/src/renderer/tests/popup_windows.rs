//! Popup-window accessibility merge in the rendered runtime.
//!
//! `HeadlessRuntime::pump_at` returns one tree update for every open
//! window — the main window and each popup — exactly as
//! `SemanticRuntime`'s merged update does. A context menu opened by a
//! secondary click mounts as a popup window whose nodes used to stay in
//! the popup core's own pending update, unreachable by label.

use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeUpdate};
use nami::Binding;
use nami::Signal as _;
use waterui::ViewExt as _;
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_form::picker::color::ColorPicker;
use waterui_graphics::Color;
use waterui_layout::frame::Frame;
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const WINDOW_SIZE: f32 = 160.0;

/// The platform hold threshold `CONTEXT_MENU_HOLD_DURATION` implements
/// (hit_`hit_test.rs)`0ms on every platform. Kept local so the test compiles
/// on the pre-fix tree for the fail-before run.
const HOLD: Duration = Duration::from_millis(500);

/// The node labelled `label` with `role`, if present.
pub(super) fn find_by_label<'a>(
    update: &'a TreeUpdate,
    role: Role,
    label: &str,
) -> Option<(NodeId, &'a Node)> {
    update.nodes.iter().find_map(|(id, node)| {
        (node.role() == role && node.label() == Some(label)).then_some((*id, node))
    })
}

fn act(runtime: &mut HeadlessRuntime, action: Action, target: NodeId) -> bool {
    runtime.perform_accessibility_action(ActionRequest {
        action,
        target_node: target,
        target_tree: TreeId::ROOT,
        data: None,
    })
}

fn secondary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
    ]
}

/// A secondary click on a `.context_menu` view mounts the menu as a popup
/// window; the pump's returned update must merge that window's nodes into
/// the main tree — the same merge `SemanticRuntime` applies — or a test
/// host can never reach the menu by label.
#[test]
fn secondary_click_merges_the_context_menu_popup_into_the_tree() {
    let copied = Binding::container(false);
    let copied_for_view = copied.clone();
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let copied = copied_for_view.clone();
        AnyView::new(
            Frame::new(button("host").action(|| {}))
                .width(WINDOW_SIZE)
                .height(WINDOW_SIZE)
                .context_menu(vec!["Copy".action(move || copied.set(true))]),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_SIZE as u32,
        WINDOW_SIZE as u32,
        MinimalTestTheme::default(),
    );

    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the first frame must publish an accessibility tree");
    assert!(
        find_by_label(&update, Role::Button, "Copy").is_none(),
        "menu items must not emit before the menu opens"
    );

    // Secondary-click the centre of the host's emitted bounds — the same
    // spot the debug Inspect element item names through `node_at_point`.
    let (_, host) = find_by_label(&update, Role::Button, "host")
        .expect("the host button must emit an accessibility node");
    let bounds = host.bounds().expect("the host button has frame bounds");
    let (x, y) = (
        f64::midpoint(bounds.x0, bounds.x1),
        f64::midpoint(bounds.y0, bounds.y1),
    );
    for event in secondary_click(x as f32, y as f32) {
        runtime.push_input_event(event);
    }
    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the click frame must publish an accessibility tree");
    let (copy, copy_node) = find_by_label(&update, Role::Button, "Copy")
        .expect("the context menu's items must merge into the returned tree");
    assert!(copy_node.supports_action(Action::Click));

    // Debug builds append "Inspect element" under the application's own
    // items — it reaches the merged tree by the same path.
    if cfg!(debug_assertions) {
        assert!(
            find_by_label(&update, Role::Button, "Inspect element").is_some(),
            "the appended Inspect element item must merge into the returned tree"
        );
    }

    // The shifted ids are the merged tree's contract: an action aimed at one
    // demuxes back to the popup core that owns the item's target.
    assert!(act(&mut runtime, Action::Click, copy));
    assert!(
        copied.snapshot(),
        "clicking the merged popup item must fire its command"
    );

    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the activation frame must publish an accessibility tree");
    assert!(
        find_by_label(&update, Role::Button, "Copy").is_none(),
        "the popup's nodes must leave the merged tree once it closes"
    );
}

/// water-rs/hydrolysis#140: a popup runs in the environment of the view that
/// opened it, so `.state(&store)` on an ancestor reaches the item action's
/// extractors — here through the secondary-click path rather than menu
/// activation.
#[test]
fn context_menu_item_action_reads_state_inherited_from_the_opening_view() {
    #[waterui::prelude::state]
    #[derive(Clone)]
    struct Store {
        hits: Binding<u32>,
    }

    let store = Store {
        hits: Binding::container(0),
    };
    let store_for_view = store.clone();
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let store = store_for_view.clone();
        AnyView::new(
            Frame::new(button("host").action(|| {}))
                .width(WINDOW_SIZE)
                .height(WINDOW_SIZE)
                .context_menu(vec!["Bump".action(|store: Store| {
                    store.hits.set(store.hits.snapshot() + 1);
                })])
                .state(&store),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_SIZE as u32,
        WINDOW_SIZE as u32,
        MinimalTestTheme::default(),
    );

    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the first frame must publish an accessibility tree");
    let (_, host) = find_by_label(&update, Role::Button, "host")
        .expect("the host button must emit an accessibility node");
    let bounds = host.bounds().expect("the host button has frame bounds");
    let (x, y) = (
        f64::midpoint(bounds.x0, bounds.x1),
        f64::midpoint(bounds.y0, bounds.y1),
    );
    for event in secondary_click(x as f32, y as f32) {
        runtime.push_input_event(event);
    }
    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the click frame must publish an accessibility tree");
    let (bump, _) = find_by_label(&update, Role::Button, "Bump")
        .expect("the context menu's items must merge into the returned tree");
    assert!(act(&mut runtime, Action::Click, bump));
    assert_eq!(
        store.hits.snapshot(),
        1,
        "the item action did not reach the injected store"
    );
}
/// `is_settled` counts a window's pending frame — the main window's and each
/// popup's alike — so a test host cannot read a tree that predates a frame the
/// runtime already committed to. An accessibility action that changes its
/// window marks that window's frame pending: nothing is queued and no renderer
/// work is scheduled, yet the runtime is not settled until the pump that runs
/// the frame.
#[test]
fn a_window_with_a_pending_frame_is_not_settled() {
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(
                Frame::new(button("host").action(|| {}))
                    .width(WINDOW_SIZE)
                    .height(WINDOW_SIZE)
                    .context_menu(vec!["Copy".action(|| {})]),
            )
        }),
        WINDOW_SIZE as u32,
        WINDOW_SIZE as u32,
        MinimalTestTheme::default(),
    );

    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the first frame must publish an accessibility tree");
    assert!(
        runtime.is_settled(),
        "a runtime with no pending frame settles"
    );

    // The focus action lands on the main window's core: only the main window's
    // frame mode can account for the runtime being unsettled.
    let (host, _) =
        find_by_label(&update, Role::Button, "host").expect("the host button is missing");
    assert!(act(&mut runtime, Action::Focus, host));
    assert!(
        !runtime.is_settled(),
        "a main window with a pending frame must not report settled"
    );
    assert!(
        runtime.has_pending_semantic_update(),
        "a pending frame is a state change requested but not yet flushed"
    );
    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the pending frame must publish an accessibility tree");
    assert!(runtime.is_settled(), "the main window's pending frame ran");

    let (_, host) =
        find_by_label(&update, Role::Button, "host").expect("the host button must still emit");
    let bounds = host.bounds().expect("the host button has frame bounds");
    let (x, y) = (
        f64::midpoint(bounds.x0, bounds.x1),
        f64::midpoint(bounds.y0, bounds.y1),
    );
    for event in secondary_click(x as f32, y as f32) {
        runtime.push_input_event(event);
    }
    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the click frame must publish the merged tree");
    let (copy, _) = find_by_label(&update, Role::Button, "Copy")
        .expect("the context menu's items must merge into the returned tree");
    // The mount pump ran the popup's first scene pump; the click on its item
    // arms the popup's next frame — mounted, pending, not yet run — and only
    // the popup's own frame mode can account for the runtime being unsettled.
    assert!(act(&mut runtime, Action::Click, copy));
    assert!(
        !runtime.is_settled(),
        "a popup with a pending frame must not report settled"
    );
    assert!(
        runtime.has_pending_semantic_update(),
        "a pending popup frame is a state change requested but not yet flushed"
    );
    let _ = runtime.pump_at(false, Instant::now());
    assert!(
        runtime.is_settled(),
        "the popup's pending frame ran and the runtime settled"
    );
}

/// A popup that changes while the main window is clean must still publish:
/// the merged update describes every open window, not only the one that
/// emitted. A `Focus` inside a colour picker's swatch panel re-emits the
/// popup core without touching the main window — the pump's update carries
/// the popup's changed tree anyway.
#[test]
fn a_popup_only_change_publishes_the_merged_tree() {
    let tint = Binding::container(Color::srgb(0, 0, 0));
    let tint_for_view = tint;
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(vstack((ColorPicker::new("Tint", &tint_for_view),)))
        }),
        320,
        240,
        MinimalTestTheme::default(),
    );

    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the first frame must publish an accessibility tree");
    let (trigger, _) =
        find_by_label(&update, Role::Button, "Tint").expect("the color picker is missing");
    assert!(
        find_by_label(&update, Role::Button, "Red").is_none(),
        "swatches must not appear before the picker opens"
    );
    assert!(act(&mut runtime, Action::Click, trigger));

    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the picker's first frame must publish an accessibility tree");
    let (swatch, _) = find_by_label(&update, Role::Button, "Red")
        .expect("the swatch panel must merge into the returned tree");

    // Focusing the swatch re-emits only the popup core; the main window has
    // no pending update, yet the pump must still publish the merged tree.
    assert!(act(&mut runtime, Action::Focus, swatch));
    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("a popup-only change must still publish the merged tree");
    assert!(
        find_by_label(&update, Role::Button, "Red").is_some(),
        "the published update must carry the popup's nodes"
    );
    assert!(
        find_by_label(&update, Role::Button, "Tint").is_some(),
        "the published update must still carry the clean main window's nodes"
    );

    // The focused popup owns the merged tree's focus: the published focus
    // is the swatch's shifted id, not the main tree's.
    assert_eq!(
        update.focus, swatch,
        "the merged focus must follow the focused popup item"
    );

    // Closing the panel retires the popup; the merged focus returns to the
    // main tree's focused node — the trigger that opened it.
    assert!(act(&mut runtime, Action::Click, swatch));
    let update = runtime
        .pump_at(false, Instant::now())
        .tree_update
        .expect("the close frame must publish an accessibility tree");
    assert_eq!(
        update.focus, trigger,
        "the merged focus must return to the main tree once the popup closes"
    );
}

/// Pumps until the runtime settles and returns the merged tree as of that
/// settle — `None` when no window has ever produced one.
fn pump_until_settled(runtime: &mut HeadlessRuntime) -> Option<TreeUpdate> {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
    assert!(
        !runtime.has_pending_semantic_update(),
        "headless runtime never settled"
    );
    runtime.accessibility_tree()
}

/// `tree_update` is the "the tree changed" signal: a pump where no window —
/// main or popup — emitted must publish `None`, or a test host invalidates
/// its snapshot on every pump.
#[test]
fn a_clean_pump_publishes_no_tree_update() {
    let copied = Binding::container(false);
    let copied_for_view = copied;
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let copied = copied_for_view.clone();
        AnyView::new(
            Frame::new(button("host").action(|| {}))
                .width(WINDOW_SIZE)
                .height(WINDOW_SIZE)
                .context_menu(vec!["Copy".action(move || copied.set(true))]),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_SIZE as u32,
        WINDOW_SIZE as u32,
        MinimalTestTheme::default(),
    );

    let update = pump_until_settled(&mut runtime)
        .expect("the first frame must publish an accessibility tree");
    assert!(
        runtime.pump_at(false, Instant::now()).tree_update.is_none(),
        "a settled pump with no popup must publish nothing"
    );

    // With a popup open the same rule holds: the menu's window is mounted
    // and merged, but once every core is clean the next pump publishes
    // nothing.
    let (_, host) =
        find_by_label(&update, Role::Button, "host").expect("the host button is missing");
    let bounds = host.bounds().expect("the host button has frame bounds");
    let (x, y) = (
        f64::midpoint(bounds.x0, bounds.x1),
        f64::midpoint(bounds.y0, bounds.y1),
    );
    for event in secondary_click(x as f32, y as f32) {
        runtime.push_input_event(event);
    }
    let update =
        pump_until_settled(&mut runtime).expect("the click frame must publish the merged tree");
    assert!(
        find_by_label(&update, Role::Button, "Copy").is_some(),
        "the menu's window must be merged before the clean-pump check"
    );
    assert!(
        runtime.pump_at(false, Instant::now()).tree_update.is_none(),
        "a settled pump with an open popup must publish nothing"
    );
}

// ---------------------------------------------------------------------------
// water-rs/hydrolysis#191: press-and-hold opens `.context_menu` on touch/pen.
// The hold is armed by a primary `PointerDown` whose `PointerKind` is `Touch`
// or `Pen` on a region a secondary press would resolve, runs on the frame
// clock (`pump_at` drives `set_frame_instant` + `handle_gesture_tick`), and
// fires at [`HOLD`] so long as the press stays within the recognizer's slop.
// A held primary *mouse* button never arms.
// ---------------------------------------------------------------------------

/// A `button("host")` filling the window with `.context_menu` — the same
/// shape `secondary_click_...` mounts — with an activation counter so "the
/// press is consumed" (no tap fires on release) is assertable.
fn menu_host_view(activations: &Binding<u32>) -> AnyViewBuilder<AnyView> {
    let activations = activations.clone();
    AnyViewBuilder::<AnyView>::new(move || {
        let activations = activations.clone();
        AnyView::new(
            Frame::new(button("host").action(move || {
                activations.set(activations.snapshot() + 1);
            }))
            .width(WINDOW_SIZE)
            .height(WINDOW_SIZE)
            .context_menu(vec!["Copy".action(|| {})]),
        )
    })
}

const PRESS: (f32, f32) = (80.0, 80.0);

fn primary_press(kind: PointerKind) -> InputEvent {
    InputEvent::PointerDown {
        id: 3,
        kind,
        x: PRESS.0,
        y: PRESS.1,
        button: PointerButton::Primary,
    }
}

fn primary_release(kind: PointerKind, x: f32, y: f32) -> InputEvent {
    InputEvent::PointerUp {
        id: 3,
        kind,
        x,
        y,
        button: PointerButton::Primary,
    }
}

fn primary_move(kind: PointerKind, x: f32, y: f32) -> InputEvent {
    InputEvent::PointerMove { id: 3, kind, x, y }
}

fn menu_runtime(activations: &Binding<u32>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        menu_host_view(activations),
        WINDOW_SIZE as u32,
        WINDOW_SIZE as u32,
        MinimalTestTheme::default(),
    )
}

/// A touch press held past the threshold mounts the region's menu as a popup
/// anchored at the press point — the same mount a secondary click takes —
/// and consumes the press, so releasing it never fires the tap.
#[test]
fn a_touch_hold_past_the_threshold_mounts_the_menu_at_the_press_point() {
    let activations = Binding::container(0_u32);
    let mut runtime = menu_runtime(&activations);
    let start = Instant::now();

    let update = runtime
        .pump_at(false, start)
        .tree_update
        .expect("the first frame must publish an accessibility tree");
    assert!(
        find_by_label(&update, Role::Button, "Copy").is_none(),
        "menu items must not emit before the menu opens"
    );

    runtime.push_input_event(primary_press(PointerKind::Touch));
    let _ = runtime.pump_at(false, start);

    // Just before the threshold nothing opens.
    let update = runtime
        .pump_at(
            false,
            (start + HOLD)
                .checked_sub(Duration::from_millis(1))
                .unwrap(),
        )
        .tree_update;
    if let Some(update) = update {
        assert!(
            find_by_label(&update, Role::Button, "Copy").is_none(),
            "the hold must not open the menu before its threshold"
        );
    }

    // At the threshold the hold fires: the menu mounts through the drawn
    // presentation — the host fills the window and the menu is taller than
    // it, so it clamps to the window's top edge — and its items merge into
    // the tree.
    let update = runtime
        .pump_at(false, start + HOLD)
        .tree_update
        .expect("the hold frame must publish the merged tree");
    assert!(
        find_by_label(&update, Role::Button, "Copy").is_some(),
        "a held touch press must mount the context menu"
    );
    let (menu, _) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    assert!(
        menu.x0.abs() < 1.0 && menu.y0.abs() < 1.0,
        "the menu sits beside the lifted source, clamped inside the window, got {menu:?}"
    );

    // The press is consumed: releasing it fires no tap.
    runtime.push_input_event(primary_release(PointerKind::Touch, PRESS.0, PRESS.1));
    let _ = runtime.pump_at(false, start + HOLD);
    assert_eq!(
        activations.snapshot(),
        0,
        "the press the menu claimed must not commit a tap on release"
    );
}

/// Releasing a touch before the threshold fails the hold: the press resolves
/// as a tap and no menu opens — the deadline never fires for a hold that no
/// longer exists.
#[test]
fn a_touch_released_early_fires_the_tap_and_no_menu() {
    let activations = Binding::container(0_u32);
    let mut runtime = menu_runtime(&activations);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(primary_press(PointerKind::Touch));
    let _ = runtime.pump_at(false, start);
    runtime.push_input_event(primary_release(PointerKind::Touch, PRESS.0, PRESS.1));
    let _ = runtime.pump_at(false, start + Duration::from_millis(200));
    assert_eq!(
        activations.snapshot(),
        1,
        "an early release resolves as a tap"
    );

    // Past the threshold nothing mounts — the hold was disarmed by the up.
    let update = runtime.pump_at(false, start + HOLD).tree_update;
    if let Some(update) = update {
        assert!(
            find_by_label(&update, Role::Button, "Copy").is_none(),
            "no menu opens once the press released before the threshold"
        );
    }
    assert!(
        runtime.popup_frames().is_empty() && runtime.context_menu_presentation_frames().is_none(),
        "no menu mounts once the press released early"
    );
}

/// A press that leaves the recognizer's slop fails the hold — the same move
/// check `LongPressDetector` applies — so the threshold tick opens nothing.
#[test]
fn a_touch_that_moves_past_slop_opens_nothing() {
    let activations = Binding::container(0_u32);
    let mut runtime = menu_runtime(&activations);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(primary_press(PointerKind::Touch));
    let _ = runtime.pump_at(false, start);
    // `LONG_PRESS_SLOP` is 10 points; this move leaves it.
    runtime.push_input_event(primary_move(PointerKind::Touch, PRESS.0 + 16.0, PRESS.1));
    let _ = runtime.pump_at(false, start + Duration::from_millis(100));
    let _ = runtime.pump_at(false, start + HOLD);
    assert!(
        runtime.popup_frames().is_empty() && runtime.context_menu_presentation_frames().is_none(),
        "a press that leaves the slop never opens the menu"
    );

    // The move already cancelled the tap (touch presses can't drift), so the
    // release commits nothing either — but the point of the test is the menu.
    runtime.push_input_event(primary_release(PointerKind::Touch, PRESS.0 + 16.0, PRESS.1));
    let _ = runtime.pump_at(false, start + HOLD);
    assert_eq!(activations.snapshot(), 0);
}

/// A held primary *mouse* button never earns the gesture — the platforms
/// bind press-and-hold to touch and pen only.
#[test]
fn a_held_primary_mouse_button_opens_nothing() {
    let activations = Binding::container(0_u32);
    let mut runtime = menu_runtime(&activations);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(primary_press(PointerKind::Mouse));
    let _ = runtime.pump_at(false, start);
    let update = runtime.pump_at(false, start + HOLD).tree_update;
    if let Some(update) = update {
        assert!(
            find_by_label(&update, Role::Button, "Copy").is_none(),
            "a held primary mouse button must not open the menu"
        );
    }
    assert!(
        runtime.popup_frames().is_empty() && runtime.context_menu_presentation_frames().is_none(),
        "a held primary mouse button mounts no menu"
    );

    // The press behaves like an ordinary click: releasing it taps.
    runtime.push_input_event(primary_release(PointerKind::Mouse, PRESS.0, PRESS.1));
    let _ = runtime.pump_at(false, start + HOLD);
    assert_eq!(
        activations.snapshot(),
        1,
        "a released mouse click still taps"
    );
}

/// Pen input earns the same hold: the gesture resolves by pointer kind, not
/// by which channel delivered the press.
#[test]
fn a_pen_hold_behaves_like_a_touch_hold() {
    let activations = Binding::container(0_u32);
    let mut runtime = menu_runtime(&activations);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(primary_press(PointerKind::Pen));
    let _ = runtime.pump_at(false, start);
    let update = runtime
        .pump_at(false, start + HOLD)
        .tree_update
        .expect("the hold frame must publish the merged tree");
    assert!(
        find_by_label(&update, Role::Button, "Copy").is_some(),
        "a held pen press must mount the context menu at the press point"
    );
    assert!(
        runtime.context_menu_presentation_frames().is_some(),
        "the drawn menu mounts"
    );

    runtime.push_input_event(primary_release(PointerKind::Pen, PRESS.0, PRESS.1));
    let _ = runtime.pump_at(false, start + HOLD);
    assert_eq!(
        activations.snapshot(),
        0,
        "the press the menu claimed must not commit a tap on release"
    );
}
