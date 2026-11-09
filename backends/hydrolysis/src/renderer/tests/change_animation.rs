//! A change that disables animation applies in the next frame, even where
//! the receiver animates a change that names no animation
//! (water-rs/waterui#4035).

use super::*;
use waterui::animation::{Animation, AnimationExt, NoAnimation};

/// The toggle's theme default: what a change naming no animation runs with.
const TOGGLE_DEFAULT: Animation = Animation::linear(Duration::from_millis(100));

/// Turns `source` on and returns the thumb progress `signal` reads half-way
/// through the toggle's default animation.
fn toggle_progress_mid_default<S>(source: &Binding<bool>, signal: &S) -> f32
where
    S: Signal<Output = bool> + Clone + 'static,
{
    let mut renderer = test_renderer();
    let owner = Rc::new(());
    let (initial, selected) = renderer.resolve_toggle_progress(signal, &owner, TOGGLE_DEFAULT);
    approx::assert_relative_eq!(initial, 0.0);
    assert!(!selected);

    let started = renderer.frame_instant();
    source.set(true);
    renderer.set_frame_instant(started + Duration::from_millis(50));
    let (progress, selected) = renderer.resolve_toggle_progress(signal, &owner, TOGGLE_DEFAULT);
    assert!(selected);
    progress
}

#[test]
fn toggle_change_naming_no_animation_runs_the_default() {
    let source = Binding::container(false);
    let progress = toggle_progress_mid_default(&source, &source.clone());
    assert!(
        progress > 0.0 && progress < 1.0,
        "a change naming no animation must run the toggle's default, got {progress}"
    );
}

#[test]
fn toggle_change_with_animation_disabled_snaps() {
    let source = Binding::container(false);
    let progress = toggle_progress_mid_default(&source, &source.clone().with(NoAnimation));
    approx::assert_relative_eq!(progress, 1.0);
}

#[test]
fn disabling_animation_wins_over_an_attached_animation() {
    let source = Binding::container(false);
    let signal = source.animated().with(NoAnimation);
    let progress = toggle_progress_mid_default(&source, &signal);
    approx::assert_relative_eq!(progress, 1.0);
}

#[test]
fn opacity_change_with_animation_disabled_needs_no_animation_frames() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
    let source = Binding::container(false);
    let view = || {
        text("Fade").opacity(
            source
                .clone()
                .map(|visible| if visible { 1.0_f32 } else { 0.0 })
                .with(Animation::linear(Duration::from_secs(1)))
                .with(NoAnimation),
        )
    };
    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.animation_refresh_range(), None);

    let started = renderer.frame_instant();
    source.set(true);
    renderer.set_frame_instant(started + Duration::from_millis(50));
    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.animation_refresh_range(), None);
}
