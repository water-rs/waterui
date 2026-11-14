//! Snippets from `.claude/skills/waterui/references/testing.md`, in file order.
//! Transcription conventions are documented in the crate README.
//!
//! Everything except the `#[preview]` block needs the `waterui-testing`
//! dev-dependency, and the rendered-form blocks need `hydrolysis-m3`, so they sit
//! behind the non-default `compile-gate-tests` feature. **Those transcriptions
//! must never execute**: the query / interaction / waiting listings address
//! elements that do not exist by design, so running them would panic for
//! reasons that say nothing about whether the documented API exists. CI
//! compiles them with `cargo check -p skill_snippets --all-targets --features
//! compile-gate-tests` and never runs them.

use waterui::prelude::*;

/// Glue: the state testing.md's preview function constructs.
#[derive(Clone)]
pub struct DemoState;

impl DemoState {
    fn new() -> Self {
        Self
    }
}

fn content(_state: DemoState) -> impl View {
    vstack((text("demo"), button("Login")))
}

// ---------------------------------------------------------------------------
// testing.md § "## `#[preview]`" — rust block 1/15
// ---------------------------------------------------------------------------
use waterui::preview;

#[preview]
pub fn demo() -> impl View {
    content(DemoState::new())
}

#[cfg(all(test, feature = "compile-gate-tests"))]
pub mod gated {
    use super::{DemoState, content};
    use core::time::Duration;
    use waterui::prelude::*;

    /// Glue: the view testing.md's mounting-form example names.
    fn login_view() -> impl View {
        vstack((button("Login"), text("Welcome")))
    }

    /// Glue: the view testing.md's bench examples name.
    fn dashboard() -> impl View {
        vstack((text("dashboard"), Divider))
    }

    /// Glue: the view testing.md's offscreen example names (`demo`, the
    /// `#[preview]` function above — repeated here so the attribute can name a
    /// path in scope).
    fn demo() -> impl View {
        content(DemoState::new())
    }

    // -----------------------------------------------------------------------
    // testing.md § "## `#[waterui::test]`" — rust block 2/15 (mounting form)
    //
    // Semantic: no `theme =`, so the macro mounts the GPU-free semantic
    // runtime.
    // -----------------------------------------------------------------------
    use waterui_testing::{Role, SemanticApp};

    #[waterui::test(login_view, viewport = (360, 320))]
    fn login_flow(app: &mut SemanticApp) {
        app.query().role(Role::BUTTON).label("Login").tap();
        app.query().label("Welcome").assert_exists();
    }

    // -----------------------------------------------------------------------
    // testing.md § "## `#[waterui::test]`" — rust block 3/15 (manual-mount form)
    // -----------------------------------------------------------------------
    use waterui_testing::UiBuilder;

    #[waterui::test]
    fn stepper_updates(ui: UiBuilder) {
        let value = Binding::i32(2);
        let for_view = value.clone();
        let mut app = ui.mount(move || stepper("Limited", &for_view));

        app.query().label("Limited").increment();
        assert_eq!(value.snapshot(), 3);
    }

