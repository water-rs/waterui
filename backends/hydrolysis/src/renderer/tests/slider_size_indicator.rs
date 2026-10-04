//! Slider `ControlSize` flows from the config to the theme's
//! `slider_metrics(size)`, and the value indicator — the theme-drawn chrome
//! plus the formatter's text above the thumb — is emitted only while the
//! pointer holds the drag.

use std::cell::RefCell;
use std::rc::Rc;

use kurbo::Rect;
use waterui::{Binding, Str};
use waterui_controls::ControlSize;
use waterui_controls::slider::slider;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;

use super::{MinimalTestTheme, capture_root_window, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const POINTER_ID: u64 = 7;
const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;

fn runtime_with(view: AnyView, theme: MinimalTestTheme) -> HeadlessRuntime {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        theme,
    )
}

fn pointer_down(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump(false);
}

fn pointer_move(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerMove {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
    });
    let _ = runtime.pump(false);
}

fn pointer_up(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump(false);
}

/// The test theme records the `ControlSize` every `slider_metrics` call is
/// asked for and answers a track height that grows with the size, so a drawn
/// track rect reveals which metrics the renderer resolved.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "the control size is the exact theme metric"
)]
fn slider_size_reaches_theme_metrics() {
    let env = test_environment();
    let sizes = Rc::new(RefCell::new(Vec::new()));
    let tracks = Rc::new(RefCell::new(Vec::new()));
    let bounds = Rect::new(0.0, 0.0, 400.0, 60.0);
    let value = Binding::container(0.5);

    for size in [
        ControlSize::ExtraSmall,
        ControlSize::Small,
        ControlSize::Medium,
        ControlSize::Large,
        ControlSize::ExtraLarge,
    ] {
        sizes.borrow_mut().clear();
        tracks.borrow_mut().clear();
        let mut renderer = super::test_renderer_with_theme(MinimalTestTheme {
            slider_metric_sizes: Rc::clone(&sizes),
            slider_track_draws: Rc::clone(&tracks),
            ..Default::default()
        });
        capture_root_window(
            &mut renderer,
            slider("Volume", &value).hide_label().size(size),
            &env,
            bounds,
        );
        assert!(
            sizes.borrow().iter().all(|asked| *asked == size),
            "every slider_metrics call must be answered for {size:?}, got {:?}",
            sizes.borrow()
        );
        assert!(
            !sizes.borrow().is_empty(),
            "the renderer must consult slider_metrics"
        );
        let track = tracks.borrow()[0];
        let expected_height = 4.0f64.mul_add(f64::from(size as u8), 6.0);
        assert_eq!(
            track.height(),
            expected_height,
            "{size:?} must draw the theme's {expected_height}pt track, got {track:?}"
        );
    }
}

/// At rest there is no indicator: the theme's chrome hook is never called and
/// the formatter is never invoked. While the pointer holds the thumb the
/// chrome is drawn above it and the formatter produces the label text; the
/// formatter tracks the dragged value, and releasing the pointer retires the
/// indicator.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "the indicator geometry is exact theme arithmetic"
)]
fn value_indicator_only_while_dragging() {
    let indicator_draws = Rc::new(RefCell::new(Vec::new()));
    let formatted = Rc::new(RefCell::new(Vec::<f64>::new()));
    let value = Binding::container(25.0);

    let record = Rc::clone(&formatted);
    let view = AnyView::new(
        slider("Volume", &value)
            .hide_label()
            .range(0.0..=100.0)
            .value_indicator(move |v| {
                record.borrow_mut().push(v);
                Str::from(format!("{v:.0}"))
            }),
    );
    let theme = MinimalTestTheme {
        slider_value_indicator_draws: Rc::clone(&indicator_draws),
        ..Default::default()
    };
    let mut runtime = runtime_with(view, theme);
    let _ = runtime.pump(false);

    assert!(
        indicator_draws.borrow().is_empty(),
        "at rest no indicator chrome may be drawn, got {:?}",
        indicator_draws.borrow()
    );
    assert!(
        formatted.borrow().is_empty(),
        "at rest the formatter must not run, got {:?}",
        formatted.borrow()
    );

    // Track: x in [12, 788] for range 0..=100, hidden label, so the value-25
    // thumb sits at x = 206. The slider reserves a 20pt label row once its
    // bounds reach 36pt, which centres the track at y = 310 in this window.
    pointer_down(&mut runtime, 206.0, 310.0);
    let draws = indicator_draws.borrow().clone();
    assert_eq!(
        draws.len(),
        1,
        "pressing the thumb draws one indicator, got {draws:?}"
    );
    let bubble = draws[0];
    assert_eq!(
        f64::midpoint(bubble.x0, bubble.x1),
        206.0,
        "the indicator is centred on the thumb, got {bubble:?}"
    );
    assert!(
        bubble.y1 <= 310.0 - 20.0,
        "the indicator floats above the thumb, got {bubble:?}"
    );
    assert_eq!(
        formatted.borrow().as_slice(),
        &[25.0],
        "the indicator formats the pressed value"
    );

    indicator_draws.borrow_mut().clear();
    pointer_move(&mut runtime, 500.0, 300.0);
    assert_eq!(
        indicator_draws.borrow().len(),
        1,
        "a dragged thumb keeps the indicator up"
    );
    let dragged = *formatted.borrow().last().expect("formatter ran");
    assert!(
        (dragged - 62.886_597_938_144_33).abs() < 0.01,
        "the indicator tracks the dragged value, got {dragged}"
    );
    let bubble = indicator_draws.borrow()[0];
    assert_eq!(
        f64::midpoint(bubble.x0, bubble.x1),
        500.0,
        "the indicator follows the thumb, got {bubble:?}"
    );

    indicator_draws.borrow_mut().clear();
    pointer_up(&mut runtime, 500.0, 300.0);
    assert!(
        indicator_draws.borrow().is_empty(),
        "after release the indicator retires, got {:?}",
        indicator_draws.borrow()
    );
}
