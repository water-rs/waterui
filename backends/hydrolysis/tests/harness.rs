//! Renderer presentation tests for the `waterui-testing` harness surface:
//! snapshot capture, styled mounts, `mount_app`, the perf entry points, and
//! pointer/hover/drag/magnify routing on the rendered runtime.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); every case names its origin file and asserts what
//! it asserted there, mounted under `Material3::defaults()` on the rendered
//! runtime.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hydrolysis_m3::Material3;
use waterui::Computed;
use waterui::Signal as _;
use waterui::SignalExt as _;
use waterui::ViewExt as _;
use waterui::app::App;
use waterui::color::WorkingColor;
use waterui::component::{text, vstack};
use waterui::graphics::color::Srgb;
use waterui::layout::scroll::ScrollView;
use waterui::theme;
use waterui_canvas::Canvas;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::{Point, Rect, Size};
use waterui_core::{AnyView, Environment};
use waterui_testing::{PerfConfig, Role, TestHost, ui};

/// Installs the custom foreground token the origin tests layered over the
/// theme package: the harness no longer takes a theme-installer closure, so
/// the override is a view-scoped plugin — applied after the style's tokens,
/// on the mounted view's environment scope, which is where the origin's
/// builder closure wrote it.
struct ForegroundSlot(WorkingColor);

impl waterui::Plugin for ForegroundSlot {
    fn install(self, env: &mut Environment) {
        theme::install_color_signal::<theme::color::Foreground>(env, Computed::constant(self.0));
    }
}

const fn white_foreground() -> ForegroundSlot {
    ForegroundSlot(WorkingColor::new([1.0, 1.0, 1.0, 1.0]))
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn smoke_snapshot_size_matches_target() {
    let host = TestHost::new(Environment::new(), 64, 48, Material3::defaults());
    let snapshot = host.render(());
    assert_eq!(snapshot.width, 64);
    assert_eq!(snapshot.height, 48);
    assert_eq!(snapshot.rgba8.len(), 64 * 48 * 4);
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn smoke_theme_foreground_slot_snapshot_preserves_semantic_labels() {
    let mut app = ui()
        .viewport(240, 120)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            vstack((text("Theme slot").body(), text("Theme slot").body()))
                .background(Srgb::BLACK)
                .install(white_foreground())
        });
    assert_eq!(
        app.query()
            .role(Role::LABEL)
            .label("Theme slot")
            .all()
            .len(),
        2,
        "theme slot text should stay queryable under custom theme environment"
    );
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 240);
    assert_eq!(snapshot.height, 120);
    assert_eq!(snapshot.rgba8.len(), 240 * 120 * 4);
}

/// A view that draws widget chrome mounts under the default theme: the scroll
/// view's scrollbar and the button both read a widget theme from the
/// environment, and `ui()` used to install none (#290).
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn default_theme_renders_hydrolysis_widgets() {
    let mut app = ui()
        .viewport(240, 160)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            ScrollView::vertical(vstack((
                waterui::component::button("Submit"),
                text("Scrolled content").body(),
            )))
        });
    assert_eq!(app.query().role(Role::BUTTON).all().len(), 1);
    let snapshot = app.snapshot();
    assert_eq!(snapshot.rgba8.len(), 240 * 160 * 4);
}

/// `mount_app` runs the application path: the app's own environment is
/// mounted verbatim, and the configured scale factor scales the captured
/// snapshot. The origin's `install_default_theme(&mut env)` on the app
/// environment has no equivalent here — the style the builder carries now
/// installs the theme.
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn mount_app_hosts_main_window_with_app_environment() {
    let app = App::new(|| text("Mounted app").body(), Environment::new());
    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(200, 100)
        .scale_factor(2.0)
        .mount_app(app);
    app.query().label("Mounted app").assert_exists();
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 400);
    assert_eq!(snapshot.height, 200);
}

