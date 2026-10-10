use super::headless::HeadlessPlatformWindow;
use super::{
    FrameMode, FrameReader, RenderDiagnosticsConfig, RuntimeWindow, acquire_surface_frame,
    advance_runtime, axes_whose_limits_changed, clamp_window_size, handle_input_events,
    pump_window_semantics, render_window, render_window_with_capture, reports_ui_idle,
    schedule_animation_update, schedule_redraw_or_refresh, surface_error_requires_reconfigure,
};
use crate::platform::{
    GpuSurfaceWindow as _, InputEvent, OffscreenSurface, PlatformWindow as _, SurfaceError,
    SurfaceFrame, SurfaceProvider,
};
use crate::renderer::tests::MinimalTestTheme;
use crate::renderer::{FontFamilyResolution, HydrolysisRenderer, InteractionKey};
use crate::text::SessionTextEngine;
use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;
use waterui::component::list::{List, ListItem};
use waterui::window::{Window, WindowState};
use waterui::{Binding, Signal, ViewExt as _};
use waterui_backend_core::widget::TextCaretMotion;
use waterui_core::animation::Animation;
use waterui_core::id::SelfId;
use waterui_core::{AnyView, Environment, binding};
use waterui_graphics::Color;
use waterui_layout::scroll::ScrollController;

#[test]
fn changed_rebuild_input_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();
    runtime.renderer.root_cell().mark_layout();

    schedule_redraw_or_refresh(&mut runtime, true);

    assert!(runtime.mode.is_pending());
    assert!(
        runtime.platform.take_redraw_request(),
        "rebuild input must wake the platform event loop for the next frame"
    );
}

#[test]
fn changed_reactive_input_refreshes_retained_tree() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();
    runtime.renderer.context_mark_layout();

    schedule_redraw_or_refresh(&mut runtime, true);

    assert!(
        runtime.mode == FrameMode::Refresh,
        "a pending reactive patch must be sampled before presenting the next frame"
    );
    assert!(
        runtime.platform.take_redraw_request(),
        "reactive input must wake the platform event loop"
    );
}

#[test]
fn changed_scroll_input_schedules_a_full_frame_and_wakes_platform_window() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();

    // A consumed scroll schedules the one per-frame pass — patch, layout,
    // re-encode — like every other content change.
    schedule_redraw_or_refresh(&mut runtime, true);

    assert!(runtime.mode.is_pending());
    assert!(
        runtime.mode == FrameMode::Refresh,
        "every awake frame runs the full pass, layout included"
    );
    assert!(
        runtime.platform.take_redraw_request(),
        "scroll input must wake the platform event loop for the next frame"
    );
}

#[test]
fn animation_ticks_schedule_full_frames() {
    let mut runtime = test_runtime_window();
    runtime.clear_frame_mode();

    schedule_animation_update(&mut runtime, true);

    assert!(runtime.mode.is_pending());
    assert!(
        runtime.mode == FrameMode::Animate,
        "an animation tick schedules the same full frame as any content change, \
         marked as continuation work rather than an unapplied update"
    );
}

#[test]
fn only_invalid_swap_chains_require_surface_reconfiguration() {
    assert!(surface_error_requires_reconfigure(SurfaceError::Lost));
    assert!(surface_error_requires_reconfigure(SurfaceError::Outdated));
    assert!(!surface_error_requires_reconfigure(SurfaceError::Timeout));
    assert!(!surface_error_requires_reconfigure(SurfaceError::Occluded));
}

#[test]
fn invalid_swap_chain_is_reconfigured_and_retried_in_the_same_frame() {
    let mut surface = RecoveringSurface::new(SurfaceError::Outdated);

    let frame = acquire_surface_frame(&mut surface)
        .expect("surface acquisition must recover after reconfiguration");

    assert_eq!(surface.acquire_count, 2);
    assert_eq!(surface.resize_count, 1);
    surface.present(frame);
}

#[test]
fn text_caret_tick_wakes_redraw_without_layout_rebuild() {
    let mut runtime = test_runtime_window();
    let now = Instant::now();
    let motion = TextCaretMotion {
        fade_cycle_duration: Duration::from_millis(1_000),
        frame_interval: Duration::from_millis(16),
        min_opacity: 0.2,
    };
    runtime.renderer.set_frame_instant(now);
    runtime.renderer.set_text_caret_motion(motion);
    // Focus by identity: the caret animates off the focused field's identity, and
    // this runtime emits no text-input targets, so there is no position to name.
    // Every focused key has an owner; the root cell owns this one.
    let focused_field = Rc::new(());
    let key = InteractionKey::for_rc(&focused_field, 0);
    runtime.renderer.bind_root_owned_key(&key);
    assert!(runtime.renderer.set_focused_text_input_key(Some(key)));
    assert!(runtime.renderer.root_is_dirty());
    assert!(
        !runtime
            .renderer
            .root_marks()
            .contains(crate::renderer::Dirty::STRUCTURE)
    );
    assert!(runtime.renderer.take_patch_request());
    runtime.clear_frame_mode();
    assert!(!runtime.platform.take_redraw_request());

    let deadline = now
        .checked_add(motion.frame_interval)
        .expect("test caret deadline overflow");
    let env = Environment::new();

    assert!(advance_runtime(&mut runtime, &env, deadline).is_some());
    assert!(
        !runtime.mode.is_pending(),
        "the caret repaints through the transient text-input overlay on present; \
         a tick must not schedule retained-scene work, or an idle window could \
         never stay idle"
    );
    assert!(runtime.renderer.take_redraw_request());
    assert!(runtime.platform.take_redraw_request());
}

/// A hidden window parks the pump: no frame renders, no wake deadline or
/// platform redraw is posted, and work armed while hidden stays armed —
/// the contract's "no frames, no wakes, no GPU pulls" half. Un-hiding
/// renders exactly one frame from the current state, not a replay of the
/// frames that were skipped.
#[test]
fn hidden_window_parks_the_pump_and_restores_exactly_one_frame() {
    let mut runtime = test_runtime_window();
    let env = crate::renderer::tests::test_environment();
    let mut now = Instant::now();

    // Settle the mount frames; the window goes idle on its own.
    let idle_frames = drive_until_idle(&mut runtime, &env, &mut now, 60);
    assert!(idle_frames < 60, "the window never went idle before hiding");
    let presented_before = runtime.presented_frames;

    runtime.set_hidden(true);
    // Work that lands while hidden stays armed: neither the pump tick nor
    // a platform redraw already in flight when the window hid may render it.
    runtime.renderer.root_cell().mark_layout();
    now += Duration::from_millis(32);
    assert!(
        advance_runtime(&mut runtime, &env, now).is_none(),
        "a hidden window reports no wake deadline"
    );
    assert!(
        !render_window(&mut runtime, &env, &mut || false),
        "a hidden window presents no frame"
    );
    runtime.platform.request_redraw();
    assert!(
        !render_window(&mut runtime, &env, &mut || false),
        "a stale in-flight wake renders nothing either"
    );
    assert_eq!(
        runtime.presented_frames, presented_before,
        "frames presented while hidden"
    );
    let _ = runtime.platform.take_redraw_request();
    assert!(
        !runtime.platform.take_redraw_request(),
        "a hidden window posts no wakes — armed work stays armed"
    );

    // Visibility returns: the armed rebuild and the refresh the un-hide
    // schedules produce exactly one frame, and the pump idles after it.
    runtime.set_hidden(false);
    assert!(runtime.mode.is_pending(), "un-hiding must arm a refresh");
    let mut rendered = 0;
    for _ in 0..10 {
        now += Duration::from_millis(16);
        let _ = advance_runtime(&mut runtime, &env, now);
        let wake = runtime.mode.is_pending() | runtime.platform.take_redraw_request();
        if !wake {
            break;
        }
        if render_window(&mut runtime, &env, &mut || false) {
            rendered += 1;
        }
    }
    assert_eq!(rendered, 1, "un-hiding must render exactly one frame");
}

