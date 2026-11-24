//! Headless interactive dynamics tests (water-rs/waterui#4928):
//! 1. Capped half-axis press growth (`PressFeedback::max_growth`).
//! 2. Smoothed drag velocity on `Inputs::velocity` and elastic `DragDeformation`.
//! 3. Windowed smoothstep transfer curves (`Driven::smoothstep_window`) running
//!    on Cherenkov's clock with zero per-frame Hydrolysis recordings.
//! 4. Bar-level gliding tab selection indicator (`TabsLook::selection_indicator`,
//!    `TabsLook::selection_indicator_over_labels`, `TabsLook::drag`).

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use approx::assert_relative_eq;
use cherenkov_record::{Draw as _, Paint, ShapeData, WorkingColor};
use kurbo::{BezPath, Rect, RoundedRect, Shape as _, Vec2};
use nami::{Binding, Signal};
use waterui::animation::Animation;
use waterui::component::text;
use waterui::{Color, ViewExt as _};
use waterui_controls::button::button;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_interaction::interaction::InteractionState;
use waterui_navigation::NavigationView;
use waterui_navigation::tab::{Tab, Tabs};

use super::chrome_primitives::{Frames, runtime_with};
use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::chrome::LiveChrome;
use crate::renderer::live_tabs::LiveTabs;
use crate::theme::{
    Chrome, CornerShape, Custom, DragDeformation, Driven, FocusIndicator, Inputs,
    Label as LookLabel, Motion, PressFeedback, StateLayer, Surface, TabsDrag, TabsLook,
    TabsMetrics, TabsValue,
};

const CENTER: f32 = 160.0;
const POINTER: u64 = 7;

fn solid(r: f32, g: f32, b: f32) -> Paint {
    Paint::from(WorkingColor::new([r, g, b, 1.0]))
}

fn the_chrome(runtime: &HeadlessRuntime) -> Rc<LiveChrome> {
    let chromes = runtime.renderer().hit_test.interaction.live_chromes();
    let [chrome] = <[_; 1]>::try_from(chromes).expect("one button binds one live chrome");
    chrome
}

fn the_tabs(runtime: &HeadlessRuntime) -> Rc<LiveTabs> {
    let mut tabs = runtime.renderer().hit_test.interaction.live_tabs();
    tabs.dedup_by(|a, b| Rc::ptr_eq(a, b));
    let [tabs] = <[_; 1]>::try_from(tabs).expect("one tab bar binds one live tab set");
    tabs
}

fn shape_bbox(shape: &ShapeData) -> Rect {
    match shape {
        ShapeData::Rect(r) => *r,
        ShapeData::RoundedRect(rr) => rr.rect(),
        ShapeData::Continuous(cr) => cr.rect,
        ShapeData::Circle(c) => c.bounding_box(),
        ShapeData::Ellipse(e) => e.bounding_box(),
        ShapeData::Line(l) => l.bounding_box(),
        ShapeData::Path { elements, .. } => BezPath::from_vec(elements.to_vec()).bounding_box(),
    }
}

fn pointer_down(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
}

fn pointer_move(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerMove {
        id: POINTER,
        kind: PointerKind::Mouse,
        x,
        y,
    });
}

