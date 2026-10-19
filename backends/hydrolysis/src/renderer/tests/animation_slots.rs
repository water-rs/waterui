//! Per-view animation slots are keyed on the view that draws them
//! (water-rs/waterui#2240). Nami derives a mapped signal's identity from its
//! source and call site, so views fed by signals from one call site share a
//! `SignalIdentity`; each test here drives such a pair through one kind of
//! slot and checks every view draws its own value.

use super::*;
use waterui::animation::Animation;
use waterui::component::progress;

/// `48.0` while `source` holds `index`, `0.0` otherwise, animated linearly
/// over 100ms. Every call mints its signal at the same call site.
fn slide(source: &Binding<i32>, index: i32) -> impl Signal<Output = f32> {
    source
        .clone()
        .map(move |value| if value == index { 48.0 } else { 0.0 })
        .with(Animation::linear(Duration::from_millis(100)))
}

/// Picker `index`'s selection out of the pair in `source`. Every call mints
/// its mapping at the same call site.
fn selection(source: &Binding<[i32; 2]>, index: usize) -> Binding<i32> {
    Binding::mapping(
        source,
        move |pair| pair[index],
        move |source: &Binding<[i32; 2]>, value| source.with_mut(|pair| pair[index] = value),
    )
}

/// The leading edge of every interactive target, top to bottom.
fn target_leading_edges(renderer: &HydrolysisRenderer) -> Vec<f64> {
    let mut targets = renderer
        .hit_test
        .pointer_targets
        .iter()
        .filter(|target| target.interaction.is_some())
        .map(|target| target.bounds)
        .collect::<Vec<_>>();
    targets.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    targets.iter().map(|bounds| bounds.x0).collect()
}

/// A signal-driven scalar (the `.opacity` / `.scale` / `.rotation` /
/// `.offset` / morph-progress slot): driving one view's offset animates it
/// while its sibling stays put.
#[test]
fn offsets_sharing_a_signal_identity_slide_independently() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
    let source = Binding::container(0_i32);
    assert_eq!(
        slide(&source, 1).identity(),
        slide(&source, 2).identity(),
        "same-site signals must share a SignalIdentity for this test to cover the bug"
    );
    let view = || {
        vstack((
            button("A").action(|| {}).offset(slide(&source, 1), 0.0),
            button("B").action(|| {}).offset(slide(&source, 2), 0.0),
        ))
    };

    capture_root_window(&mut renderer, view(), &env, bounds);
    let rest = target_leading_edges(&renderer);
    assert_eq!(rest.len(), 2, "both buttons must register a target");

    let started = renderer.frame_instant();
    source.set(2);
    renderer.set_frame_instant(started + Duration::from_millis(50));
    capture_root_window(&mut renderer, view(), &env, bounds);
    let mid = target_leading_edges(&renderer);
    approx::assert_relative_eq!(mid[0], rest[0]);
    assert!(
        mid[1] > rest[1] && mid[1] < rest[1] + 48.0,
        "B must be mid-slide from {} toward {}, got {}",
        rest[1],
        rest[1] + 48.0,
        mid[1]
    );

    renderer.set_frame_instant(started + Duration::from_millis(200));
    capture_root_window(&mut renderer, view(), &env, bounds);
    let settled = target_leading_edges(&renderer);
    approx::assert_relative_eq!(settled[0], rest[0]);
    approx::assert_relative_eq!(settled[1], rest[1] + 48.0);
}

/// A target-driven widget scalar (the progress fill and the text-field label
/// float): each indicator fills to its own value.
#[test]
fn progress_indicators_sharing_a_signal_identity_fill_to_their_own_values() {
    let track_draws = Rc::new(RefCell::new(Vec::new()));
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        progress_linear_track_draws: Rc::clone(&track_draws),
        ..Default::default()
    });
    let source = Binding::container(1_i32);
    let fraction = |index: i32| {
        source
            .clone()
            .map(move |value| if value == index { 0.8 } else { 0.2 })
    };
    assert_eq!(
        fraction(1).identity(),
        fraction(2).identity(),
        "same-site signals must share a SignalIdentity for this test to cover the bug"
    );

    capture_root_window(
        &mut renderer,
        vstack((progress(fraction(1)), progress(fraction(2)))),
        &test_environment(),
        Rect::new(0.0, 0.0, 200.0, 200.0),
    );

    let mut draws = track_draws.borrow_mut().drain(..).collect::<Vec<_>>();
    draws.sort_by(|a, b| a.0.y0.total_cmp(&b.0.y0));
    let filled = draws
        .iter()
        .map(|(track, end)| {
            let end = end.expect("a determinate indicator resolves its active end");
            (end - track.x0) / track.width()
        })
        .collect::<Vec<_>>();
    assert_eq!(filled.len(), 2, "both indicators must draw a track");
    approx::assert_relative_eq!(filled[0], 0.8, epsilon = 1e-6);
    approx::assert_relative_eq!(filled[1], 0.2, epsilon = 1e-6);
}

/// The radio-indicator choreography: each picker draws its own selection.
#[test]
fn radio_pickers_sharing_a_signal_identity_draw_their_own_selection() {
    let indicator_draws = Rc::new(RefCell::new(Vec::new()));
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        radio_indicator_draws: Rc::clone(&indicator_draws),
        ..Default::default()
    });
    let source = Binding::container([0_i32, 1_i32]);
    let (first, second) = (selection(&source, 0), selection(&source, 1));
    assert_eq!(
        first.identity(),
        second.identity(),
        "same-site mappings must share a SignalIdentity for this test to cover the bug"
    );
    let options = || vec![text("One").tag(0_i32), text("Two").tag(1_i32)];

    capture_root_window(
        &mut renderer,
        vstack((
            Picker::new("First", options(), &first).style(PickerStyle::Radio),
            Picker::new("Second", options(), &second).style(PickerStyle::Radio),
        )),
        &test_environment(),
        Rect::new(0.0, 0.0, 200.0, 240.0),
    );

    let mut draws = indicator_draws.borrow_mut().drain(..).collect::<Vec<_>>();
    draws.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
    let selected = draws
        .iter()
        .map(|(_, state)| state.outer_selected_progress)
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 4, "two pickers of two options each");
    for (row, expected) in [1.0, 0.0, 0.0, 1.0].into_iter().enumerate() {
        approx::assert_relative_eq!(selected[row], expected);
    }
}