/// An animation in flight does not wake a hidden pump: no gesture
/// deadline, no platform redraw — the armed wake the visible pump would
/// post simply never runs.
#[test]
fn hidden_window_reports_no_deadline_for_an_armed_animation() {
    let mut runtime = test_runtime_window();
    let now = Instant::now();
    let motion = TextCaretMotion {
        fade_cycle_duration: Duration::from_millis(1_000),
        frame_interval: Duration::from_millis(16),
        min_opacity: 0.2,
    };
    runtime.renderer.set_frame_instant(now);
    runtime.renderer.set_text_caret_motion(motion);
    let focused_field = Rc::new(());
    let key = InteractionKey::for_rc(&focused_field, 0);
    runtime.renderer.bind_root_owned_key(&key);
    assert!(runtime.renderer.set_focused_text_input_key(Some(key)));

    let env = Environment::new();
    let deadline = now
        .checked_add(motion.frame_interval)
        .expect("test caret deadline overflow");

    // The same arm the caret test proves wakes a visible pump.
    runtime.set_hidden(true);
    assert!(
        advance_runtime(&mut runtime, &env, deadline).is_none(),
        "an armed caret animation must not wake a hidden window"
    );
    assert!(
        !runtime.platform.take_redraw_request(),
        "an armed caret animation must not post a redraw while hidden"
    );
    assert!(
        !render_window(&mut runtime, &env, &mut || false),
        "a hidden window presents no frame"
    );
}

/// The pump's "first frame presented; ui idle" readiness line must stay
/// quiet while parked: the Choreographer wake a parked pump cannot unpost
/// presents nothing, and a present-named line there reads as a frame
/// presented while hidden — the false positive a device run counts.
#[test]
fn hidden_window_reports_no_ui_idle_readiness() {
    assert!(
        !reports_ui_idle(true, false, true),
        "a parked pump must not report 'first frame presented; ui idle'"
    );
    assert!(
        reports_ui_idle(true, false, false),
        "a visible pump going idle after presenting reports readiness"
    );
    assert!(
        !reports_ui_idle(false, false, false),
        "readiness requires a presented frame"
    );
    assert!(
        !reports_ui_idle(true, true, false),
        "readiness requires the pump to be idle"
    );
}

/// The un-hide contract hosts without an about-to-wait pass rely on:
/// `sync_occlusion_and_post_restore` posts exactly one restore wake when
/// the platform report flips the pump back to visible. It is what the
/// Android host calls from `set_visible` and `surface_resized` — the
/// resize that gives a band parked on a 0x0 attach its real extent —
/// where only a Choreographer post reaches the frame scheduler.
/// `sync_occlusion` itself must never post: a winit desktop's platform
/// delivers its own restore event, and a second post would double the
/// restore frame.
#[test]
fn un_hide_sync_posts_exactly_one_restore_wake() {
    let mut runtime = test_runtime_window();
    runtime.platform.set_occluded(true);
    runtime.sync_occlusion();
    assert!(runtime.is_hidden(), "an occluded report must park the pump");
    assert!(
        !runtime.platform.take_redraw_request(),
        "sync_occlusion arms only — the restore post is the host's choice"
    );

    runtime.platform.set_occluded(false);
    runtime.sync_occlusion_and_post_restore();
    assert!(!runtime.is_hidden(), "a clear report must unpark the pump");
    assert!(
        runtime.platform.take_redraw_request(),
        "un-hiding through the posting sync must wake the frame scheduler"
    );
    assert!(
        !runtime.platform.take_redraw_request(),
        "the restore wake is a single post, not a stream"
    );

    // Re-syncing a window already visible posts nothing again.
    runtime.sync_occlusion_and_post_restore();
    assert!(
        !runtime.platform.take_redraw_request(),
        "a sync that did not un-hide posts no wake"
    );
}

/// `request_redraw`'s hidden gate: callers outside `advance_runtime` —
/// the winit runner's cross-window rebuild flush, GPU settle wakes, a
/// stale in-flight platform post — reach the platform's redraw post
/// directly, and a parked window must absorb them. Without the gate each
/// of those would keep waking a hidden pump.
#[test]
fn a_hidden_window_absorbs_direct_redraw_requests() {
    let mut runtime = test_runtime_window();
    let _ = runtime.platform.take_redraw_request();

    runtime.set_hidden(true);
    runtime.request_redraw();
    assert!(
        !runtime.platform.take_redraw_request(),
        "a hidden window must not reach the platform redraw post"
    );

    runtime.set_hidden(false);
    runtime.request_redraw();
    assert!(
        runtime.platform.take_redraw_request(),
        "a visible window's redraw request must reach the platform"
    );
}

