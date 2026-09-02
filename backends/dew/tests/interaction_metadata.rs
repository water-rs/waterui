//! Interaction metadata on dew: `Metadata<OnEvent>` and
//! `Metadata<GestureObserver>`.
//!
//! These are the wrappers an interactive chart puts around its canvas, and
//! before dew handled them every one of them reached `Metadata::body` and
//! panicked with "not caught by your renderer" — a chart could not be rendered
//! on dew at all. Each test here therefore doubles as the regression: a frame
//! that renders is a frame that did not fast-fail.
//!
//! Input is driven the way a board drives it, by queueing
//! [`PointerSample`]s and pumping, because that is the only path dew has to
//! its renderer — there is no shortcut that skips the pointer plumbing being
//! tested. State is observed through `Binding`s the handlers write.

use nami::{Binding, binding};
use waterui::prelude::{Color, vstack};
use waterui_backend_core::input::TouchPhase;
use waterui_chart::{DataPoint, HitResult, LineChart};
use waterui_core::event::{Event, HoverEvent, OnEvent};
use waterui_core::gesture::{
    DragEvent, DragGesture, GestureObserver, GesturePhase, TapEvent, TapGesture,
};
use waterui_core::{AnyView, Environment, Metadata};
use waterui_dew::{DewRuntime, HostBoard, PointerSample, render_view_png};

mod support;

const WIDTH: u32 = 200;
const HEIGHT: u32 = 120;
/// A row inside the top half of the test stack — the half a chart occupies.
const HOVER_Y: f64 = 30.0;

fn runtime(build_root: impl Fn() -> AnyView + 'static) -> DewRuntime<HostBoard> {
    DewRuntime::new(
        HostBoard::new(WIDTH, HEIGHT),
        support::test_environment(),
        16,
        build_root,
    )
}

fn send(runtime: &mut DewRuntime<HostBoard>, x: f64, y: f64, phase: TouchPhase) {
    runtime
        .board_mut()
        .push_pointer(PointerSample { x, y, phase });
}

/// A hover handler must see the pointer enter, move within, and leave its own
/// rectangle — the three edges an interactive chart's focus tracking is built
/// from.
///
/// The hovered view is the top half of a two-colour stack, so "outside the
/// target" is a real position on the panel rather than an off-screen
/// coordinate no board would ever report.
#[test]
fn hover_metadata_reports_enter_move_and_exit() {
    let entered: Binding<i32> = binding(0_i32);
    let exited: Binding<i32> = binding(0_i32);
    let last_move: Binding<Option<(f32, f32)>> = Binding::container(None);

    let entered_for_view = entered.clone();
    let exited_for_view = exited.clone();
    let last_move_for_view = last_move.clone();

    let mut runtime = runtime(move || {
        let hot = Metadata::new(
            Metadata::new(
                Metadata::new(Color::red(), {
                    let entered = entered_for_view.clone();
                    OnEvent::new(Event::HoverEnter, move || *entered.get_mut() += 1)
                }),
                {
                    let last_move = last_move_for_view.clone();
                    OnEvent::new(Event::HoverMove, move |env: Environment| {
                        let hover = env
                            .get::<HoverEvent>()
                            .expect("dew must place HoverEvent in the hover environment");
                        last_move.set(Some((hover.location.x, hover.location.y)));
                    })
                },
            ),
            {
                let exited = exited_for_view.clone();
                OnEvent::new(Event::HoverExit, move || *exited.get_mut() += 1)
            },
        );
        AnyView::new(vstack((hot, Color::blue())))
    });

    runtime
        .pump()
        .expect("the initial frame must render the hover-wrapped tree");
    assert_eq!(entered.get(), 0, "rendering alone must not fire a hover");

    // Into the top half: enter and move both fire, and the move location is
    // local to the hovered view, whose origin is the window origin here.
    send(&mut runtime, 40.0, 20.0, TouchPhase::Moved);
    runtime
        .pump()
        .expect("a hover enter must refresh the frame it may have changed");
    assert_eq!(entered.get(), 1);
    assert_eq!(exited.get(), 0);
    assert_eq!(last_move.get(), Some((40.0, 20.0)));

    // Still inside: the move handler runs again, and because nothing in the
    // tree observes what it wrote, no frame is spent. That is the whole point
    // of not forcing a refresh from a hover move — see `interaction.rs`.
    send(&mut runtime, 60.0, 30.0, TouchPhase::Moved);
    assert!(
        runtime.pump().is_none(),
        "a hover move that changes nothing must not cost a frame"
    );
    assert_eq!(entered.get(), 1, "staying inside must not re-enter");
    assert_eq!(last_move.get(), Some((60.0, 30.0)));

    // Into the bottom half: outside the hovered view, so it exits.
    send(&mut runtime, 60.0, 100.0, TouchPhase::Moved);
    runtime
        .pump()
        .expect("a hover exit must refresh the frame it may have changed");
    assert_eq!(exited.get(), 1);
    assert_eq!(entered.get(), 1);
}

