//! Refresh-rate vote classification and `.frame_rate(...)` lifecycle tests
//! (water-rs/waterui#4638).

use core::time::Duration;

#[cfg(hydrolysis_gpu_painter)]
use nami::SignalExt as _;
use waterui::accessibility::AccessibilityRole;
#[cfg(hydrolysis_gpu_painter)]
use waterui::animation::Animation;
use waterui::animation::FrameRate;
use waterui::gesture::TapGesture;
use waterui::prelude::{text, when};
use waterui::{AnyView, Color, ViewExt as _};
use waterui_core::Binding;
use waterui_core::handler::AnyViewBuilder;
#[cfg(hydrolysis_gpu_painter)]
use waterui_frame_rate::combine;

#[cfg(not(target_arch = "wasm32"))]
use super::capture_root_window;
use super::{Rect, test_environment, test_renderer};
#[cfg(not(target_arch = "wasm32"))]
use crate::HeadlessRuntime;
#[cfg(not(target_arch = "wasm32"))]
use crate::platform::{InputEvent, PointerButton, PointerKind};
#[cfg(not(target_arch = "wasm32"))]
use crate::testing::TestStyle;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn static_view_has_no_frame_rate_vote() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
    capture_root_window(&mut renderer, text("Static"), &env, bounds);
    assert_eq!(renderer.frame_rate_vote(), None);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mounted_visible_frame_rate_modifier_votes_and_unmounting_clears() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
    let mounted = Binding::container(true);
    let rate = FrameRate::fixed(24.0);
    let view = || {
        when(mounted.clone(), move || {
            text("Cinema").frame_rate(Some(rate))
        })
    };

    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.frame_rate_vote(), Some(rate));

    mounted.set(false);
    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.frame_rate_vote(), None);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn zero_opacity_suppresses_frame_rate_vote() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
    let opacity = Binding::container(0.0_f32);
    let view = || {
        text("Hidden")
            .frame_rate(Some(FrameRate::FAST))
            .opacity(opacity.clone())
    };

    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.frame_rate_vote(), None);

    opacity.set(1.0);
    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.frame_rate_vote(), Some(FrameRate::FAST));
}

#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
#[test]
fn hidden_modifier_suppresses_frame_rate_vote() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
    let visible = Binding::container(false);
    let view = || {
        text("Conditional")
            .frame_rate(Some(FrameRate::FAST))
            .visible(visible.clone())
    };

    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.frame_rate_vote(), None);

    visible.set(true);
    capture_root_window(&mut renderer, view(), &env, bounds);
    assert_eq!(renderer.frame_rate_vote(), Some(FrameRate::FAST));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn pointer_down_votes_fast_until_pointer_up() {
    let env = test_environment();
    let builder = AnyViewBuilder::new(|| {
        AnyView::new(
            Color::srgb_hex("#3F3F46")
                .gesture(TapGesture::new(), || {})
                .a11y_role(AccessibilityRole::Button),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 120, 120, TestStyle::new());
    let _ = runtime.pump(false);
    assert_eq!(runtime.renderer_frame_rate_vote(), None);

    runtime.push_input_event(InputEvent::PointerDown {
        id: 1,
        kind: PointerKind::Mouse,
        x: 60.0,
        y: 60.0,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump(false);
    assert_eq!(runtime.renderer_frame_rate_vote(), Some(FrameRate::FAST));

    runtime.push_input_event(InputEvent::PointerUp {
        id: 1,
        kind: PointerKind::Mouse,
        x: 60.0,
        y: 60.0,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump(false);
    let settled = runtime.frame_instant() + Duration::from_secs(2);
    let _ = runtime.pump_at(false, settled);
    assert_eq!(runtime.renderer_frame_rate_vote(), None);
}

#[cfg(hydrolysis_gpu_painter)]
#[test]
fn lower_app_vote_never_lowers_active_fade_rate() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
    let visible = Binding::container(true);
    let rate = FrameRate::fixed(24.0);
    let view = || {
        text("Combo")
            .opacity(
                visible
                    .clone()
                    .map(|shown| if shown { 1.0_f32 } else { 0.25 })
                    .with(Animation::linear(Duration::from_secs(1))),
            )
            .frame_rate(Some(rate))
    };

    capture_root_window(&mut renderer, view(), &env, bounds);
    renderer.commit_mirror();
    let started = renderer.frame_instant();
    assert_eq!(renderer.frame_rate_vote(), Some(rate));
    assert_eq!(renderer.mirror().sample(started).rate, None);

    visible.set(false);
    renderer.commit_mirror();
    let mid = started + Duration::from_millis(50);
    renderer.set_frame_instant(mid);
    assert_eq!(
        combine(
            renderer.frame_rate_vote(),
            renderer.mirror().sample(mid).rate,
        ),
        Some(FrameRate::SLOW)
    );
}