/// The window's effective size limits reach the platform: the content's
/// measured minimum is the default, the maximum stays unbounded unless the
/// app pins one, and explicit limits override both.
#[test]
fn window_size_limits_reach_the_platform_window() {
    use waterui_core::layout::Size;
    use waterui_layout::frame::Frame;

    // Content with finite bounds declares no stretch axis, so the measured
    // minimum reaches the platform while the maximum stays open — the window
    // resizes and maximizes with the content laid out inside it.
    let content = || {
        Frame::new(())
            .min_width(200.0)
            .min_height(100.0)
            .max_width(640.0)
            .max_height(480.0)
    };
    let window = Window::new("", binding(WindowState::Normal), content);
    let mut runtime = runtime_window_for(window);
    let env = Environment::new();
    let _ = super::pump_window_semantics(&mut runtime, &env);
    let (min, max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    assert_eq!(min, Some(Size::new(200.0, 100.0)));
    assert_eq!(max, None);

    // Explicit limits win over the content-derived ones on both axes.
    let window = Window::new("", binding(WindowState::Normal), content)
        .min_size(Size::new(300.0, 150.0))
        .max_size(Size::new(640.0, 480.0));
    let mut runtime = runtime_window_for(window);
    let _ = super::pump_window_semantics(&mut runtime, &env);
    let (min, max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    assert_eq!(min, Some(Size::new(300.0, 150.0)));
    assert_eq!(max, Some(Size::new(640.0, 480.0)));
}

/// Clamping only moves an axis that falls outside the new limits, to the
/// nearer bound; a size inside the limits passes through untouched.
#[test]
fn clamp_window_size_only_moves_out_of_bounds_axes() {
    use waterui_core::layout::Size;

    let min = Some(Size::new(200.0, 100.0));
    let max = Some(Size::new(640.0, 480.0));

    // Inside the limits: untouched — a re-measure keeps the user's size.
    assert_eq!(
        clamp_window_size(Size::new(400.0, 300.0), min, max, (true, true)),
        Size::new(400.0, 300.0)
    );
    // Below the minimum: lifted to it.
    assert_eq!(
        clamp_window_size(Size::new(50.0, 300.0), min, max, (true, true)),
        Size::new(200.0, 300.0)
    );
    // Above the maximum: pulled down to it.
    assert_eq!(
        clamp_window_size(Size::new(800.0, 300.0), min, max, (true, true)),
        Size::new(640.0, 300.0)
    );
    // Out of bounds on one axis only: the other axis passes through.
    assert_eq!(
        clamp_window_size(Size::new(50.0, 700.0), min, max, (true, true)),
        Size::new(200.0, 480.0)
    );
    // An inverted range floors the maximum at the minimum rather than
    // reporting an empty box.
    assert_eq!(
        clamp_window_size(
            Size::new(400.0, 300.0),
            Some(Size::new(700.0, 100.0)),
            Some(Size::new(640.0, 480.0)),
            (true, true),
        ),
        Size::new(700.0, 300.0)
    );
}

/// A re-apply clamps only the axes whose applied limits changed: a
/// width-only move never snaps the height, and the first apply — with no
/// previous limits — moves no axis at all, so a launch frame below the
/// content minimum keeps its size until that axis's own limit moves.
#[test]
fn a_reapply_clamps_only_the_axes_whose_limits_changed() {
    use waterui_core::layout::Size;

    let installed = (Some(Size::new(200.0, 100.0)), Some(Size::new(640.0, 480.0)));
    let wider_min = (Some(Size::new(500.0, 100.0)), Some(Size::new(640.0, 480.0)));
    let taller_max = (Some(Size::new(200.0, 100.0)), Some(Size::new(640.0, 400.0)));

    // The first apply has no previous limits: it installs, moving no axis.
    assert_eq!(axes_whose_limits_changed(None, installed), (false, false));
    // A width-only move names the width axis; a height-max move the height.
    assert_eq!(
        axes_whose_limits_changed(Some(installed), wider_min),
        (true, false)
    );
    assert_eq!(
        axes_whose_limits_changed(Some(installed), taller_max),
        (false, true)
    );
    // An identical re-apply moves nothing.
    assert_eq!(
        axes_whose_limits_changed(Some(installed), installed),
        (false, false)
    );

    // The frame therefore clamps on the moved axis only: 90 is below the
    // height minimum, yet stays put while only the width's limit moved —
    // and lifts only when the height's own limit moves.
    let settled = Size::new(600.0, 90.0);
    assert_eq!(
        clamp_window_size(settled, wider_min.0, wider_min.1, (true, false)),
        Size::new(600.0, 90.0)
    );
    assert_eq!(
        clamp_window_size(settled, wider_min.0, wider_min.1, (true, true)),
        Size::new(600.0, 100.0)
    );
}

/// A root stretching on one axis — a text field, a row with a `Spacer`, a
/// frame pinned infinite on one side — leaves the window maximum unbounded
/// on both axes: the axis it does not claim is laid out inside a larger
/// offer per the layout spec, not capped to a measurement.
#[test]
fn stretch_axis_content_leaves_the_window_maximum_unbounded() {
    use waterui_layout::frame::Frame;

    let window = Window::new("", binding(WindowState::Normal), || {
        Frame::new(().size(100.0, 50.0)).max_width(f32::INFINITY)
    });
    let mut runtime = runtime_window_for(window);
    let _ = super::pump_window_semantics(&mut runtime, &crate::renderer::tests::test_environment());
    let (min, max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    assert!(min.is_some());
    assert_eq!(
        max, None,
        "a stretching root applies no window maximum on either axis"
    );
}

/// An app-pinned maximum may leave one axis unbounded explicitly: a `+∞`
/// component inside `Window::max_size` binds only the other axis — a window
/// past the bound clamps on that axis and keeps its size on the open one.
#[test]
fn an_infinite_max_size_component_leaves_that_axis_unbounded() {
    use waterui_core::layout::Size;

    let max = binding(Size::new(640.0, 480.0));
    let window = Window::new("", binding(WindowState::Normal), || ().size(100.0, 50.0))
        .max_size(max.clone());
    let mut runtime = runtime_window_for(window);
    let env = crate::renderer::tests::test_environment();
    let _ = pump_window_semantics(&mut runtime, &env);

    // The user stretches the window past the pinned maximum.
    runtime.platform.push_event(InputEvent::Resize {
        width: 900,
        height: 700,
    });
    let _ = handle_input_events(&mut runtime, &env);
    let _ = pump_window_semantics(&mut runtime, &env);
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(900.0, 700.0)
    );

    max.set(Size::new(500.0, f32::INFINITY));
    let _ = pump_window_semantics(&mut runtime, &env);
    let (_, applied_max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    assert_eq!(applied_max, Some(Size::new(500.0, f32::INFINITY)));
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(500.0, 700.0),
        "the bounded axis clamps while the +∞ axis keeps the user size"
    );
}

/// A NaN in the app's `frame` binding is a programming error: the runner
/// panics at the read, naming the field and the value, rather than quietly
/// repairing it deeper in the geometry path.
#[test]
#[should_panic(expected = "Window::frame.width must be finite, got NaN")]
fn a_nan_frame_panics_naming_the_field_and_value() {
    use waterui_core::layout::{Point, Rect, Size};

    let window = Window::new("", binding(WindowState::Normal), || ().size(100.0, 50.0));
    window
        .frame
        .set(Rect::new(Point::new(0.0, 0.0), Size::new(f32::NAN, 300.0)));
    let _ = runtime_window_for(window);
}

/// The same trust boundary applies to the explicit size limits: a NaN
/// `min_size` panics at the read naming the field.
#[test]
#[should_panic(expected = "Window::min_size.width must be finite, got NaN")]
fn a_nan_min_size_panics_naming_the_field_and_value() {
    use waterui_core::layout::Size;

    let window = Window::new("", binding(WindowState::Normal), || ().size(100.0, 50.0))
        .min_size(Size::new(f32::NAN, 100.0));
    let mut runtime = runtime_window_for(window);
    let _ = pump_window_semantics(&mut runtime, &crate::renderer::tests::test_environment());
}

/// `max_size` accepts `+∞` per axis but nothing else non-finite.
#[test]
#[should_panic(
    expected = "Window::max_size.height must be finite or +inf for an unbounded axis, got NaN"
)]
fn a_nan_max_size_panics_naming_the_field_and_value() {
    use waterui_core::layout::Size;

    let window = Window::new("", binding(WindowState::Normal), || ().size(100.0, 50.0))
        .max_size(Size::new(640.0, f32::NAN));
    let mut runtime = runtime_window_for(window);
    let _ = pump_window_semantics(&mut runtime, &crate::renderer::tests::test_environment());
}

