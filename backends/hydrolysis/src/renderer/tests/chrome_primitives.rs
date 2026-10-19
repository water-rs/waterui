//! Engine-provided chrome primitives (water-rs/waterui#1433): a button whose
//! theme hands out a [`Chrome`] records it once, at mount, and a hover-in,
//! press and release cycle afterwards moves only the recording's live
//! operands — no chrome recording, no theme draw call.

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use cherenkov_record::{Paint, Shadow, WorkingColor};
use waterui::{AnyView, View};
use waterui_controls::button::button;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::{InteractionState, StateValue};

use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::FrameWorkCounters;
use crate::renderer::chrome::LiveChrome;
use crate::runner::FrameCounters;
use crate::theme::{
    Chrome, CornerShape, Corners, Elevation, FocusIndicator, PressFeedback, Ripple, StateLayer,
    Surface,
};

const WINDOW: u32 = 320;
/// The window's centre, where the button lies.
const CENTER: f32 = 160.0;
const POINTER: u64 = 1;

fn color(r: f32, g: f32, b: f32) -> WorkingColor {
    WorkingColor::new([r, g, b, 1.0])
}

/// Every primitive at once: a pill that squares off while pressed, a fill
/// and an elevation that change on hover, a border, an M3-style ripple and
/// a focus ring.
fn chrome() -> Chrome {
    let raised = Shadow {
        sigma: 2.0,
        offset: kurbo::Vec2::new(0.0, 1.0),
        spread: 0.0,
        color: WorkingColor::new([0.0, 0.0, 0.0, 0.3]),
    };
    Chrome::new(
        Surface::new(
            CornerShape::full().pressed(Corners::from(8.0)),
            StateValue::new(Paint::from(color(0.4, 0.3, 0.8)))
                .when(InteractionState::HOVERED, Paint::from(color(0.5, 0.4, 0.9))),
        )
        .border(crate::theme::Border::new(1.0, color(0.2, 0.2, 0.2)))
        .elevation(
            StateValue::new(Elevation::NONE)
                .when(InteractionState::HOVERED, Elevation::new(raised, raised)),
        ),
        StateLayer::new(color(1.0, 1.0, 1.0)).opacities(0.08, 0.1, 0.1, 0.16),
        PressFeedback::Ripple(Ripple {
            min_diameter: 48.0,
            initial_scale: 0.4,
            drift: 1.0,
        }),
        FocusIndicator::Ring {
            width: 3.0,
            offset: 2.0,
            color: color(0.1, 0.1, 0.9),
        },
    )
}

fn runtime_with(view: impl View, theme: MinimalTestTheme) -> HeadlessRuntime {
    let view = RefCell::new(Some(AnyView::new(view)));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the fixture view is built once")
    });
    HeadlessRuntime::new_for_tests(pumped_test_environment(), builder, WINDOW, WINDOW, theme)
}

/// Pumps rendering frames on a fixed 16 ms cadence.
struct Frames {
    start: Instant,
    next: u64,
}

impl Frames {
    fn pump(&mut self, runtime: &mut HeadlessRuntime) -> FrameCounters {
        let at = self.start + Duration::from_millis(self.next * 16);
        self.next += 1;
        runtime.pump_at(true, at).profile.counters
    }
}

/// The one bound button's live chrome.
fn the_chrome(runtime: &HeadlessRuntime) -> Rc<LiveChrome> {
    let chromes = runtime.renderer().hit_test.interaction.live_chromes();
    let [chrome] = <[_; 1]>::try_from(chromes).expect("exactly one button binds a live chrome");
    chrome
}

/// The tree work done between two frames' counters. The renderer resets
/// the counters only when it builds the scene, so between steady frames
/// they accumulate; a reset in between means the interaction rebuilt it.
fn tree_work(before: FrameWorkCounters, after: FrameWorkCounters) -> [u64; 3] {
    let delta = |before: u64, after: u64| {
        after
            .checked_sub(before)
            .expect("an interaction rebuilt the scene and reset the frame-work counters")
    };
    [
        delta(before.live_recordings, after.live_recordings),
        delta(before.recorded_view_contents, after.recorded_view_contents),
        delta(before.layout_calls, after.layout_calls),
    ]
}