/// A cancelled pointer sequence is the pointer being gone rather than
/// elsewhere, so a hovered target must exit rather than wait for a move that
/// never arrives. This is what the simulator's `CursorLeft` produces.
#[test]
fn cancelled_pointer_exits_hovered_targets() {
    let exited: Binding<i32> = binding(0_i32);
    let exited_for_view = exited.clone();

    let mut runtime = runtime(move || {
        let exited = exited_for_view.clone();
        AnyView::new(Metadata::new(
            Metadata::new(
                Color::red(),
                OnEvent::new(Event::HoverExit, move || *exited.get_mut() += 1),
            ),
            OnEvent::new(Event::HoverEnter, || {}),
        ))
    });

    runtime.pump().expect("the initial frame must render");
    send(&mut runtime, 50.0, 50.0, TouchPhase::Moved);
    runtime.pump().expect("hover enter refreshes");
    assert_eq!(exited.get(), 0);

    send(&mut runtime, 50.0, 50.0, TouchPhase::Cancelled);
    runtime
        .pump()
        .expect("a cancelled pointer must refresh the frame its exit may have changed");
    assert_eq!(exited.get(), 1);
}

/// A tap observer must recognize a press/release pair and receive the
/// `TapEvent` payload, localized to the observed view.
#[test]
fn tap_gesture_metadata_recognizes_a_press_and_release() {
    let taps: Binding<Option<(f32, f32, u32)>> = Binding::container(None);
    let taps_for_view = taps.clone();

    let mut runtime = runtime(move || {
        let taps = taps_for_view.clone();
        AnyView::new(Metadata::new(
            Color::red(),
            GestureObserver::new(TapGesture::new(), move |env: Environment| {
                let tap = env
                    .get::<TapEvent>()
                    .expect("dew must place TapEvent in the gesture environment");
                taps.set(Some((tap.location.x, tap.location.y, tap.count)));
            }),
        ))
    });

    runtime
        .pump()
        .expect("the initial frame must render the gesture-wrapped tree");
    assert_eq!(taps.get(), None, "rendering alone must not fire a gesture");

    send(&mut runtime, 80.0, 40.0, TouchPhase::Started);
    send(&mut runtime, 80.0, 40.0, TouchPhase::Ended);
    runtime
        .pump()
        .expect("a recognized tap must refresh the frame its action may have changed");
    assert_eq!(taps.get(), Some((80.0, 40.0, 1)));
}

/// A drag observer must see the whole sequence, not just its end: an
/// interactive chart tracks the pointer through `Started`/`Updated` and
/// commits on `Ended`.
///
/// Tap and drag are stacked over the same view exactly as a chart stacks them,
/// which also proves dew delivers input to every observer over a point rather
/// than only the topmost one.
#[test]
fn drag_gesture_metadata_reports_every_phase() {
    let phases: Binding<Vec<GesturePhase>> = Binding::container(Vec::new());
    let taps: Binding<i32> = binding(0_i32);
    let phases_for_view = phases.clone();
    let taps_for_view = taps.clone();

    let mut runtime = runtime(move || {
        let phases = phases_for_view.clone();
        let taps = taps_for_view.clone();
        AnyView::new(Metadata::new(
            Metadata::new(
                Color::red(),
                GestureObserver::new(TapGesture::new(), move |_: Environment| {
                    *taps.get_mut() += 1;
                }),
            ),
            GestureObserver::new(DragGesture::new(0.0), move |env: Environment| {
                let drag = env
                    .get::<DragEvent>()
                    .expect("dew must place DragEvent in the gesture environment");
                phases.get_mut().push(drag.phase);
            }),
        ))
    });

    runtime.pump().expect("the initial frame must render");

    send(&mut runtime, 20.0, 20.0, TouchPhase::Started);
    send(&mut runtime, 60.0, 40.0, TouchPhase::Moved);
    send(&mut runtime, 90.0, 60.0, TouchPhase::Moved);
    send(&mut runtime, 90.0, 60.0, TouchPhase::Ended);
    runtime
        .pump()
        .expect("a recognized drag must refresh the frame its action may have changed");

    assert_eq!(
        phases.get(),
        vec![
            GesturePhase::Started,
            GesturePhase::Updated,
            GesturePhase::Ended
        ]
    );
    // The pointer travelled further than the tap tolerance, so the stacked tap
    // observer correctly failed rather than firing alongside the drag.
    assert_eq!(taps.get(), 0);
}