/// A re-measure on a screen change updates the limits only: a user-set size
/// inside the new limits is left alone.
#[test]
#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
fn a_remeasure_preserves_the_user_size_inside_the_new_limits() {
    use waterui_core::dynamic::watch;
    use waterui_core::layout::Size;

    let main = binding(false);
    let main_for_view = main.clone();
    let window = Window::new("", binding(WindowState::Normal), move || {
        let main = main_for_view.clone();
        watch(main, |main| {
            ().size(
                if main { 700.0 } else { 200.0 },
                if main { 500.0 } else { 100.0 },
            )
        })
    });
    let mut runtime = runtime_window_sized(window, 800, 600);
    let env = crate::renderer::tests::test_environment();
    let _ = pump_window_semantics(&mut runtime, &env);

    // The user settles on a size beyond the loading screen's box.
    runtime.platform.push_event(InputEvent::Resize {
        width: 900,
        height: 700,
    });
    let _ = handle_input_events(&mut runtime, &env);
    let _ = pump_window_semantics(&mut runtime, &env);
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(900.0, 700.0)
    );

    // The main screen re-measures: only the limits move.
    main.set(true);
    let _ = pump_window_semantics(&mut runtime, &env);
    let (min, max) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");
    assert_eq!(min, Some(Size::new(700.0, 500.0)));
    assert_eq!(max, None);
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(900.0, 700.0),
        "a re-measure must not override a user size inside the new limits"
    );
}

/// A re-measure clamps a user size that falls outside the new limits.
#[test]
fn a_remeasure_clamps_the_window_size_into_the_new_limits() {
    use waterui_core::dynamic::watch;
    use waterui_core::layout::Size;

    let main = binding(false);
    let main_for_view = main.clone();
    let window = Window::new("", binding(WindowState::Normal), move || {
        let main = main_for_view.clone();
        watch(main, |main| {
            ().size(
                if main { 700.0 } else { 200.0 },
                if main { 500.0 } else { 100.0 },
            )
        })
    });
    let mut runtime = runtime_window_sized(window, 800, 600);
    let env = crate::renderer::tests::test_environment();
    let _ = pump_window_semantics(&mut runtime, &env);

    runtime.platform.push_event(InputEvent::Resize {
        width: 400,
        height: 300,
    });
    let _ = handle_input_events(&mut runtime, &env);
    let _ = pump_window_semantics(&mut runtime, &env);
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(400.0, 300.0)
    );

    // The new screen's minimum is larger than the user size: the window clamps
    // into the new limits — and only the size moves, not the layout semantics.
    main.set(true);
    let _ = pump_window_semantics(&mut runtime, &env);
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(700.0, 500.0),
        "a size outside the new limits clamps to the nearer bound"
    );
    let _ = pump_window_semantics(&mut runtime, &env);
    assert_eq!(runtime.platform.surface().size(), (700, 500));
}

/// An explicit `Window::max_size` wins over the user size: a window larger
/// than the pin clamps down to it.
#[test]
fn an_explicit_maximum_clamps_a_larger_window() {
    use waterui_core::layout::Size;

    // The pin is reactive: tightening it below the settled window size is a
    // limits change, so the window is clamped into it.
    let pinned_max = Binding::container(Size::new(2000.0, 2000.0));
    let window = Window::new("", binding(WindowState::Normal), || ()).max_size(pinned_max.clone());
    let mut runtime = runtime_window_sized(window, 800, 600);
    let _ = pump_window_semantics(&mut runtime, &Environment::new());
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(800.0, 600.0),
        "a pin the window already satisfies leaves its size alone"
    );
    pinned_max.set(Size::new(500.0, 400.0));
    let _ = pump_window_semantics(&mut runtime, &Environment::new());
    assert_eq!(
        *runtime.window.frame.snapshot().size(),
        Size::new(500.0, 400.0),
        "tightening the app-pinned maximum clamps the window into it"
    );
}

#[test]
fn zero_layout_minimum_is_not_replaced_by_ideal_size() {
    use waterui_core::layout::{Layout, ProposalSize, Rect, Size, SubView, SubviewPlacement};
    use waterui_layout::container::FixedContainer;

    #[derive(Debug)]
    struct IdealOnlyLayout;

    impl Layout for IdealOnlyLayout {
        fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
            Size::new(
                proposal.width.unwrap_or(500.0),
                proposal.height.unwrap_or(400.0),
            )
        }

        fn place(
            &self,
            _bounds: Rect,
            _proposal: ProposalSize,
            _children: &[&dyn SubView],
        ) -> Vec<SubviewPlacement> {
            Vec::new()
        }
    }

    let window = Window::new("", binding(WindowState::Normal), || {
        FixedContainer::new(IdealOnlyLayout, ())
    });
    let mut runtime = runtime_window_for(window);
    let _ = super::pump_window_semantics(&mut runtime, &Environment::new());
    let (min, _) = runtime
        .platform
        .applied_size_limits()
        .expect("runner must apply size limits on the pump");

    assert!(
        approx::relative_eq!(min.expect("content-derived minimum must exist").width, 0.0),
        "a valid zero minimum must not fall back to the content's ideal width: left {:?}, right {:?}",
        min.expect("content-derived minimum must exist").width,
        0.0
    );
}

/// The reported minimum must be a box the content can actually occupy: text
/// re-wraps at the minimum width, so the minimum height is the wrapped height,
/// not the single-line height a per-axis probe reports.
#[test]
fn window_minimum_is_the_coupled_box_not_independent_axes() {
    use waterui::prelude::text;

    let env = crate::renderer::tests::test_environment();
    let minimum_of = |content: &'static str| {
        let window = Window::new("", binding(WindowState::Normal), move || text(content));
        let mut runtime = runtime_window_for(window);
        let _ = super::pump_window_semantics(&mut runtime, &env);
        runtime
            .platform
            .applied_size_limits()
            .expect("runner must apply size limits on the pump")
            .0
            .expect("content-derived minimum must exist")
    };

    // Five words collapse to one word per line at the minimum width, so the
    // window's minimum height must be five text lines — the per-axis probes
    // used to report one line here, a box the content could never fit in.
    let wrapped = minimum_of("AAAA AAAA AAAA AAAA AAAA");
    let single_line = minimum_of("AAAA");
    assert!(
        wrapped.height >= single_line.height * 4.0,
        "minimum {wrapped:?} must be the height the text needs at its minimum \
         width, not the single-line height {single_line:?}"
    );
    assert!(
        wrapped.width <= single_line.width * 2.0,
        "minimum width {wrapped:?} should be word-granular, not the full line"
    );
}