/// A `max_width` derived from the mounted window's `frame` must measure the
/// text against the width the mounted viewport produces — the wrap the
/// window runner gives the same tree (water-rs/hydrolysis#130). Mounting the
/// window's content inside a synthetic default window left the app's
/// `Window::frame` binding at its initial value, so the frame-derived cap
/// resolved stale: on a 320pt viewport the initial 800pt frame produced a
/// 200pt cap where the window runner's 80pt cap wraps the text to several
/// lines, and the semantic runtime rendered a single line clipped at the
/// cap's edge.
#[test]
fn max_width_derived_from_the_mounted_window_frame_wraps_text() {
    use waterui::layout::frame::Frame;
    use waterui::window::{Window, WindowState};

    const LONG: &str = "the quick brown fox jumps over the lazy dog, again and \
        again and again, until the sentence refuses to fit on one line";

    let frame = waterui::binding(Rect::new(Point::zero(), Size::new(800.0, 600.0)));
    let cap = frame.map(|f: Rect| f.size().width / 4.0);
    let mut window = Window::new("app", waterui::binding(WindowState::Normal), move || {
        Frame::new(text(LONG).body()).max_width(cap.clone())
    });
    window.frame = frame;
    let app = App::new_with_windows([window], Environment::new());
    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(320, 900)
        .mount_app(app);

    let bounds = app.query().label(LONG).single().bounds();
    assert!(
        bounds.width() < 100.0,
        "the measured width must honour the frame-derived 80pt cap, not the stale 200pt one: {bounds:?}"
    );
    assert!(
        bounds.height() > 130.0,
        "the text must wrap to the lines an 80pt cap produces, not render one clipped line: {bounds:?}"
    );
}