#[test]
fn interaction_cycle_records_chrome_once() {
    let theme = MinimalTestTheme::default().with_button_chrome(chrome());
    let draws = theme.button_chrome_draws();
    let mut runtime = runtime_with(button("Chrome").action(|| {}), theme);
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    let mut mounted = frames.pump(&mut runtime).frame_work;
    for _ in 0..3 {
        mounted = frames.pump(&mut runtime).frame_work;
    }
    let chrome = the_chrome(&runtime);
    assert_eq!(
        chrome.recordings(),
        2,
        "mount records the chrome once: its surface below the label and its layers above"
    );
    assert_eq!(chrome.state(), InteractionState::empty());

    runtime.push_input_event(InputEvent::PointerMove {
        id: POINTER,
        kind: PointerKind::Mouse,
        x: CENTER,
        y: CENTER,
    });
    let mut hovered = mounted;
    for _ in 0..16 {
        hovered = frames.pump(&mut runtime).frame_work;
    }
    assert_eq!(chrome.state(), InteractionState::HOVERED);
    assert_eq!(
        tree_work(mounted, hovered),
        [0, 0, 0],
        "hover records nothing and walks no Hydrolysis tree"
    );

    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER,
        kind: PointerKind::Mouse,
        x: CENTER,
        y: CENTER,
        button: PointerButton::Primary,
    });
    frames.pump(&mut runtime);
    assert_eq!(
        chrome.state(),
        InteractionState::HOVERED | InteractionState::PRESSED
    );
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER,
        kind: PointerKind::Mouse,
        x: CENTER,
        y: CENTER,
        button: PointerButton::Primary,
    });
    // Through the minimum press duration and the ripple's fade-out.
    for _ in 0..48 {
        frames.pump(&mut runtime);
    }
    assert_eq!(chrome.state(), InteractionState::HOVERED);
    assert!(
        Rc::ptr_eq(&chrome, &the_chrome(&runtime)),
        "the cycle keeps the mounted chrome"
    );
    assert_eq!(chrome.recordings(), 2, "press and release record no chrome");
    assert_eq!(draws.get(), 0, "the theme never draws the button's chrome");
}

/// water-rs/waterui#2940: a pointer press that moves keyboard focus onto a
/// live-chrome control is pure bookkeeping — the focus indicator rides the
/// chrome's live operands and the accessibility focus publishes as a
/// focus-only tree update. The press frame therefore does no tree work:
/// no layout pass, no recorded contents, no semantic builds — and the
/// update it publishes carries only the moved focus, not a re-emitted
/// tree.
#[test]
#[cfg(feature = "accessibility")]
fn press_focus_move_needs_no_tree_pass() {
    // The press feedback is a static tint, not a ripple: the press needs
    // no animation frames, so every mark this test may produce is the
    // focus move's — anything beyond the operands is the bug.
    let mut focus_chrome = chrome();
    focus_chrome.press = PressFeedback::Tint;
    let theme = MinimalTestTheme::default().with_button_chrome(focus_chrome);
    let mut runtime = runtime_with(button("Chrome").action(|| {}), theme);
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    // Mount plus a few steady frames settle the scene before input lands;
    // the last pump's counters are the baseline the press pump's delta is
    // measured against (the renderer resets them only on a scene build, so
    // a rebuild inside the press pump reads as a reset — itself a failure).
    let mut settled = frames.pump(&mut runtime).frame_work;
    for _ in 0..3 {
        settled = frames.pump(&mut runtime).frame_work;
    }
    let button_node = runtime
        .accessibility_tree()
        .and_then(|tree| {
            tree.nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Chrome"))
                .map(|(id, _)| *id)
        })
        .expect("the button emits an accessibility node");

    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER,
        kind: PointerKind::Mouse,
        x: CENTER,
        y: CENTER,
        button: PointerButton::Primary,
    });
    let at = frames.start + Duration::from_millis(frames.next * 16);
    let result = runtime.pump_at(true, at);
    let work = result.profile.counters.frame_work;

    assert!(
        the_chrome(&runtime)
            .state()
            .contains(InteractionState::PRESSED),
        "the press landed on the live chrome"
    );
    assert_eq!(
        tree_work(settled, work),
        [0, 0, 0],
        "a focus move records nothing and lays nothing out"
    );
    assert_eq!(
        work.semantic_builds - settled.semantic_builds,
        0,
        "a focus move builds nothing"
    );
    let update = result
        .tree_update
        .expect("the focus move publishes an accessibility update");
    assert_eq!(
        update.focus, button_node,
        "the published focus lands on the pressed button's node"
    );
    assert!(
        update.nodes.is_empty(),
        "a focus move emits no node changes — {} nodes emitted",
        update.nodes.len()
    );
}