#[test]
fn rapid_resize_events_keep_the_retained_tree_at_the_latest_size() {
    use std::{cell::Cell, rc::Rc};

    let build_count = Rc::new(Cell::new(0));
    let window = Window::new("", binding(WindowState::Normal), {
        let build_count = Rc::clone(&build_count);
        move || build_count.set(build_count.get() + 1)
    });
    let mut runtime = runtime_window_for(window);
    // The rendered idle-drive below resolves the window's background through
    // the theme, so the test runs against the installed test theme.
    let env = crate::renderer::tests::test_environment();
    let _ = pump_window_semantics(&mut runtime, &env);
    assert_eq!(build_count.get(), 1);

    runtime.platform.push_event(InputEvent::Resize {
        width: 960,
        height: 720,
    });
    runtime.platform.push_event(InputEvent::Resize {
        width: 320,
        height: 240,
    });
    runtime.platform.push_event(InputEvent::Resize {
        width: 640,
        height: 480,
    });
    let _ = handle_input_events(&mut runtime, &env);
    let _ = pump_window_semantics(&mut runtime, &env);

    assert_eq!(
        build_count.get(),
        1,
        "resize must retain the existing view tree"
    );
    assert_eq!(runtime.platform.surface().size(), (640, 480));
    approx::assert_relative_eq!(runtime.window.frame.snapshot().width(), 640.0);
    approx::assert_relative_eq!(runtime.window.frame.snapshot().height(), 480.0);

    // The runtime's own `frame` write on a resize must not loop
    // (water-rs/waterui#2131): the declaration's subscription requests one
    // bounded refresh, then the window goes idle and stays idle.
    let mut now = Instant::now();
    let settle = drive_until_idle(&mut runtime, &env, &mut now, 60);
    assert!(
        settle < 60,
        "the resize must not loop: the window stayed awake for {settle} frames"
    );
}

/// A window declaration input changed while the pump is parked must still
/// reach it: `RuntimeWindow` holds a subscription on every reactive input
/// of the declaration for the window's lifetime, so `handle().set_background`
/// or a `title` write on an idle window requests a frame — and the frame the
/// background write produces paints the new clear colour (water-rs/waterui#2131).
#[test]
fn an_idle_window_repaints_when_its_background_changes() {
    let title = binding(waterui_core::Str::from("old title"));
    let window = Window::new(title.clone(), binding(WindowState::Normal), || ());
    let handle = window.handle();
    let mut runtime = runtime_window_sized(window, 16, 16);
    let env = crate::renderer::tests::test_environment();
    let mut now = Instant::now();

    let idle_frames = drive_until_idle(&mut runtime, &env, &mut now, 60);
    assert!(idle_frames < 60, "the window never went idle");

    handle.set_background(Color::srgb(255, 0, 0));
    assert!(
        runtime.renderer.has_pending_semantic_update(),
        "a write to the window's background binding must request a frame"
    );

    // The flag becomes a scheduled refresh on the next advance, and the
    // frame it runs paints the new clear colour behind the (empty) content.
    now += Duration::from_millis(16);
    let _ = advance_runtime(&mut runtime, &env, now);
    assert!(
        runtime.mode.is_pending(),
        "the binding's update must arm a refresh frame"
    );
    let snapshot =
        render_window_with_capture(&mut runtime, &env, FrameReader::Snapshot, &mut || false)
            .snapshot
            .expect("the refresh frame captures a snapshot");
    let pixel = &snapshot.rgba8[0..4];
    assert!(
        pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60 && pixel[3] == 255,
        "the snapshot must show the new background colour, got {pixel:?}"
    );

    // The repaint is one bounded frame: the window settles and stays idle.
    let settle = drive_until_idle(&mut runtime, &env, &mut now, 60);
    assert!(
        settle < 60,
        "the window never went idle after the background repaint"
    );

    // `title` is a declaration input too — the pump only `.snapshot()`s it
    // per frame, so its subscription is the only thing that wakes an idle
    // window for a rename.
    title.set(waterui_core::Str::from("new title"));
    assert!(
        runtime.renderer.has_pending_semantic_update(),
        "a write to the window's title must request a frame"
    );
    let settle = drive_until_idle(&mut runtime, &env, &mut now, 60);
    assert!(
        settle < 60,
        "the window never went idle after the title rename"
    );
}

#[test]
fn pending_lazy_height_refresh_precedes_scroll_input() {
    use waterui::ViewExt as _;
    use waterui_core::dynamic::watch;
    use waterui_core::id::SelfId;
    use waterui_layout::scroll::scroll;
    use waterui_layout::stack::VStack;

    let expanded = binding(false);
    let expanded_for_view = expanded.clone();
    let rows = vec![SelfId::new(0usize)];
    let window = Window::new("", binding(WindowState::Normal), move || {
        let expanded = expanded_for_view.clone();
        scroll(VStack::for_each(rows.clone(), move |_| {
            let expanded = expanded.clone();
            watch(expanded, |expanded| {
                ().size(800.0, if expanded { 1_200.0 } else { 200.0 })
            })
        }))
    });
    let mut runtime = runtime_window_for(window);
    let env = crate::renderer::tests::test_environment();
    let _ = pump_window_semantics(&mut runtime, &env);

    expanded.set(true);
    runtime.platform.push_event(InputEvent::Scroll {
        x: 400.0,
        y: 300.0,
        dx: 0.0,
        dy: -4.0,
        is_line_delta: false,
    });
    let _ = handle_input_events(&mut runtime, &env);

    let metrics = runtime
        .renderer
        .scroll_metrics_at(400.0, 300.0)
        .expect("scroll target must remain registered after the input geometry refresh");
    assert!(
        metrics.max_y > 8.0,
        "expanded lazy item must update the scroll extent before input dispatch: {metrics:?}"
    );
    assert!(
        runtime
            .renderer
            .handle_scroll(400.0, 300.0, 0.0, -4.0, false),
        "scroll input must see a Dynamic lazy item's latest height before the pending frame is presented"
    );
}

/// The background a frame pumps reaches the platform's compositor hook:
/// `set_blur_behind(true)` for the behind-window levels, `false` for a
/// within-window level and for a colour — the headless window records the
/// last request the way a winit window records its protocol state.
#[test]
fn a_behind_window_material_asks_the_platform_to_blur_behind() {
    use waterui::background::Material;

    let env = crate::renderer::tests::test_environment();
    for (material, expected) in [
        (Material::UltraThin, true),
        (Material::Thin, true),
        (Material::Regular, false),
        (Material::Thick, false),
        (Material::UltraThick, false),
    ] {
        let window = Window::new("", binding(WindowState::Normal), || ()).background(material);
        let mut runtime = runtime_window_for(window);
        render_window(&mut runtime, &env, &mut || false);
        assert_eq!(
            runtime.platform.blur_behind(),
            Some(expected),
            "{material:?} must dispatch set_blur_behind({expected})"
        );
    }

    // A colour asks for no compositor blur, translucent or not.
    for color in [
        waterui::Color::srgb(51, 51, 51),
        waterui::Color::srgb(51, 51, 51).with_opacity(0.5),
    ] {
        let window = Window::new("", binding(WindowState::Normal), || ()).background(color);
        let mut runtime = runtime_window_for(window);
        render_window(&mut runtime, &env, &mut || false);
        assert_eq!(
            runtime.platform.blur_behind(),
            Some(false),
            "a colour window background must dispatch set_blur_behind(false)"
        );
    }
}