    // -----------------------------------------------------------------------
    // testing.md § "## `#[waterui::test]`" (prose): a styled manual mount names
    // the style in its parameter type. Not counted as a rust block.
    // -----------------------------------------------------------------------
    pub fn styled_manual_mount(ui: UiBuilder<waterui_testing::Styled<hydrolysis_m3::Material3>>) {
        let _ = ui;
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Querying the accessibility tree" — rust block 4/15
    //
    // One chain. Compiled, never called.
    // -----------------------------------------------------------------------
    pub fn testing_block_04(app: &mut SemanticApp, handle: waterui_testing::ElementRef) {
        use waterui_testing::ContentType;

        app.query()
            .role(Role::SWITCH)
            .label("Wi-Fi") // or .label_contains("Wi-")
            .identifier("settings.wifi")
            .within(&handle) // or .children_of(&handle)
            .enabled(true)
            .selected(true)
            .checked(true)
            .mixed()
            .expanded(true)
            .busy(false)
            .hidden(false)
            .value("42") // or .value_contains("4")
            .placeholder("Search") // or .placeholder_contains("Sea")
            .content_type(ContentType::EmailAddress)
            .assert_exists();

        // The "or" alternatives named in the trailing comments.
        app.query().label_contains("Wi-").assert_exists();
        app.query().children_of(&handle).assert_exists();
        app.query().value_contains("4").assert_exists();
        app.query().placeholder_contains("Sea").assert_exists();
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Querying the accessibility tree" — rust block 5/15
    //
    // A terminator listing. The `-> bool` / `-> ElementRef` / `-> ElementSet` /
    // `-> Option<ElementRef>` arrows are prose annotations, not Rust, so each
    // line is transcribed as the call plus a typed `let` that asserts exactly
    // what the arrow claims.
    // -----------------------------------------------------------------------
    pub fn testing_block_05(app: &mut SemanticApp) {
        app.query().assert_exists();
        app.query().assert_not_exists();
        app.query().assert_ui_focus();

        let _: bool = app.query().exists();
        let _: waterui_testing::ElementRef = app.query().single();
        let _: waterui_testing::ElementSet = app.query().all();
        let _: Option<waterui_testing::ElementRef> = app.query().optional();

        let timeout = Duration::from_secs(2);
        let _: bool = app.query().wait_for_existence(timeout);
        let _: bool = app.query().wait_for_nonexistence(timeout);
        let _: bool = app.query().wait_for_value_eq("Done", timeout);

        // The prose's own instruction: wrap them in `assert!`.
        assert!(app.query().label("Done").wait_for_existence(timeout));
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Interacting" — rust block 6/15
    //
    // The semantic actions: accessibility actions on a located element, on
    // every runtime. Compiled, never called.
    // -----------------------------------------------------------------------
    pub fn testing_block_06(app: &mut SemanticApp) {
        app.query().tap();
        app.query().focus();
        app.query().set_text("hello");
        app.query().increment();
        app.query().decrement();
        app.query().scroll_down();
        app.query().expand();
        app.query().collapse();
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Interacting" — rust block 7/15
    //
    // The pointer gestures: rendered sessions only, so the receiver is the
    // headless runtime. Compiled, never called.
    // -----------------------------------------------------------------------
    pub fn testing_block_07(app: &mut SemanticApp<waterui_testing::HeadlessRuntime>) {
        use waterui_testing::DragOptions;

        let (nx, ny) = (0.5_f32, 0.5_f32);
        let (dx, dy) = (0.0_f32, -24.0_f32);
        let (fx, fy, tx, ty) = (0.1_f32, 0.1_f32, 0.9_f32, 0.9_f32);

        app.query().tap_at(nx, ny);
        app.query().hover();
        app.query().hover_at(nx, ny);

        app.query().drag_by(dx, dy);
        app.query().drag_by_with(
            dx,
            dy,
            DragOptions::default()
                .with_steps(12)
                .with_frame_per_step(true),
        );

        app.query().drag_between(fx, fy, tx, ty);
        app.query().magnify(1.5);
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Interacting" — rust block 8/15 (session input, semantic)
    // -----------------------------------------------------------------------
    pub fn testing_block_08(app: &mut SemanticApp) {
        let modifiers = waterui_testing::Modifiers::default();

        app.text_input("hello");
        app.press_named_key("Tab");
        app.press_named_key_with("Tab", modifiers);
        app.press_character_key_with("a", modifiers);
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Interacting" — rust block 9/15 (held keys)
    // -----------------------------------------------------------------------
    pub fn testing_block_09(app: &mut SemanticApp) {
        use waterui_testing::KeyCode;

        let modifiers = waterui_testing::Modifiers::default();

        let key = KeyCode::Named("Shift".to_string());
        app.key_down(key.clone(), modifiers);
        app.key_up(key, modifiers);
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Interacting" — rust block 10/15 (session input, rendered)
    // -----------------------------------------------------------------------
    pub fn testing_block_10(app: &mut SemanticApp<waterui_testing::HeadlessRuntime>) {
        let (x, y) = (10.0_f32, 20.0_f32);
        let (dx, dy) = (0.0_f32, -24.0_f32);
        let is_line_delta = false;
        let path = "drop.txt"; // glue: the file the drop calls name

        app.tap_at(x, y);
        app.scroll_at(x, y, dx, dy, is_line_delta);
        app.hover_at(x, y);
        app.secondary_click_at(x, y);
        app.pointer_down_at(x, y);
        app.pointer_up_at(x, y);
        app.magnify_at(x, y, 1.5);
        app.hover_files_at(x, y, [path]);
        app.drop_files_at(x, y, [path]);
        app.cancel_files_hover();
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Interacting" (prose): the `queue_*` twins of the
    // rendered session-input calls. Not counted as a rust block.
    // -----------------------------------------------------------------------
    pub fn testing_queue_twins_prose(app: &mut SemanticApp<waterui_testing::HeadlessRuntime>) {
        let (x, y) = (10.0_f32, 20.0_f32);
        let path = "drop.txt";

        app.queue_scroll_at(x, y, 0.0, -24.0, false);
        app.queue_hover_at(x, y);
        app.queue_pointer_down_at(x, y);
        app.queue_pointer_up_at(x, y);
        app.queue_magnify_at(x, y, 1.5);
        app.queue_hover_files_at(x, y, [path]);
        app.queue_drop_files_at(x, y, [path]);
        app.queue_cancel_files_hover();
        app.queue_secondary_click(x, y);
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Waiting" — rust block 11/15
    // -----------------------------------------------------------------------
    pub fn testing_block_11(app: &mut SemanticApp) {
        use waterui_testing::{Selector, WaitOptions};

        let timeout = Duration::from_secs(2);

        let selector = Selector::default().label("Done");

        app.wait_for_existence(&selector, Duration::from_secs(2));
        app.wait_for_nonexistence(&selector, timeout);
        app.wait_for_value_eq(&selector, "Done", timeout);
        app.wait_for(
            &[app.expect_value_eq(selector, "Done")],
            WaitOptions::new(timeout),
        );
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Waiting" (prose): the two frame pumps exist on
    // `OffscreenApp` only. Proven on the type that has them.
    // Not counted as a rust block.
    // -----------------------------------------------------------------------
    pub fn testing_pumps_on_offscreen(app: &mut waterui_testing::OffscreenApp) {
        app.pump_for(Duration::from_millis(120));
        let _ = app.pump_until(Duration::from_secs(2), || true);
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Visual tests and snapshots" — rust block 12/15
    // -----------------------------------------------------------------------
    use waterui_testing::OffscreenApp;

    #[waterui::test(demo, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (390, 844))]
    fn renders(app: &mut OffscreenApp) {
        app.pump_for(Duration::from_millis(120)); // advance the virtual clock exactly
        app.capture_snapshot("gallery", "cards", "settled");
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Visual tests and snapshots" — rust block 13/15
    // Compiled, never called: it would write a PNG to /tmp.
    // -----------------------------------------------------------------------
    pub fn testing_block_13(app: &mut OffscreenApp) {
        let shot = app.snapshot(); // pumps a frame; shot.width(), shot.height(), shot.rgba8()
        shot.save_png("/tmp/my_view.png")
            .expect("snapshot must be writable");
    }

    // -----------------------------------------------------------------------
    // testing.md § "## `#[waterui::bench]` and `water bench`" — rust block 14/15
    // -----------------------------------------------------------------------
    use waterui_testing::PerfApp;

    #[waterui::bench(dashboard, theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844), max_p95_us = 8_000)]
    fn dashboard_redraw(perf: &mut PerfApp) {
        perf.measure("steady-redraw", |run| run.redraw());
        perf.measure("wheel-scroll", |run| {
            run.scroll_at(195.0, 600.0, 0.0, -24.0, false);
        });
    }

    // -----------------------------------------------------------------------
    // testing.md § "## `#[waterui::bench]`" (prose): the remaining budget
    // attribute arguments and `run.pointer_move/down/up`, `run.app()`.
    // Not counted as a rust block.
    // -----------------------------------------------------------------------
    #[waterui::bench(
        dashboard,
        theme = hydrolysis_m3::Material3::defaults(),
        max_mean_us = 8_000,
        max_rebuild_ratio = 0.5,
        max_scene_layers = 64,
        max_gpu_surface_layers = 4,
        max_clip_layers = 8
    )]
    fn dashboard_budgets(perf: &mut PerfApp) {
        perf.measure("pointer", |run| {
            run.pointer_move(10.0, 10.0);
            run.pointer_down(10.0, 10.0);
            run.pointer_up(10.0, 10.0);
            let _ = run.app();
        });
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Waiting" (prose): the `bool` / `WaitResult` return
    // types, `wait_for_ui_focus` and `assert_value_eq`. Not counted as a rust
    // block.
    // -----------------------------------------------------------------------
    pub fn testing_waiting_prose(app: &mut SemanticApp) {
        use waterui_testing::{Selector, WaitOptions, WaitResult};

        let timeout = Duration::from_secs(2);
        let selector = Selector::default().label("Done");

        let _: bool = app.wait_for_existence(&selector, timeout);
        let _: bool = app.wait_for_nonexistence(&selector, timeout);
        let _: bool = app.wait_for_value_eq(&selector, "Done", timeout);
        let _: WaitResult = app.wait_for(
            &[app.expect_value_eq(selector.clone(), "Done")],
            WaitOptions::new(timeout),
        );
        let _: bool = app.wait_for_ui_focus(&selector, timeout);
        app.assert_value_eq(&selector, "Done");
    }

    // -----------------------------------------------------------------------
    // testing.md § "## Visual tests and snapshots" (prose): the snapshot
    // accessors, `scale_factor`, and the builder form of `mount_app`. Not
    // counted as a rust block.
    // -----------------------------------------------------------------------
    pub fn testing_snapshot_prose(app: &mut OffscreenApp, window_app: waterui::app::App) {
        let shot = app.snapshot();
        let _: (u32, u32, &[u8]) = (shot.width(), shot.height(), shot.rgba8());

        let _ = waterui_testing::ui()
            .theme(hydrolysis_m3::Material3::defaults())
            .viewport(390, 844)
            .scale_factor(2.0)
            .environment(waterui::Environment::new())
            .mount_app(window_app);
    }

    // -----------------------------------------------------------------------
    // testing.md § "## `#[waterui::bench]` and `water bench`" — rust block
    // 15/15 (manual form). In its own module: the block's `use` would collide
    // with block 3's `UiBuilder` import.
    // -----------------------------------------------------------------------
    pub mod bench_manual {
        use waterui::prelude::*;

        use waterui_testing::{PerfReport, Styled, UiBuilder};

        #[waterui::bench(theme = hydrolysis_m3::Material3::defaults(), max_rebuild_ratio = 0.2)]
        fn counter_updates(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) -> PerfReport {
            let value = Binding::i32(0);
            let for_view = value.clone();
            ui.perf_with(
                move || stepper("Count", &for_view),
                |perf| {
                    perf.measure("increment", |run| {
                        run.app().query().label("Count").increment();
                    });
                },
            )
        }
    }
}