fn pointer_up(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    runtime.push_input_event(InputEvent::PointerUp {
        id: POINTER,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
}

#[test]
fn press_feedback_max_growth_caps_wide_button_silhouette_and_content_scale() {
    let spec = Chrome::new(
        Surface::new(CornerShape::radius(8.0), solid(0.2, 0.4, 0.8)),
        StateLayer::new(Color::srgb_f32(1.0, 1.0, 1.0)),
        PressFeedback::new().scale(0.25).max_growth(10.0),
        FocusIndicator::Tint,
        Motion::uniform(Animation::linear(Duration::ZERO)),
    );
    let theme = MinimalTestTheme::default()
        .with_button_chrome(spec)
        .with_button_label(
            LookLabel::new()
                .color(Color::srgb_f32(1.0, 1.0, 1.0))
                .follows_press_scale(),
        );

    let mut runtime = runtime_with(
        button(text("Wide button with enough text to cap growth")).action(|| {}),
        theme,
    );
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    let _ = frames.pump(&mut runtime);
    let chrome = the_chrome(&runtime);
    let rest_rect = shape_bbox(&chrome.shape());
    let recordings_before = chrome.recordings();
    assert!(
        0.5 * rest_rect.width() * 0.25 > 10.0,
        "uncapped press growth should exceed 10 pt on this wide button, got width {}",
        rest_rect.width()
    );

    pointer_down(&mut runtime, CENTER, CENTER);
    let _ = frames.pump(&mut runtime);
    let pressed_rect = shape_bbox(&chrome.shape());
    let expected_scale = 1.0 + 10.0 / (0.5 * rest_rect.width());
    assert_relative_eq!(
        pressed_rect.width(),
        rest_rect.width() * expected_scale,
        epsilon = 1e-3
    );
    assert_relative_eq!(
        pressed_rect.height(),
        rest_rect.height() * expected_scale,
        epsilon = 1e-3
    );
    // Half-axis growth is capped at 10.0 pt and aspect ratio is preserved.
    assert_relative_eq!(
        0.5 * (pressed_rect.width() - rest_rect.width()),
        10.0,
        epsilon = 1e-3
    );
    let [sx, _, _, sy, _, _] = chrome.content_scale_operand().snapshot().as_coeffs();
    assert_relative_eq!(sx, expected_scale, epsilon = 1e-6);
    assert_relative_eq!(sy, expected_scale, epsilon = 1e-6);
    assert_eq!(
        chrome.recordings(),
        recordings_before,
        "pressing must not re-record the chrome"
    );
}

#[test]
fn live_chrome_exposes_smoothed_drag_velocity_and_applies_area_preserving_drag_deformation() {
    let observed_velocities: Rc<RefCell<Vec<Option<Vec2>>>> = Rc::new(RefCell::new(Vec::new()));
    let observer = Rc::clone(&observed_velocities);

    let deform = DragDeformation::new()
        .lag(6.0, 40.0)
        .displacement_stretch(0.10, 60.0)
        .velocity_stretch(0.12, 800.0)
        .stretch_bounds(0.85, 1.25);

    let spec = Chrome::new(
        Surface::new(CornerShape::radius(8.0), solid(0.2, 0.5, 0.7)),
        StateLayer::new(Color::srgb_f32(1.0, 1.0, 1.0)),
        PressFeedback::new().drag_deformation(deform),
        FocusIndicator::Tint,
        Motion::uniform(Animation::linear(Duration::ZERO)),
    )
    .custom(Custom::new(
        (Driven::new(move |inputs| {
            observer.borrow_mut().push(inputs.velocity);
            0.0_f32
        }),),
        |_recorder, _frame, (_live,)| {},
    ));

    let theme = MinimalTestTheme::default().with_button_chrome(spec);
    let mut runtime = runtime_with(
        button(text("Drag")).action(|| {}).width(120.0).height(44.0),
        theme,
    );
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    let _ = frames.pump(&mut runtime);
    let chrome = the_chrome(&runtime);
    let rest_rect = shape_bbox(&chrome.shape());
    let recordings_before = chrome.recordings();

    // Press at centre, then drag right at 8 pt per 16 ms frame (500 pt/s).
    pointer_down(&mut runtime, CENTER, CENTER);
    let _ = frames.pump(&mut runtime);
    for step in 1_u16..=6 {
        pointer_move(&mut runtime, f32::from(step).mul_add(8.0, CENTER), CENTER);
        let _ = frames.pump(&mut runtime);
    }

    let last_vel = observed_velocities
        .borrow()
        .iter()
        .rev()
        .find_map(|v| *v)
        .expect("drag moves must populate smoothed velocity on Inputs");
    assert!(
        last_vel.x > 300.0 && last_vel.x <= 505.0,
        "smoothed horizontal velocity should converge toward 500 pt/s, got {last_vel:?}"
    );
    assert_relative_eq!(last_vel.y, 0.0, epsilon = 1e-3);

    let deformed_rect = shape_bbox(&chrome.shape());
    // Rubber-band lag shifts centre right by (0, 6] pt.
    let lag_x = deformed_rect.center().x - rest_rect.center().x;
    assert!(
        lag_x > 1.0 && lag_x <= 6.0,
        "rubber-band lag should stay in (1, 6] pt, got {lag_x}"
    );
    // Horizontal drag stretches width and compresses height while preserving area.
    assert!(deformed_rect.width() > rest_rect.width());
    assert!(deformed_rect.height() < rest_rect.height());
    assert_relative_eq!(deformed_rect.area(), rest_rect.area(), epsilon = 1e-2);

    // Content transform carries unit-determinant stretch + lag.
    let content_affine = chrome.content_scale_operand().snapshot();
    assert_relative_eq!(content_affine.determinant(), 1.0, epsilon = 1e-6);

    // Releasing clears drag/velocity and returns the silhouette to rest with zero re-recording.
    pointer_up(&mut runtime, CENTER + 48.0, CENTER);
    let _ = frames.pump(&mut runtime);
    let released_rect = shape_bbox(&chrome.shape());
    assert_relative_eq!(released_rect.x0, rest_rect.x0, epsilon = 1e-3);
    assert_relative_eq!(released_rect.width(), rest_rect.width(), epsilon = 1e-3);
    assert_relative_eq!(released_rect.height(), rest_rect.height(), epsilon = 1e-3);
    assert_eq!(
        observed_velocities.borrow().last().copied(),
        Some(None),
        "velocity must reset to None after release"
    );
    assert_eq!(chrome.recordings(), recordings_before);
}

#[test]
fn driven_smoothstep_window_runs_on_engine_clock_without_hydrolysis_rerecording() {
    let part_opacity = Driven::new(|inputs| {
        if inputs.state.contains(InteractionState::PRESSED) {
            1.0_f32
        } else {
            0.0_f32
        }
    })
    .smoothstep_window(0.05, 0.55);

    let spec = Chrome::new(
        Surface::new(CornerShape::radius(8.0), solid(0.3, 0.3, 0.6)),
        StateLayer::new(Color::srgb_f32(1.0, 1.0, 1.0)),
        PressFeedback::new(),
        FocusIndicator::Tint,
        Motion::uniform(Animation::linear(Duration::from_millis(200))),
    )
    .custom(
        Custom::new((), |recorder, frame, ()| {
            recorder.fill(frame, solid(1.0, 1.0, 1.0));
        })
        .opacity(part_opacity),
    );

    let theme = MinimalTestTheme::default().with_button_chrome(spec);
    let mut runtime = runtime_with(
        button(text("Morph"))
            .action(|| {})
            .width(120.0)
            .height(44.0),
        theme,
    );
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    let _ = frames.pump(&mut runtime);
    let chrome = the_chrome(&runtime);
    let recordings_before = chrome.recordings();

    pointer_down(&mut runtime, CENTER, CENTER);
    // Run across the full 200 ms press window (15 frames * 16 ms = 240 ms) and
    // verify zero Hydrolysis tree/view re-recordings occur while the track runs.
    let work = frames.work(&mut runtime, 15);
    assert_eq!(
        work,
        [0, 0, 0],
        "smoothstep-windowed press transition must do zero Hydrolysis work per frame"
    );

    pointer_up(&mut runtime, CENTER, CENTER);
    let release_work = frames.work(&mut runtime, 15);
    assert_eq!(
        release_work,
        [0, 0, 0],
        "mirrored smoothstep-windowed release transition must do zero Hydrolysis work per frame"
    );
    assert_eq!(chrome.recordings(), recordings_before);
}

struct GlidingTabProbes {
    positions: Rc<RefCell<Vec<Option<f64>>>>,
    velocities: Rc<RefCell<Vec<Option<Vec2>>>>,
    bounds: Rc<RefCell<Vec<Option<Rect>>>>,
    over_label_draws: Rc<RefCell<u32>>,
}

fn gliding_tabs_look() -> (TabsLook, GlidingTabProbes) {
    let positions: Rc<RefCell<Vec<Option<f64>>>> = Rc::new(RefCell::new(Vec::new()));
    let velocities: Rc<RefCell<Vec<Option<Vec2>>>> = Rc::new(RefCell::new(Vec::new()));
    let bounds: Rc<RefCell<Vec<Option<Rect>>>> = Rc::new(RefCell::new(Vec::new()));
    let over_label_draws: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

    let pos_sink = Rc::clone(&positions);
    let vel_sink = Rc::clone(&velocities);
    let bounds_sink = Rc::clone(&bounds);
    let over_sink = Rc::clone(&over_label_draws);

    let indicator = Custom::new(
        (Driven::new(move |inputs: &Inputs<'_, TabsValue>| {
            pos_sink.borrow_mut().push(inputs.value.position);
            vel_sink.borrow_mut().push(inputs.velocity);
            let rect = inputs.value.indicator_bounds();
            bounds_sink.borrow_mut().push(rect);
            let rect = rect.unwrap_or(Rect::ZERO);
            ShapeData::RoundedRect(RoundedRect::from_rect(rect, 12.0))
        }),),
        |recorder, _frame, (shape,)| {
            recorder.fill(shape, solid(0.2, 0.5, 0.9));
        },
    );

    let over_indicator = Custom::new(
        (Driven::new(|inputs: &Inputs<'_, TabsValue>| {
            let rect = inputs.value.indicator_bounds().unwrap_or(Rect::ZERO);
            ShapeData::RoundedRect(RoundedRect::from_rect(rect, 12.0))
        }),),
        move |recorder, _frame, (shape,)| {
            *over_sink.borrow_mut() += 1;
            recorder.fill(shape, Paint::from(WorkingColor::new([1.0, 1.0, 1.0, 0.1])));
        },
    );

    let look = TabsLook::new(
        TabsMetrics::new(56.0, 64.0, 8.0, 4.0),
        Custom::new((), |recorder, frame, ()| {
            recorder.fill(frame, solid(0.95, 0.95, 0.97));
        }),
        Custom::new((), |recorder, frame, ()| {
            recorder.fill(frame, solid(0.85, 0.85, 0.88));
        }),
        Motion::uniform(Animation::linear(Duration::from_millis(120))),
        |_item, _selected| {
            Chrome::new(
                Surface::clear(CornerShape::radius(8.0)),
                StateLayer::new(Color::srgb_f32(0.0, 0.0, 0.0)),
                PressFeedback::new(),
                FocusIndicator::Tint,
                Motion::uniform(Animation::linear(Duration::from_millis(120))),
            )
        },
    )
    .selection_indicator(indicator)
    .selection_indicator_over_labels(over_indicator)
    .drag(TabsDrag::new());

    (
        look,
        GlidingTabProbes {
            positions,
            velocities,
            bounds,
            over_label_draws,
        },
    )
}