fn runtime_window_for(window: Window) -> RuntimeWindow<HeadlessPlatformWindow> {
    runtime_window_sized(window, 16, 16)
}

fn runtime_window_sized(
    window: Window,
    width: u32,
    height: u32,
) -> RuntimeWindow<HeadlessPlatformWindow> {
    let platform =
        HeadlessPlatformWindow::new_for_tests(width, height, wgpu::TextureFormat::Rgba8Unorm);
    runtime_window_with_platform(window, platform)
}

fn runtime_window_with_platform(
    window: Window,
    mut platform: HeadlessPlatformWindow,
) -> RuntimeWindow<HeadlessPlatformWindow> {
    platform.apply_properties(&window);
    let renderer = HydrolysisRenderer::with_engine(
        Rc::new(MinimalTestTheme::default()),
        SessionTextEngine::system(FontFamilyResolution::Strict),
    );
    RuntimeWindow::new(
        window,
        platform,
        renderer,
        RenderDiagnosticsConfig {
            enabled: false,
            interval: Duration::from_secs(1),
            slow_frame_threshold_override: None,
        },
    )
}

/// An animated `List` scroll must keep the window awake until it settles.
///
/// This drives the runtime the way the winit loop does — a frame runs only while
/// the runtime is still asking to be woken — so a scroll that fails to schedule
/// its own animation frames shows up as the loop going idle almost immediately.
/// Pumping frames unconditionally cannot see that, because it supplies the very
/// frames the bug withholds.
#[test]
fn an_animated_list_scroll_keeps_the_window_awake_until_it_settles() {
    const ROWS: usize = 200;
    const TARGET_ROW: usize = 40;
    /// Hard stop so a runaway loop fails loudly instead of hanging.
    const MAX_FRAMES: usize = 400;

    let controller = ScrollController::new(0usize);
    let controller_for_view = controller.clone();
    let window = Window::new("", binding(WindowState::Normal), move || {
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        AnyView::new(
            List::for_each(rows, |_row| {
                ListItem::new(().size(160.0, ROW_HEIGHT_FOR_JUMP_TEST))
            })
            .scroll_controller(&controller_for_view),
        )
    });
    let mut runtime = runtime_window_sized(window, 160, 320);
    let env = crate::renderer::tests::test_environment();
    let mut now = Instant::now();

    // Settle the initial layout, then let the window go idle.
    let idle_frames = drive_until_idle(&mut runtime, &env, &mut now, MAX_FRAMES);
    assert!(
        idle_frames < MAX_FRAMES,
        "the window never went idle before the jump"
    );

    controller.animate_to(TARGET_ROW, Animation::default());
    // The frame the button press itself produces: the loop is already running an
    // iteration for that input, and this is where the scroll gets armed.
    now += Duration::from_millis(16);
    let _ = advance_runtime(&mut runtime, &env, now);
    render_window(&mut runtime, &env, &mut || false);

    // Everything after this point has to be self-sustaining. The flight is
    // continuation work, never an unapplied change: no frame of it may leave
    // the tree it emitted stale (water-rs/waterui#2243).
    let animation_frames =
        drive_until_idle_checking(&mut runtime, &env, &mut now, MAX_FRAMES, |runtime| {
            assert!(
                !runtime.renderer.has_pending_semantic_update(),
                "a gliding list scroll must not leave a pending semantic update after its frame"
            );
        });

    assert!(
        animation_frames < MAX_FRAMES,
        "the scroll never settled: the window stayed awake for {animation_frames} frames"
    );
    // `Animation::default()` resolves to a 250ms ease-in-out, so a real
    // animation spans many frames. A scroll that teleported, or one whose
    // frames were never scheduled, would idle again almost at once.
    assert!(
        animation_frames >= 8,
        "an animated scroll should span many frames; the window went idle after \
         {animation_frames}, so the scroll landed without animating"
    );
}

/// Runs frames while the runtime is still asking to be woken, returning how many
/// it took to go idle. This is the winit loop's rule: no wake request, no frame.
fn drive_until_idle(
    runtime: &mut RuntimeWindow<HeadlessPlatformWindow>,
    env: &Environment,
    now: &mut Instant,
    max_frames: usize,
) -> usize {
    drive_until_idle_checking(runtime, env, now, max_frames, |_| {})
}

/// [`drive_until_idle`], running `check` on the window after every frame.
fn drive_until_idle_checking(
    runtime: &mut RuntimeWindow<HeadlessPlatformWindow>,
    env: &Environment,
    now: &mut Instant,
    max_frames: usize,
    mut check: impl FnMut(&RuntimeWindow<HeadlessPlatformWindow>),
) -> usize {
    for frame in 0..max_frames {
        let wake = runtime.mode.is_pending()
            | runtime.platform.take_redraw_request()
            | runtime.renderer.has_pending_frame_request();
        if !wake {
            return frame;
        }
        *now += Duration::from_millis(16);
        let _ = advance_runtime(runtime, env, *now);
        render_window(runtime, env, &mut || false);
        check(runtime);
    }
    max_frames
}

/// Row height used by the jump test; any fixed height works, it only has to be
/// stable so the target sits a predictable distance away.
const ROW_HEIGHT_FOR_JUMP_TEST: f32 = 24.0;

/// The one-time build path records the presentation hosts once, inside
/// the window capture: a `.context_menu` presentation already open when
/// the tree is first built mounts there under its host cell — painted and
/// registered once, never again outside a reader.
#[test]
fn an_open_context_menu_presentation_builds_once_through_the_build_path() {
    use waterui_controls::menu::{CommandExt as _, ResolvedMenuItem};

    let mut runtime = sized_test_runtime_window(320, 240);
    let mut env = crate::renderer::tests::test_environment();
    env.insert(crate::renderer::HydrolysisWindowOrigin { x: 0.0, y: 0.0 });
    let items = vec![ResolvedMenuItem::Command(
        "Copy".action(|| {}).resolve(&env),
    )];
    let nodes = crate::renderer::popup_menu_nodes(&items, &env, runtime.renderer.window_closable());
    let metrics = runtime.renderer.theme().text_context_menu_metrics();
    assert!(runtime.renderer.show_context_menu(
        nodes,
        None,
        waterui_core::layout::Point::new(40.0, 40.0),
        metrics,
        &env,
        true,
    ));
    assert!(
        !runtime.renderer.has_render_tree(),
        "the menu must be open before the first build"
    );

    assert!(render_window(&mut runtime, &env, &mut || false));
    assert!(runtime.renderer.has_render_tree());
    assert!(
        runtime
            .renderer
            .context_menu_presentation_frames()
            .is_some(),
        "the drawn presentation mounts on the build frame"
    );
    let built_targets = runtime.renderer.registries().pointer_targets.len();
    let built_occluders = runtime.renderer.registries().gesture_occluders.len();

    // A later refresh records the hosts through the ordinary path; the
    // build frame registered exactly what it does.
    runtime.renderer.root_cell().mark_layout();
    runtime.request_refresh();
    assert!(render_window(&mut runtime, &env, &mut || false));
    assert!(
        runtime
            .renderer
            .context_menu_presentation_frames()
            .is_some()
    );
    assert_eq!(
        runtime.renderer.registries().pointer_targets.len(),
        built_targets,
        "the build frame must register the presentation's targets once"
    );
    assert_eq!(
        runtime.renderer.registries().gesture_occluders.len(),
        built_occluders,
        "the build frame must register the presentation's occluders once"
    );
}