/// The pre-#180 behaviour, stated directly: a view carrying interaction
/// metadata used to abort dew's dispatch with `Metadata::panic_not_caught`.
/// Rendering it to a PNG now succeeds, and the content under the metadata is
/// what gets drawn.
#[test]
fn interaction_metadata_renders_instead_of_panicking() {
    let png = render_view_png(
        || {
            Metadata::new(
                Metadata::new(
                    Color::red(),
                    OnEvent::new(Event::HoverMove, |_: Environment| {}),
                ),
                GestureObserver::new(TapGesture::new(), || {}),
            )
        },
        support::test_environment(),
        64,
        64,
    );
    let pixmap = vello_cpu::Pixmap::from_png(std::io::Cursor::new(png.as_slice()))
        .expect("the rendered frame must decode as a PNG");
    let pixel = pixmap.data()[32 * 64 + 32];
    assert_eq!(
        [pixel.r, pixel.g, pixel.b, pixel.a],
        [244, 67, 54, 255],
        "the metadata must be transparent to what it wraps"
    );
}

/// Interaction metadata must stay transparent to layout: the wrapped view is
/// placed exactly where it would be without the wrapper, or a chart's hit
/// rectangle would not match the pixels it drew.
#[test]
fn interaction_metadata_is_transparent_to_layout() {
    let png = render_view_png(
        || {
            vstack((
                Metadata::new(Color::red(), GestureObserver::new(TapGesture::new(), || {})),
                Color::blue(),
            ))
        },
        support::test_environment(),
        64,
        64,
    );
    let pixmap = vello_cpu::Pixmap::from_png(std::io::Cursor::new(png.as_slice()))
        .expect("the rendered frame must decode as a PNG");
    let pixel = |x: usize, y: usize| {
        let pixel = pixmap.data()[y * 64 + x];
        [pixel.r, pixel.g, pixel.b, pixel.a]
    };
    assert_eq!(pixel(32, 10), [244, 67, 54, 255]);
    assert_eq!(pixel(32, 54), [33, 150, 243, 255]);
}

/// The component the issue is about, end to end on dew: an interactive
/// `LineChart` wraps its canvas in a tap observer, a drag observer, a hover
/// move handler, and a hover exit handler — four `Metadata` layers, every one
/// of which used to abort dew's dispatch before the first pixel was drawn.
///
/// Hovering sweeps across the plot rather than aiming at one datum: the
/// assertion is that dew delivers hover positions the chart's own hit-testing
/// can resolve, not a re-derivation of the chart's geometry inside a backend
/// test. Leaving the canvas must then clear the focus, which is the
/// `Event::HoverExit` half.
#[test]
fn line_chart_hover_and_tap_drive_selection_on_dew() {
    let focused: Binding<Option<HitResult<DataPoint>>> = Binding::container(None);
    let selected: Binding<Option<HitResult<DataPoint>>> = Binding::container(None);
    let focused_for_view = focused.clone();
    let selected_for_view = selected.clone();

    let mut runtime = runtime(move || {
        let data: Vec<DataPoint> = (0_u8..8)
            .map(|index| {
                let x = f32::from(index);
                DataPoint::new(x, (x * 0.7).sin().mul_add(6.0, 12.0))
            })
            .collect();
        AnyView::new(vstack((
            LineChart::new(Binding::container(data))
                .focused(&focused_for_view)
                .selected(&selected_for_view),
            Color::blue(),
        )))
    });

    runtime
        .pump()
        .expect("the initial frame must render an interactive chart");
    assert!(focused.get().is_none());

    // Sweep the top half — the chart's own share of the stack — until its hit
    // testing resolves a datum, and remember where that was so the tap below
    // aims at a point the chart agrees is there.
    let hovered = (1_u8..10).find_map(|step| {
        let x = f64::from(step) * f64::from(WIDTH) / 10.0;
        send(&mut runtime, x, HOVER_Y, TouchPhase::Moved);
        let _ = runtime.pump();
        focused.get().map(|hit| (x, hit))
    });
    let (hit_x, hovered) = hovered.expect("hovering the chart must focus one of its points");
    assert_eq!(hovered.series, 0);

    // Down into the bottom half: off the chart, so its hover-exit handler
    // clears the focus.
    send(&mut runtime, hit_x, 110.0, TouchPhase::Moved);
    let _ = runtime.pump();
    assert!(
        focused.get().is_none(),
        "leaving the chart must clear its focus"
    );

    // A tap at the position the chart just resolved a hover for commits that
    // point as the selection, through the chart's tap observer.
    send(&mut runtime, hit_x, HOVER_Y, TouchPhase::Started);
    send(&mut runtime, hit_x, HOVER_Y, TouchPhase::Ended);
    let _ = runtime.pump();
    let selected_hit = selected
        .get()
        .expect("tapping the chart must select one of its points");
    assert_eq!(selected_hit.series, hovered.series);
    assert_eq!(selected_hit.index, hovered.index);
}