#[test]
fn tabs_look_bar_level_selection_indicator_glides_on_tap_and_drag_without_rerecording() {
    let (look, probes) = gliding_tabs_look();

    let selection = Binding::container(0_usize);
    let tabs: Vec<_> = (1..=3)
        .map(|i| {
            Tab::new(format!("Tab {i}"), move || {
                NavigationView::new(format!("Tab {i}"), text(format!("Page {i}")))
            })
        })
        .collect();
    let tabs_view = Tabs::new(&selection, tabs);

    let view = RefCell::new(Some(AnyView::new(tabs_view)));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the tabs fixture view is built once")
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        360,
        240,
        MinimalTestTheme::default().with_tabs_look(look),
    );
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    let _ = frames.pump(&mut runtime);
    let live = the_tabs(&runtime);
    let recordings_before = live.recordings();
    assert_eq!(live.selected(), Some(0));
    assert_eq!(*probes.over_label_draws.borrow(), 1);
    let first_bounds = probes
        .bounds
        .borrow()
        .last()
        .copied()
        .flatten()
        .expect("initial tab 0 bounds recorded");

    // Tap tab 2 (3 equal tabs across 360 pt -> tab 2 centre at x = 300, bottom bar y = 212).
    pointer_down(&mut runtime, 300.0, 212.0);
    let _ = frames.pump(&mut runtime);
    pointer_up(&mut runtime, 300.0, 212.0);
    let _ = frames.pump(&mut runtime);
    assert_eq!(selection.snapshot(), 2);
    assert_eq!(live.selected(), Some(2));
    let third_bounds = probes
        .bounds
        .borrow()
        .last()
        .copied()
        .flatten()
        .expect("tab 2 bounds recorded");
    assert!(
        third_bounds.x0 > first_bounds.x1,
        "bar-level indicator moved to tab 2 bounds ({third_bounds:?} vs {first_bounds:?})"
    );
    assert_eq!(
        live.recordings(),
        recordings_before,
        "switching tabs must glide the indicator without re-recording the bar"
    );

    // Drag from selected tab 2 (x = 300) left toward tab 1 (x = 180) across 16 ms steps.
    probes.positions.borrow_mut().clear();
    probes.velocities.borrow_mut().clear();
    pointer_down(&mut runtime, 300.0, 212.0);
    let _ = frames.pump(&mut runtime);
    for step in 1_u16..=6 {
        pointer_move(&mut runtime, f32::from(step).mul_add(-20.0, 300.0), 212.0);
        let _ = frames.pump(&mut runtime);
    }
    assert!(
        probes
            .positions
            .borrow()
            .iter()
            .flatten()
            .any(|&p| p > 1.0 && p < 2.0),
        "dragging across tabs must produce continuous fractional indicator positions"
    );
    let drag_vel = probes
        .velocities
        .borrow()
        .iter()
        .rev()
        .find_map(|v| *v)
        .expect("dragging a tab indicator must populate smoothed velocity");
    assert!(
        drag_vel.x < -100.0,
        "dragging left must produce negative horizontal velocity, got {drag_vel:?}"
    );

    // Release at x = 180 (tab 1 centre) -> settles on tab 1.
    pointer_up(&mut runtime, 180.0, 212.0);
    let _ = frames.pump(&mut runtime);
    assert_eq!(selection.snapshot(), 1);
    assert_eq!(live.selected(), Some(1));
    assert_eq!(live.recordings(), recordings_before);
}