fn test_runtime_window() -> RuntimeWindow<HeadlessPlatformWindow> {
    sized_test_runtime_window(16, 16)
}

fn sized_test_runtime_window(width: u32, height: u32) -> RuntimeWindow<HeadlessPlatformWindow> {
    let window = Window::new("", binding(WindowState::Normal), || ());
    let mut platform =
        HeadlessPlatformWindow::new_for_tests(width, height, wgpu::TextureFormat::Rgba8Unorm);
    platform.apply_properties(&window);
    let renderer = HydrolysisRenderer::with_engine(
        Rc::new(MinimalTestTheme::default()),
        SessionTextEngine::system(FontFamilyResolution::Strict),
    );
    RuntimeWindow::new(
        window,
        platform,
        renderer,
        RenderDiagnosticsConfig {
            enabled: false,
            interval: Duration::from_secs(1),
            slow_frame_threshold_override: None,
        },
    )
}

struct RecoveringSurface {
    inner: OffscreenSurface,
    first_error: Option<SurfaceError>,
    acquire_count: usize,
    resize_count: usize,
}

impl RecoveringSurface {
    fn new(first_error: SurfaceError) -> Self {
        Self {
            inner: pollster::block_on(OffscreenSurface::new_for_tests(
                16,
                16,
                wgpu::TextureFormat::Rgba8Unorm,
            )),
            first_error: Some(first_error),
            acquire_count: 0,
            resize_count: 0,
        }
    }
}

impl crate::platform::PresentationSurface for RecoveringSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        self.inner.adapter()
    }

    fn device(&self) -> &wgpu::Device {
        self.inner.device()
    }

    fn queue(&self) -> &wgpu::Queue {
        self.inner.queue()
    }

    fn device_loss(&self) -> &crate::platform::DeviceLoss {
        self.inner.device_loss()
    }

    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.resize_count += 1;
        self.inner.resize(width, height);
    }
}

impl SurfaceProvider for RecoveringSurface {
    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        self.acquire_count += 1;
        self.first_error
            .take()
            .map_or_else(|| Ok(self.inner.acquire()), Err)
    }

    fn present(&mut self, frame: SurfaceFrame) {
        self.inner.present(frame);
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.inner.format()
    }
}

/// Regression test for water-rs/hydrolysis#228: an `on_change` handler fed by
/// `debounce` must still fire once the quiet period elapses. `OnChange`
/// retains only the guard `watch()` returns, so the `Debounce` value drops
/// when `body` evaluates — and with it the cell holding the upstream
/// subscription, unless the guard keeps it alive. The timer itself has to run
/// on the runner's local executor, the queue every `pump_*` drains the way
/// the winit loop drains `PollLocalTasks`.
#[test]
fn debounced_on_change_fires_after_the_quiet_period() {
    use nami::SignalExt as _;
    use waterui::text;
    use waterui_core::handler::AnyViewBuilder;

    // `pumped_test_environment` leaves the local-executor slot open so the
    // runtime installs its draining `HeadlessMainThreadExecutor` — a parked
    // runnable would make the timer invisible regardless of the bug.
    let env = crate::renderer::tests::pumped_test_environment();
    let source = binding(0i32);
    let fired = Rc::new(RefCell::new(Vec::<i32>::new()));
    let builder = {
        let source = source.clone();
        let fired = Rc::clone(&fired);
        AnyViewBuilder::<AnyView>::new(move || {
            let debounced = source.debounce(Duration::from_millis(20));
            AnyView::new(text!("x").on_change(&debounced, {
                let fired = Rc::clone(&fired);
                move |value: i32| {
                    fired.borrow_mut().push(value);
                }
            }))
        })
    };
    let mut runtime =
        crate::HeadlessRuntime::new_for_tests(env, builder, 200, 120, MinimalTestTheme::default());
    let executor = super::executor::HeadlessMainThreadExecutor::thread_shared();

    // The mount pump builds the view, installs the watch, and drops the
    // `Debounce` value — where a buggy subscription died with it.
    let _ = runtime.pump_snapshot();
    source.set(1);
    // The first drain polls the spawned task once, arming the real
    // `async_io::Timer`; its reactor-thread wake re-queues the runnable.
    let _ = runtime.pump_offscreen();
    // Wait on the wake edge itself: the executor signals when the reactor
    // thread re-queues the runnable, however long that takes a loaded
    // runner. The deadline fails the test only when the timer never fires.
    let deadline = Instant::now() + Duration::from_secs(10);
    while fired.borrow().is_empty() {
        assert!(
            executor.wait_queued(deadline.saturating_duration_since(Instant::now())),
            "the debounce timer never re-queued its runnable on the local executor"
        );
        let _ = runtime.pump_offscreen();
    }

    assert_eq!(
        fired.borrow().as_slice(),
        &[1],
        "debounce never re-emitted: the upstream watch died with the combinator"
    );
}

/// A `Binding::set` outside a frame must schedule one
/// (water-rs/waterui#2286): the host wake installed at mount fires on the
/// none→some pending-request edge — once per burst — and the pump's drain
/// re-arms it. The counter is supplied by the platform before mount, not
/// installed over the runtime's wake, and the binding belongs to content.
#[test]
fn a_binding_set_outside_a_frame_fires_the_installed_wake_once() {
    let value = Binding::i32(0);
    let content = value.clone();
    let mut runtime =
        runtime_window_for(Window::new("", binding(WindowState::Normal), move || {
            waterui::text!("{content}")
        }));
    let env = crate::renderer::tests::test_environment();
    let mut now = Instant::now();
    assert!(drive_until_idle(&mut runtime, &env, &mut now, 60) < 60);
    let fires = Rc::clone(&runtime.platform.frame_wake_count);
    let initial = fires.get();

    value.set(1);
    assert_eq!(
        fires.get(),
        initial + 1,
        "the content set fires the platform wake"
    );
    assert!(runtime.renderer.has_pending_frame_request());
    assert!(
        runtime.renderer.root_is_dirty(),
        "the content watcher marks its NodeCell"
    );

    value.set(2);
    assert_eq!(
        fires.get(),
        initial + 1,
        "a set while the request is pending fires nothing"
    );

    assert!(drive_until_idle(&mut runtime, &env, &mut now, 60) < 60);
    assert!(!runtime.renderer.has_pending_frame_request());
    let drained = fires.get();
    value.set(3);
    assert_eq!(fires.get(), drained + 1);
}

