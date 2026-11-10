//! A change that disables animation applies in the next frame, even where
//! the receiver animates a change that names no animation
//! (water-rs/waterui#4035).

use super::*;
use waterui::animation::{Animation, NoAnimation};

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