/// `mount_app` must mount the application's own `Window` — the mount the
/// window runner performs — so the runtime writes `Window::frame` from the
/// viewport onto the app's binding at mount. Mounting the window's content
/// inside a synthetic default window leaves the app's `frame` at whatever
/// the app seeded regardless of the viewport, orphaning every signal derived
/// from it (water-rs/hydrolysis#128).
#[test]
fn mount_app_drives_the_app_window_frame_from_the_viewport() {
    use waterui::window::{Window, WindowState};

    let frame = waterui::binding(Rect::new(Point::zero(), Size::new(800.0, 600.0)));
    let mut window = Window::new("app", waterui::binding(WindowState::Normal), || {
        text("app content").body()
    });
    window.frame = frame.clone();
    let app = App::new_with_windows([window], Environment::new());
    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(320, 240)
        .mount_app(app);
    assert_eq!(
        frame.snapshot(),
        Rect::new(Point::zero(), Size::new(320.0, 240.0)),
        "the app's Window::frame must carry the mounted viewport"
    );
    let bounds = app.query().role(Role::LABEL).single().bounds();
    approx::assert_relative_eq!(bounds.width(), 320.0);
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn ui_test_environment_builder_preserves_custom_theme() {
    let mut app = ui()
        .viewport(240, 120)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            vstack((text("Mounted theme").body(), text("Mounted theme").body()))
                .background(Srgb::BLACK)
                .install(white_foreground())
        });
    assert_eq!(
        app.query()
            .role(Role::LABEL)
            .label("Mounted theme")
            .all()
            .len(),
        2,
        "UiBuilder environment builder should keep mounted text semantics intact"
    );
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 240);
    assert_eq!(snapshot.height, 120);
    assert_eq!(snapshot.rgba8.len(), 240 * 120 * 4);
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn smoke_text_color_snapshot_preserves_semantic_labels() {
    let mut app = ui()
        .viewport(240, 120)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            vstack((
                text("Explicit color").body().color(Srgb::WHITE),
                text("Explicit color").body().color(Srgb::WHITE),
            ))
            .background(Srgb::BLACK)
        });
    assert_eq!(
        app.query()
            .role(Role::LABEL)
            .label("Explicit color")
            .all()
            .len(),
        2,
        "explicit text color should not break semantic text exposure"
    );
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn smoke_text_snapshot_preserves_semantic_labels() {
    let mut app = ui()
        .viewport(240, 120)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            vstack((
                text("Focused datum").body().foreground(Srgb::WHITE),
                text("Selected datum").body().foreground(Srgb::WHITE),
            ))
            .background(Srgb::BLACK)
        });
    app.query()
        .role(Role::LABEL)
        .label("Focused datum")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Selected datum")
        .assert_exists();
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn ui_test_snapshot_renders_text_after_canvas() {
    let mut app = ui()
        .viewport(320, 320)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            vstack((
                Canvas::new(|ctx| {
                    ctx.set_fill_style(Srgb::new(0.0, 0.85, 0.65));
                    ctx.fill_rect(Rect::new(Point::new(0.0, 0.0), Size::new(240.0, 180.0)));
                })
                .size(240.0, 180.0)
                .a11y_role(waterui::accessibility::AccessibilityRole::Image)
                .a11y_label("Canvas layer"),
                text("W")
                    .size(48.0)
                    .color(Srgb::WHITE)
                    .body()
                    .padding_with(6.0)
                    .a11y_label("Letter W"),
            ))
            .spacing(6.0)
            .background(Srgb::BLACK)
        });
    app.query()
        .role(Role::IMAGE)
        .label("Canvas layer")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Letter W")
        .assert_exists();
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 320);
    assert_eq!(snapshot.height, 320);
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn smoke_canvas_snapshot_preserves_accessibility_metadata() {
    let mut app = ui()
        .viewport(96, 72)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            Canvas::new(|ctx| {
                ctx.set_fill_style(Srgb::new(1.0, 0.0, 0.0));
                ctx.fill_rect(Rect::new(Point::new(8.0, 8.0), Size::new(40.0, 24.0)));
            })
            .a11y_role(waterui::accessibility::AccessibilityRole::Image)
            .a11y_label("Canvas smoke")
        });
    app.query()
        .role(Role::IMAGE)
        .label("Canvas smoke")
        .assert_exists();
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 96);
    assert_eq!(snapshot.height, 72);
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn themed_builder_exposes_offscreen_perf_closure_api() {
    let report = ui()
        .viewport(96, 72)
        .theme(Material3::defaults())
        .perf_config(PerfConfig {
            warmups: 1,
            samples: 3,
            repetitions: 1,
        })
        .perf_with(
            || text("Measured").body(),
            |perf| {
                perf.measure("steady", |run| {
                    let _ = run
                        .app()
                        .query()
                        .role(Role::LABEL)
                        .label("Measured")
                        .single();
                });
            },
        );

    let measurements = report.measurements();
    assert_eq!(measurements.len(), 1);
    assert_eq!(measurements[0].name, "steady");
    let stats = measurements[0].stats();
    assert_eq!(stats.samples, 3);
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn themed_builder_default_perf_requests_redraw() {
    let report = ui()
        .viewport(96, 72)
        .theme(Material3::defaults())
        .perf_config(PerfConfig {
            warmups: 1,
            samples: 3,
            repetitions: 1,
        })
        .perf(|| text("Redraw measured").body());

    let measurements = report.measurements();
    assert_eq!(measurements.len(), 1);
    assert_eq!(measurements[0].name, "steady-redraw");
    let stats = measurements[0].stats();
    assert_eq!(stats.samples, 3);
    assert_eq!(stats.rebuilt_frames, 0);
    assert!(
        stats.phases.render.p95 > Duration::ZERO,
        "default perf should measure real redraw frames"
    );
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn ui_test_hover_drag_and_magnify_update_semantic_bounds() {
    use waterui::gesture::{
        DragEvent, DragGesture, GestureObserver, MagnificationEvent, MagnificationGesture,
    };
    use waterui::prelude::text;
    use waterui::{Binding, SignalExt as _, ViewExt as _, state};
    use waterui_core::extract::Use;

    #[state]
    #[derive(Clone)]
    struct DragOffset(Binding<f32>);

    #[state]
    #[derive(Clone)]
    struct ZoomScale(Binding<f32>);

    #[state]
    #[derive(Clone)]
    struct HoverState(Binding<bool>);

    let offset = Binding::f32(0.0);
    let scale = Binding::f32(1.0);
    let hovered = Binding::bool(false);

    let mut app = ui()
        .viewport(160, 160)
        .theme(Material3::defaults())
        .mount_offscreen({
            let offset = offset.clone();
            let scale = scale.clone();
            let hovered = hovered.clone();
            move || {
                let drag_offset_state = DragOffset(offset.clone());
                let zoom_scale_state = ZoomScale(scale.clone());
                let hover_state = HoverState(hovered.clone());
                let hovered_opacity = hovered
                    .clone()
                    .map(|hovered| if hovered { 1.0 } else { 0.68 });
                let surface = text("interactive canvas")
                    .padding()
                    .size(120.0, 120.0)
                    .offset(offset.clone(), 0.0)
                    .scale(scale.clone(), scale.clone())
                    .opacity(hovered_opacity);
                surface
                    .gesture_observer(GestureObserver::new(
                        DragGesture::new(0.0),
                        |DragOffset(offset): DragOffset, drag: Use<DragEvent>| {
                            offset.set(drag.translation.x);
                        },
                    ))
                    .state(&drag_offset_state)
                    .gesture_observer(GestureObserver::new(
                        MagnificationGesture::new(1.0),
                        |ZoomScale(scale): ZoomScale, magnification: Use<MagnificationEvent>| {
                            scale.set(magnification.scale);
                        },
                    ))
                    .state(&zoom_scale_state)
                    .on_hover_enter(|HoverState(hovered): HoverState| hovered.set(true))
                    .on_hover_exit(|HoverState(hovered): HoverState| hovered.set(false))
                    .state(&hover_state)
            }
        });

    let initial_bounds = app.query().label("interactive canvas").single().bounds();
    assert!(initial_bounds.width() > 0.0 && initial_bounds.height() > 0.0);

    app.query().label("interactive canvas").hover();
    assert!(
        hovered.snapshot(),
        "hover should update the tracked binding"
    );

    let center_before_drag = app.query().label("interactive canvas").single().center();
    app.magnify_at(center_before_drag.0, center_before_drag.1, 1.2);
    assert!(
        (scale.snapshot() - 1.2).abs() < 0.001,
        "magnify should update the tracked scale binding"
    );

    app.query().label("interactive canvas").drag_by(24.0, 0.0);
    assert!(
        (offset.snapshot() - 24.0).abs() < 0.001,
        "drag should update the tracked offset binding"
    );

    let center_after_drag = app.query().label("interactive canvas").single().center();
    app.magnify_at(center_after_drag.0, center_after_drag.1, 1.4);
    assert!(
        (scale.snapshot() - 1.4).abs() < 0.001,
        "second magnify should update the tracked scale binding"
    );

    let updated_bounds = app.query().label("interactive canvas").single().bounds();
    assert!(
        updated_bounds.width() > initial_bounds.width(),
        "magnify should grow the accessible bounds width"
    );
    assert!(
        updated_bounds.height() > initial_bounds.height(),
        "magnify should grow the accessible bounds height"
    );
    assert!(
        updated_bounds.x() > initial_bounds.x(),
        "drag should move the accessible bounds horizontally"
    );
}

// Origin: waterui `testing/src/tests.rs`.
#[test]
fn ui_test_drains_local_tasks_through_headless_runtime() {
    use waterui::task::spawn_local;
    use waterui::{Binding, ViewExt as _};

    let status = Binding::container(String::from("idle"));
    let status_for_view = status.clone();

    let mut app = ui().theme(Material3::defaults()).mount_offscreen(move || {
        waterui::text!("{status_for_view}")
            .on_appear(|status: waterui::State<Binding<String>>| {
                spawn_local(async move {
                    status.set(String::from("ready"));
                })
                .detach();
            })
            .state(&status_for_view)
    });

    let deadline = std::time::Instant::now() + Duration::from_millis(200);
    while status.snapshot() != "ready" && std::time::Instant::now() < deadline {
        let _ = app.snapshot();
    }
    assert_eq!(
        status.snapshot().as_str(),
        "ready",
        "expected headless runtime to drain spawn_local task and update the binding"
    );
}

// ============================================================================
// Rendered-runtime behavior the semantic pipeline cannot express
// ============================================================================

use waterui::graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui::graphics::{GpuContent, GpuContentView};

/// A `GpuContent` that draws nothing until its `setup` has run — the
/// readiness flag distinguishes "the engine ran setup" from "a render slipped
/// in first", the property the async-setup regression exercised.
#[derive(Debug)]
struct DeferredClearRenderer {
    color: waterui_graphics::wgpu::Color,
    ready: Arc<AtomicBool>,
}

impl GpuContent for DeferredClearRenderer {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {
        self.ready.store(true, Ordering::Relaxed);
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        if !self.ready.load(Ordering::Relaxed) {
            return;
        }
        let mut encoder = frame.device.create_command_encoder(
            &waterui_graphics::wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_deferred_gpu_surface_encoder"),
            },
        );
        {
            let _pass = encoder.begin_render_pass(&waterui_graphics::wgpu::RenderPassDescriptor {
                label: Some("hydrolysis_deferred_gpu_surface_pass"),
                color_attachments: &[Some(waterui_graphics::wgpu::RenderPassColorAttachment {
                    view: frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: waterui_graphics::wgpu::Operations {
                        load: waterui_graphics::wgpu::LoadOp::Clear(self.color),
                        store: waterui_graphics::wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        frame.queue.submit([encoder.finish()]);
    }
}

/// A `GpuContentView` must reach the captured frame only after its `setup`
/// ran. Capturing the very first pumped frame photographs the surface before
/// any GPU content exists — the regression that made every GPU preview in the
/// book render as a flat background.
///
/// Origin: waterui `testing/src/tests.rs`; the style parameter replaced the
/// origin's `install_theme`/`install_m3` environment writes, which is where
/// the same install now happens.
#[test]
fn headless_capture_waits_for_gpu_setup() {
    let ready = Arc::new(AtomicBool::new(false));
    let ready_for_view = Arc::clone(&ready);
    let content = AnyViewBuilder::new(move || {
        AnyView::new(GpuContentView::new(DeferredClearRenderer {
            // Black is gamut-neutral: it survives the engine's linear-P3
            // presentation unchanged, where a saturated primary would
            // shift under the P3 → sRGB gamut map.
            color: waterui_graphics::wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            ready: Arc::clone(&ready_for_view),
        }))
    });

    let env = Environment::new();
    let mut runtime =
        hydrolysis::HeadlessRuntime::new_for_tests(env, content, 64, 64, Material3::defaults());

    // Pump until the frame settles, exactly as the preview runtime does.
    let mut settled = false;
    for _ in 0..64 {
        if !runtime.pump_at(false, std::time::Instant::now()).rebuilt {
            settled = true;
            break;
        }
    }
    assert!(settled, "frame never settled");
    assert!(
        ready.load(Ordering::Relaxed),
        "GpuContent::setup must have run before the frame was presented"
    );

    let snapshot = runtime
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("capture must produce a snapshot");
    let center = ((snapshot.width as usize / 2)
        + (snapshot.height as usize / 2) * snapshot.width as usize)
        * 4;
    assert_eq!(
        &snapshot.rgba8[center..center + 3],
        &[0, 0, 0],
        "the GpuContentView content must be present in the captured frame"
    );
}

/// A capture has to photograph the frame the pump just presented, not the
/// composite from the frame before it: the render encodes into the surface
/// texture and the readback copies that same texture on the same queue.
/// `screenshot` and `advance(screenshot: true)` both end in `pump_at(true)`;
/// after a visible state change the first capture must already show the new
/// state: the tree update carries it and the image is byte-identical to a
/// second capture taken once the frame has fully settled.
///
/// Origin: water-rs/hydrolysis#324 — `screenshot` returned the previous
/// composite, so a capture taken right after a state change showed the old
/// scene.
#[test]
fn capture_reads_the_current_frame() {
    let label = waterui::reactive::binding(waterui::Str::from("before"));
    let probe = label.clone();
    let content =
        AnyViewBuilder::new(move || AnyView::new(waterui::text::Text::computed(label.clone())));
    let mut runtime = hydrolysis::HeadlessRuntime::new_for_tests(
        Environment::new(),
        content,
        390,
        200,
        Material3::defaults(),
    );
    // Settle the mount the way a session's initial advance does.
    for _ in 0..64 {
        if !runtime.pump_at(false, std::time::Instant::now()).rebuilt {
            break;
        }
    }
    let before = runtime
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("capture must produce a snapshot");

    probe.set(waterui::Str::from("after"));

    let captured_outcome = runtime.pump_at(true, std::time::Instant::now());
    assert!(
        captured_outcome.tree_update.is_some_and(|update| update
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("after"))),
        "the flushed tree must carry the new label"
    );
    let captured = captured_outcome
        .snapshot
        .expect("capture must produce a snapshot");
    assert_ne!(
        captured.rgba8, before.rgba8,
        "the capture must show the new label, not the previous composite"
    );
    let settled = runtime
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("capture must produce a snapshot");
    assert_eq!(
        captured.rgba8, settled.rgba8,
        "a settled capture is deterministic: the first capture must equal it"
    );
}

/// An app that never comes to rest still answers queries promptly.
///
/// An indeterminate indicator keeps the runtime unsettled forever, so a read
/// that waited for quiescence would spend its whole pump budget on every
/// query and still find the app busy. Reading waits on *unapplied* work
/// instead, which is a state the app does reach between changes.
///
/// Origin: waterui `testing/src/tests.rs`. The origin read the driver through
/// the mounted session's crate-private fields; the session's `runtime` is
/// crate-private on the split harness, so the same probes run on
/// `HeadlessRuntime` itself — the same rendered runtime the session wraps.
#[test]
fn a_perpetually_animating_app_is_never_settled_yet_stays_current() {
    let label = waterui::reactive::binding(waterui::Str::from("before"));
    let probe = label.clone();
    let content = AnyViewBuilder::new(move || {
        AnyView::new(vstack((
            waterui::component::progress::loading().label("Loading"),
            waterui::text::Text::computed(label.clone()),
        )))
    });
    let mut runtime = hydrolysis::HeadlessRuntime::new_for_tests(
        Environment::new(),
        content,
        390,
        844,
        Material3::defaults(),
    );

    // Mount: pump until the first tree update lands — the same frame a mounted
    // session's initial pump produces.
    let mut mounted = false;
    for _ in 0..64 {
        if runtime
            .pump_at(false, std::time::Instant::now())
            .tree_update
            .is_some()
        {
            mounted = true;
            break;
        }
    }
    assert!(mounted, "initial pumps produced no accessibility tree");

    assert!(
        !runtime.is_settled(),
        "an indeterminate indicator keeps the runtime busy for as long as it is on screen"
    );
    assert!(
        !runtime.has_pending_semantic_update(),
        "busy is not the same as stale: with nothing unapplied the tree is current"
    );

    probe.set(waterui::Str::from("after"));
    assert!(
        runtime.has_pending_semantic_update(),
        "a signal change leaves an update the last flush did not apply"
    );

    // A pump flushes the pending update; the produced tree carries the new
    // label — what a query on the session would have answered.
    let mut saw_after = false;
    for _ in 0..64 {
        let outcome = runtime.pump_at(false, std::time::Instant::now());
        if let Some(update) = outcome.tree_update
            && update
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some("after"))
        {
            saw_after = true;
            break;
        }
    }
    assert!(saw_after, "the flushed tree must carry the new label");
    assert!(
        !runtime.has_pending_semantic_update(),
        "reading the tree must have applied the update, not merely waited for it"
    );
}