#[test]
fn frame_transaction_gates_wakes_and_renders_the_restore_frame() {
    use super::window::{FrameDemand, FrameTransaction};

    let transaction = Rc::new(FrameTransaction::default());
    let mut platform =
        HeadlessPlatformWindow::new_for_tests(16, 16, wgpu::TextureFormat::Rgba8Unorm);
    platform.frame_transaction = Some(Rc::clone(&transaction));
    let mut runtime = runtime_window_with_platform(
        Window::new("", binding(WindowState::Normal), || ()),
        platform,
    );
    let env = crate::renderer::tests::test_environment();
    let now = Instant::now();
    transaction.sync_occlusion(runtime.platform.is_occluded());
    let _ = pump_frame_transaction(&mut runtime, &transaction, &env, now);
    assert!(!pump_frame_transaction(
        &mut runtime,
        &transaction,
        &env,
        now
    ));
    let fires = Rc::clone(&runtime.platform.frame_wake_count);
    let initial = fires.get();

    transaction.begin();
    transaction.sync_occlusion(false);
    runtime.renderer.root_cell().mark_layout();
    assert_eq!(
        fires.get(),
        initial,
        "visibility sync must not reopen a transaction"
    );
    assert!(
        transaction.finish(
            false,
            FrameDemand {
                mode: runtime.mode,
                redraw_pending: runtime.platform.take_redraw_request(),
                signals_pending: runtime.renderer.has_pending_frame_request(),
            }
        ),
        "a request raised inside the transaction must schedule continuation"
    );
    let _ = pump_frame_transaction(&mut runtime, &transaction, &env, now);
    assert!(!pump_frame_transaction(
        &mut runtime,
        &transaction,
        &env,
        now
    ));

    runtime.renderer.root_cell().mark_layout();
    assert_eq!(
        fires.get(),
        initial + 1,
        "closing the transaction reopens the gate"
    );
    let _ = pump_frame_transaction(&mut runtime, &transaction, &env, now);
    assert!(!pump_frame_transaction(
        &mut runtime,
        &transaction,
        &env,
        now
    ));

    runtime.platform.set_occluded(true);
    transaction.sync_occlusion(runtime.platform.is_occluded());
    runtime.sync_occlusion();
    let before_hide = fires.get();
    let presented = runtime.presented_frames;
    runtime.renderer.root_cell().mark_layout();
    assert_eq!(fires.get(), before_hide);
    assert!(!pump_frame_transaction(
        &mut runtime,
        &transaction,
        &env,
        now
    ));
    assert!(runtime.renderer.has_pending_frame_request());
    assert_eq!(runtime.presented_frames, presented);

    runtime.platform.set_occluded(false);
    transaction.sync_occlusion(runtime.platform.is_occluded());
    runtime.sync_occlusion_and_post_restore();
    assert!(
        runtime.platform.take_redraw_request(),
        "restore must post its frame"
    );
    assert!(runtime.mode.is_pending());
    let _ = pump_frame_transaction(&mut runtime, &transaction, &env, now);
    assert!(!pump_frame_transaction(
        &mut runtime,
        &transaction,
        &env,
        now
    ));
    assert_eq!(runtime.presented_frames, presented + 1);
    assert!(!runtime.renderer.has_pending_frame_request());
}

/// The cross-thread redraw handle's UI-thread post reads the gate as it
/// stands when the post runs: one drained inside a transaction must not
/// post a second frame into the one already running (water-rs/waterui#2287).
#[test]
fn frame_transaction_gates_a_ui_thread_wake() {
    use super::window::{FrameDemand, FrameMode, FrameTransaction};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let transaction = FrameTransaction::default();
    let posts = Arc::new(AtomicUsize::new(0));
    let wake = transaction.ui_thread_wake({
        let posts = Arc::clone(&posts);
        move || {
            posts.fetch_add(1, Ordering::Relaxed);
        }
    });
    let posted = || posts.load(Ordering::Relaxed);

    transaction.sync_occlusion(false);
    wake();
    assert_eq!(posted(), 1, "an idle visible window posts the frame");

    transaction.begin();
    wake();
    assert_eq!(posted(), 1, "a post inside a transaction joins it");
    let _ = transaction.finish(
        false,
        FrameDemand {
            mode: FrameMode::Idle,
            redraw_pending: true,
            signals_pending: false,
        },
    );
    wake();
    assert_eq!(posted(), 2, "closing the transaction reopens the gate");

    transaction.sync_occlusion(true);
    wake();
    assert_eq!(posted(), 2, "an occluded window posts nothing");
}

#[test]
fn frame_transaction_consumes_a_focused_fields_caret_redraw() {
    use super::window::FrameTransaction;
    use waterui_controls::text_field::TextField;
    use waterui_core::Str;

    let value = binding(Str::from("Caret"));
    let mut runtime = runtime_window_sized(
        Window::new("", binding(WindowState::Normal), move || {
            TextField::new("Value", &value).hide_label()
        }),
        240,
        120,
    );
    let transaction = FrameTransaction::default();
    let env = crate::renderer::tests::test_environment();
    let now = Instant::now();
    let _ = pump_frame_transaction(&mut runtime, &transaction, &env, now);
    assert!(runtime.renderer.set_focused_text_input(Some(0)));
    // Settle focus animations before exercising the redraw-only caret path.
    for second in 0..3 {
        let _ = pump_frame_transaction(
            &mut runtime,
            &transaction,
            &env,
            now + Duration::from_secs(second),
        );
    }
    assert!(!runtime.mode.is_pending());
    let presented = runtime.presented_frames;
    let tick = now + Duration::from_secs(3);
    let _ = pump_frame_transaction(&mut runtime, &transaction, &env, tick);
    assert_eq!(
        runtime.presented_frames,
        presented + 1,
        "the caret tick must render"
    );
    assert!(
        !runtime.renderer.has_pending_frame_request(),
        "the caret redraw must not latch the pump awake"
    );
    assert!(
        !pump_frame_transaction(&mut runtime, &transaction, &env, tick),
        "without another caret tick the pump must idle"
    );
    assert_eq!(runtime.presented_frames, presented + 1);
}

fn pump_frame_transaction(
    runtime: &mut RuntimeWindow<HeadlessPlatformWindow>,
    transaction: &super::window::FrameTransaction,
    env: &Environment,
    now: Instant,
) -> bool {
    use super::window::{FrameDemand, FrameTransaction};

    transaction.begin();
    let _ = advance_runtime(runtime, env, now);
    let surface_attached = !runtime.platform.is_occluded();
    if FrameTransaction::take_render_request(runtime, surface_attached) {
        assert!(render_window(runtime, env, &mut || false));
    }
    transaction.finish(
        runtime.platform.is_occluded(),
        FrameDemand {
            mode: runtime.mode,
            redraw_pending: runtime.platform.take_redraw_request(),
            signals_pending: runtime.renderer.has_pending_frame_request(),
        },
    )
}
