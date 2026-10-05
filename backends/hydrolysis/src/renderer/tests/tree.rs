//! Phase 1 unit tests for the persistent retained render tree.

use super::{MinimalTestTheme, test_environment, test_renderer};
use crate::renderer::{ContainerNode, RenderContext, RenderId, RenderNode, TextNode};
use core::cell::{Cell, RefCell};
use kurbo::{Affine, Rect};
use nami::Computed;
use nami::Signal as _;
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui_controls::button::button;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size};
use waterui_core::{AnyView, SignalExt as _};
use waterui_testing::TestArtifacts;

/// Where this module's visual evidence is written: `waterui-testing`'s
/// canonical `<root>/hydrolysis/<case>/<stage>.png` layout, with the root from
/// `WATERUI_TEST_ARTIFACTS_DIR` when CI sets it (uploaded with every run) and
/// the platform temp directory otherwise.
fn export_path(case: &str, stage: &str) -> std::path::PathBuf {
    let path = TestArtifacts::new("hydrolysis").snapshot_path(case, stage);
    std::fs::create_dir_all(path.parent().expect("a snapshot path has a case directory"))
        .expect("the export directory must be creatable");
    path
}
use waterui_layout::stack::{VStackLayout, vstack};
use waterui_text::styled::StyledStr;

fn text_node(content: &'static str) -> RenderNode {
    let content = Computed::constant(StyledStr::plain(content));
    let alignment = Computed::constant(HorizontalAlignment::Leading);
    RenderNode::Text(Box::new(TextNode {
        memo_gate: Cell::default(),
        memo_slots: RefCell::default(),
        accessibility_identity: Rc::new(()),
        render_id: RenderId::next(),
        _guards: [content.watch(|_| {}), alignment.watch(|_| {})],
        content,
        alignment,
        line_limit: None,
        layout_dirty: Rc::new(Cell::new(false)),
    }))
}

#[test]
fn render_node_container_lays_out_and_flushes_text() {
    let env = test_environment();
    let mut renderer = test_renderer();

    let mut node = RenderNode::Container(Box::new(ContainerNode {
        memo_gate: Cell::default(),
        memo_slots: RefCell::default(),
        accessibility_identity: Rc::new(()),
        render_id: RenderId::next(),
        layout: Box::new(VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(8.0),
        }),
        children: vec![text_node("Hello"), text_node("World")],
        #[cfg(feature = "accessibility")]
        accessibility_child_env: None,
        placed: Vec::new(),
        #[cfg(feature = "accessibility")]
        resolved: waterui_core::layout::Rect::from_size(Size::zero()),
        layout_dirty: Rc::new(Cell::new(false)),
        _guards: Vec::new(),
    }));

    let window = Size::new(200.0, 120.0);
    let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    node.layout(
        &mut renderer,
        &env,
        ProposalSize::new(Some(window.width), Some(window.height)),
        window,
    );

    match &node {
        RenderNode::Container(container) => {
            assert_eq!(
                container.placed.len(),
                2,
                "the container must cache one frame per child"
            );
            assert!(
                container.placed[1].y() > container.placed[0].y(),
                "a VStack must stack its children top-to-bottom, got {:?}",
                container.placed
            );
        }
        _ => panic!("expected a container node"),
    }

    let ctx = RenderContext::with_transforms(bounds, Affine::IDENTITY, Affine::IDENTITY);
    node.flush(&mut renderer, ctx, &env);
    // Check before `finish_rebuild_frame`, which moves the scene into the
    // compositor's layer stack (leaving `renderer.scene` reset).
    assert!(
        !renderer.scene_is_empty(),
        "flushing two text nodes must draw glyphs into the scene"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn geometry_static_flush_reuses_cached_placement() {
    let env = test_environment();
    let mut renderer = test_renderer();

    let mut node = RenderNode::Container(Box::new(ContainerNode {
        memo_gate: Cell::default(),
        memo_slots: RefCell::default(),
        accessibility_identity: Rc::new(()),
        render_id: RenderId::next(),
        layout: Box::new(VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(8.0),
        }),
        children: vec![text_node("Cached")],
        #[cfg(feature = "accessibility")]
        accessibility_child_env: None,
        placed: Vec::new(),
        #[cfg(feature = "accessibility")]
        resolved: waterui_core::layout::Rect::from_size(Size::zero()),
        layout_dirty: Rc::new(Cell::new(false)),
        _guards: Vec::new(),
    }));

    let window = Size::new(200.0, 120.0);
    let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);

    renderer.begin_rebuild_frame();
    node.layout(
        &mut renderer,
        &env,
        ProposalSize::new(Some(window.width), Some(window.height)),
        window,
    );
    let placed_after_layout = match &node {
        RenderNode::Container(container) => container.placed.clone(),
        _ => panic!("expected a container node"),
    };

    // A geometry-static frame re-encodes without re-running layout; the cached
    // placement must survive untouched across flushes.
    let ctx = RenderContext::with_transforms(bounds, Affine::IDENTITY, Affine::IDENTITY);
    renderer.reset_scene();
    node.flush(&mut renderer, ctx, &env);
    renderer.reset_scene();
    node.flush(&mut renderer, ctx, &env);
    renderer.finish_rebuild_frame();

    match &node {
        RenderNode::Container(container) => {
            assert_eq!(
                container.placed, placed_after_layout,
                "flush must not mutate cached placements"
            );
        }
        _ => panic!("expected a container node"),
    }
}

#[test]
fn opacity_wrapper_builds_and_flushes_via_dsl() {
    let env = test_environment();
    let mut renderer = test_renderer();

    // `.opacity(..)` lowers to `Metadata<Opacity>` wrapping the text; the build
    // walk must turn it into an `Opacity` wrapper node over a `Text` leaf.
    let view = AnyView::new(waterui_text::text("Faded").opacity(0.5));
    let mut node = RenderNode::build(view, &env, &mut renderer);
    assert!(
        matches!(node, RenderNode::Opacity(_)),
        "opacity-wrapped text must build an Opacity wrapper node"
    );

    let window = Size::new(200.0, 80.0);
    let bounds = Rect::new(0.0, 0.0, 200.0, 80.0);

    renderer.begin_rebuild_frame();
    node.layout(
        &mut renderer,
        &env,
        ProposalSize::new(Some(window.width), Some(window.height)),
        window,
    );
    let ctx = RenderContext::with_transforms(bounds, Affine::IDENTITY, Affine::IDENTITY);
    node.flush(&mut renderer, ctx, &env);
    assert!(
        !renderer.scene_is_empty(),
        "an opacity-wrapped text must still draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn capture_window_tree_renders_mixed_widgets() {
    let env = test_environment();
    let mut renderer = test_renderer();

    // text -> Text node; button -> Captured leaf (Native<ButtonConfig> the
    // dispatcher renders); vstack -> Container. Exercises the whole build walk.
    let view = AnyView::new(vstack((
        waterui_text::text("Title"),
        button("Tap").action(|| {}),
    )));
    let bounds = Rect::new(0.0, 0.0, 220.0, 200.0);

    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(view, &env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    assert!(
        !renderer.scene_is_empty(),
        "the render-tree path must draw a mixed text + widget view"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn flush_window_tree_reuses_retained_tree() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 220.0, 200.0);

    let view = AnyView::new(vstack((
        waterui_text::text("Title"),
        button("Tap").action(|| {}),
    )));
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(view, &env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    renderer.finish_rebuild_frame();

    // A geometry-static frame re-flushes the retained tree without rebuilding it.
    // `flush_window_tree` does full frame management (reset + flush + move the
    // scene into the compositor's layer stack), so verify a scene segment resulted.
    let flushed = renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    assert!(flushed, "a retained tree must be present to flush");
    let scene_layers = renderer.render_layer_stats().scene_segments;
    assert!(
        scene_layers > 0,
        "re-flushing the retained tree must produce a scene segment layer"
    );
}

/// A reactive label INSIDE a native widget (a button) stays live on the render-tree
/// path: the button is a persistent `Widget` node that re-renders from its retained
/// config every flush, so a `text!`-driven label updates instead of freezing in a
/// one-shot capture. Before/after a binding change must differ.
#[test]
fn widget_reactive_label_stays_live() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui::text;
    use waterui_core::handler::AnyViewBuilder;

    let n = binding(0i32);
    let builder = {
        let n = n.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let n = n.clone();
            AnyView::new(button(text!("N={n}", n = n)).action(|| {}))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 200, 120, MinimalTestTheme::default());
    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame snapshot");
    n.set(7);
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("post-change snapshot");
    assert!(
        before.rgba8 != after.rgba8,
        "a reactive button label must update on the render-tree path (the persistent \
         Widget node must re-render from the live config, not freeze a captured bake)"
    );
}

/// The single-pump contract: a reactive *value* change that alters a leaf's size
/// reflows the layout through the cheap refresh pump. Full layout runs every frame,
/// so a widening text pushes its trailing sibling — and the frame is a window
/// *refresh*, not a structural `body()` rebuild (`watch_signal` schedules a refresh).
/// This is what lets every reactive update read current state + relayout + redraw on
/// one uniform path instead of re-running the view tree.
#[test]
fn reactive_size_change_reflows_via_refresh_not_rebuild() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::prelude::Color;
    use waterui::reactive::binding;
    use waterui::text;
    use waterui_core::Str;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_layout::stack::hstack;

    let label = binding(Str::from("x"));
    let builder = {
        let label = label.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let label = label.clone();
            AnyView::new(hstack((
                text!("{label}", label = label),
                ().size(40.0, 40.0).background(Color::srgb_hex("#2563EB")),
            )))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 320, 80, MinimalTestTheme::default());
    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame snapshot");
    label.set(Str::from("a very wide label that grows"));
    let result = rt.pump_at(true, start + Duration::from_millis(16));
    assert!(
        !result.rebuilt,
        "a reactive value change must reflow through the refresh pump, not a structural \
         body() rebuild"
    );
    let after = result.snapshot.expect("post-change snapshot");
    assert_eq!(
        (before.width, before.height),
        (after.width, after.height),
        "snapshots must share dimensions to compare pixel-for-pixel"
    );
    assert_ne!(
        before.rgba8, after.rgba8,
        "a widening reactive text must reflow its sibling through full per-frame layout"
    );
}

/// A reactive *value* inside a native widget (a `Progress` bound to a
/// `Binding<f64>`) stays live on the render-tree path: the progress indicator is a
/// persistent `Widget` node that re-renders from its retained config every flush,
/// reading the value through `read_signal` (so a change schedules a frame). Its
/// percentage value-label is reactive text derived from the value, so flipping the
/// binding renders a different percentage string ("0%" -> "80%") — glyphs the test
/// theme actually draws — proving the value is not frozen in a one-shot capture.
#[test]
fn widget_reactive_value_stays_live() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::component::progress::progress;
    use waterui::reactive::binding;
    use waterui_core::handler::AnyViewBuilder;

    let value = binding(0.0f64);
    let builder = {
        let value = value.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let value = value.clone();
            AnyView::new(progress(value))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 240, 120, MinimalTestTheme::default());
    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame snapshot");
    value.set(0.8);
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("post-change snapshot");
    assert_eq!(
        (before.width, before.height),
        (after.width, after.height),
        "snapshots must share dimensions to compare pixel-for-pixel"
    );
    assert_ne!(
        before.rgba8, after.rgba8,
        "a reactive progress value must update on the render-tree path (the persistent \
         Widget node must re-render its value-label from the live binding, not freeze a \
         captured bake)"
    );
}

/// A reactive *value display* inside a native text field stays live on the
/// render-tree path: a `TextField` bound to a `Binding<Str>` is a persistent
/// `Widget` node that re-renders from its retained config every flush, reading the
/// value through `read_signal` (so a binding change schedules a frame). Changing the
/// bound string from one set of glyphs ("AAA") to another ("ZZZ") renders a
/// different displayed value — proving the field's value display is not frozen in a
/// one-shot capture, which is the most important text-field reactivity guarantee.
#[test]
fn text_field_value_display_stays_live() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_controls::text_field::TextField;
    use waterui_core::Str;
    use waterui_core::handler::AnyViewBuilder;

    let value = binding(Str::from("AAA"));
    let builder = {
        let value = value.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(TextField::new("Value", &value).hide_label())
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 240, 120, MinimalTestTheme::default());
    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame snapshot");
    value.set(Str::from("ZZZ"));
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("post-change snapshot");
    assert_eq!(
        (before.width, before.height),
        (after.width, after.height),
        "snapshots must share dimensions to compare pixel-for-pixel"
    );
    assert_ne!(
        before.rgba8, after.rgba8,
        "a reactive text field value must update on the render-tree path (the persistent \
         Widget node must re-render its displayed value from the live binding, not freeze a \
         captured bake)"
    );
}

#[test]
fn render_tree_live_path_processes_watch_switch() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::dynamic::watch;
    use waterui_core::handler::AnyViewBuilder;

    // A `watch`-driven content switch (the chart example's `chart_layers` shape):
    // a scene bound to a frame-ordered effect slot could silently fail to switch. The
    // render-tree path patches only the Dynamic node, so the switch always takes.
    let mode = binding(false);
    let builder = {
        let mode = mode.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let mode = mode.clone();
            AnyView::new(watch(mode, |selected| {
                if selected {
                    AnyView::new(waterui_text::text("Mode B"))
                } else {
                    AnyView::new(waterui_text::text("Mode A"))
                }
            }))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 200, 120, MinimalTestTheme::default());

    let start = Instant::now();
    let first = rt.pump_at(false, start);
    assert!(
        first.rebuilt,
        "the initial frame must build the render tree"
    );

    mode.set(true);
    let switched = rt.pump_at(false, start + Duration::from_millis(16));
    // The switch is applied by reusing the persistent tree (Dynamic patch +
    // relayout + flush), not by rebuilding it from scratch (which would
    // re-connect the Dynamic). It must render a frame.
    assert!(
        switched.profile.counters.rendered,
        "a watch-driven switch must render a frame on the render-tree path"
    );
}

/// Build-once contract: the view tree's `body()` is dispatched recursively exactly
/// ONCE — on the first frame, which builds the persistent render tree. Every later
/// frame (a reactive value change, a structural `watch` switch) refreshes that
/// retained tree (`rebuilt == false`), never re-running `build_content` / `body()`.
/// This is what makes the per-frame pump uniform: build once at startup, then read
/// current state + patch + full layout + draw every frame.
#[test]
fn body_dispatched_once_then_every_frame_refreshes() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui::text;
    use waterui_core::dynamic::watch;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_layout::stack::vstack;

    let label = binding(0i32);
    let mode = binding(false);
    let builder = {
        let label = label.clone();
        let mode = mode.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let label = label.clone();
            let mode = mode.clone();
            AnyView::new(vstack((
                text!("N={label}", label = label),
                watch(mode, |selected| {
                    if selected {
                        AnyView::new(waterui_text::text("B"))
                    } else {
                        AnyView::new(waterui_text::text("A"))
                    }
                }),
            )))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 200, 160, MinimalTestTheme::default());
    let start = Instant::now();

    assert!(
        rt.pump_at(false, start).rebuilt,
        "the first frame must build the persistent tree (recursive body() once)"
    );
    label.set(7);
    assert!(
        !rt.pump_at(false, start + Duration::from_millis(16)).rebuilt,
        "a reactive value change must refresh the persistent tree, not re-run body()"
    );
    mode.set(true);
    assert!(
        !rt.pump_at(false, start + Duration::from_millis(32)).rebuilt,
        "a structural watch switch must patch + refresh, not rebuild the whole tree"
    );
    label.set(9);
    assert!(
        !rt.pump_at(false, start + Duration::from_millis(48)).rebuilt,
        "subsequent reactive changes keep refreshing the one persistent tree"
    );
}

/// Visual verification of the chart-switch fix: a `watch`-driven swap between two
/// distinctly-coloured boxes (through the same `Captured` path a `SceneView`
/// chart takes) rendered via the render-tree path. Exports before/after PNG files for
/// direct inspection — the switch must visibly take effect (red -> blue).
#[test]
fn render_tree_chart_switch_snapshot() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::dynamic::watch;
    use waterui_core::handler::AnyViewBuilder;

    fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: Vec<u8>) {
        let image = image::RgbaImage::from_raw(width, height, rgba)
            .expect("snapshot dimensions must match the rgba buffer");
        image.save(path).expect("snapshot png must be writable");
    }

    let mode = binding(false);
    let builder = {
        let mode = mode.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let mode = mode.clone();
            AnyView::new(watch(mode, |selected| {
                use waterui::prelude::*;
                if selected {
                    AnyView::new(().size(140.0, 140.0).background(Color::srgb_hex("#2563EB")))
                } else {
                    AnyView::new(().size(140.0, 140.0).background(Color::srgb_hex("#DC2626")))
                }
            }))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");
    write_png(
        &export_path("switch", "before"),
        before.width,
        before.height,
        before.rgba8,
    );

    mode.set(true);
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("switched frame must capture a snapshot");
    write_png(
        &export_path("switch", "after"),
        after.width,
        after.height,
        after.rgba8,
    );

    eprintln!("wrote /tmp/waterui_tree_switch_before.png and /tmp/waterui_tree_switch_after.png");
}

/// The exact chart case: a `watch`-driven swap between two `Canvas` (`SceneView`)
/// charts — the effect-slot path where a swap could render the previous scene. On the render-tree path the
/// switch must visibly take effect (red -> blue scene), proving the `SceneView`
/// `SceneView` fixed (the patched node is re-dispatched in isolation).
#[test]
fn render_tree_scene_view_switch_snapshot() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::ViewExt as _;
    use waterui::reactive::binding;
    use waterui_canvas::Canvas;
    use waterui_core::dynamic::watch;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_core::layout::{Point, Rect, Size as LayoutSize};
    use waterui_graphics::color::Srgb;

    fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: Vec<u8>) {
        let image = image::RgbaImage::from_raw(width, height, rgba)
            .expect("snapshot dimensions must match the rgba buffer");
        image.save(path).expect("snapshot png must be writable");
    }

    fn canvas_box(r: u8, g: u8, b: u8) -> AnyView {
        AnyView::new(
            Canvas::new(move |ctx| {
                ctx.set_fill_style(Srgb::new_u8(r, g, b));
                ctx.fill_rect(Rect::new(Point::zero(), LayoutSize::new(150.0, 150.0)));
            })
            .size(150.0, 150.0),
        )
    }

    let mode = binding(false);
    let builder = {
        let mode = mode.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let mode = mode.clone();
            AnyView::new(watch(mode, |selected| {
                if selected {
                    canvas_box(0x25, 0x63, 0xEB)
                } else {
                    canvas_box(0xDC, 0x26, 0x26)
                }
            }))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");
    write_png(
        &export_path("sceneview", "before"),
        before.width,
        before.height,
        before.rgba8,
    );

    mode.set(true);
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("switched frame must capture a snapshot");
    write_png(
        &export_path("sceneview", "after"),
        after.width,
        after.height,
        after.rgba8,
    );

    eprintln!(
        "wrote /tmp/waterui_tree_sceneview_before.png and /tmp/waterui_tree_sceneview_after.png"
    );
}

/// Verifies scroll works on the render-tree path: a fixed-content scroll view is
/// scrolled, and the exported before/after PNG files must show the content shifted
/// (different colour band at the top). Tells us whether a dedicated `ScrollNode`
/// is needed or the Captured scroll replays at the current offset correctly.
#[test]
fn render_tree_scroll_snapshot() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui_core::handler::AnyViewBuilder;

    fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: Vec<u8>) {
        let image = image::RgbaImage::from_raw(width, height, rgba)
            .expect("snapshot dimensions must match the rgba buffer");
        image.save(path).expect("snapshot png must be writable");
    }

    fn block(hex: &'static str) -> AnyView {
        use waterui::prelude::*;
        AnyView::new(().size(160.0, 60.0).background(Color::srgb_hex(hex)))
    }
    fn screen() -> AnyView {
        use waterui::prelude::*;
        AnyView::new(scroll(vstack((
            block("#DC2626"),
            block("#16A34A"),
            block("#2563EB"),
            block("#CA8A04"),
            block("#9333EA"),
        ))))
    }

    let builder = AnyViewBuilder::<AnyView>::new(screen);
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");
    write_png(
        &export_path("scroll", "before"),
        before.width,
        before.height,
        before.rgba8,
    );

    rt.push_input_event(crate::platform::InputEvent::Scroll {
        x: 80.0,
        y: 80.0,
        dx: 0.0,
        dy: -150.0,
        is_line_delta: false,
    });
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("scrolled frame must capture a snapshot");
    write_png(
        &export_path("scroll", "after"),
        after.width,
        after.height,
        after.rgba8,
    );

    eprintln!("wrote /tmp/waterui_tree_scroll_before.png and /tmp/waterui_tree_scroll_after.png");
}

/// The scroll offset must survive the refresh pump. The retained `ScrollNode` owns
/// one handle and re-binds its extents on each layout; no renderer-order slot is
/// involved. Asserts the scrolled frame differs from the unscrolled one, and that
/// a following no-op refresh keeps the offset (no reset, no drift).
#[test]
fn scroll_offset_persists_across_refresh() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui_core::handler::AnyViewBuilder;

    fn block(hex: &'static str) -> AnyView {
        use waterui::prelude::*;
        AnyView::new(().size(160.0, 60.0).background(Color::srgb_hex(hex)))
    }
    fn screen() -> AnyView {
        use waterui::prelude::*;
        AnyView::new(scroll(vstack((
            block("#DC2626"),
            block("#16A34A"),
            block("#2563EB"),
            block("#CA8A04"),
            block("#9333EA"),
        ))))
    }

    let builder = AnyViewBuilder::<AnyView>::new(screen);
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());
    let start = Instant::now();

    let unscrolled = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame snapshot");
    rt.push_input_event(crate::platform::InputEvent::Scroll {
        x: 80.0,
        y: 80.0,
        dx: 0.0,
        dy: -150.0,
        is_line_delta: false,
    });
    let scrolled = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("scrolled frame snapshot");
    assert_ne!(
        unscrolled.rgba8, scrolled.rgba8,
        "scrolling must change the rendered content — the offset must apply through the refresh pump"
    );
    let still_scrolled = rt
        .pump_at(true, start + Duration::from_millis(32))
        .snapshot
        .expect("re-pumped frame snapshot");
    assert_eq!(
        scrolled.rgba8, still_scrolled.rgba8,
        "a no-op refresh must preserve the scroll offset, not reset or drift it"
    );
}

/// Verifies a reactive collection (`ForEach` in a scroll → `LazyContainer`)
/// renders via the tree path: the exported PNG must show the stacked coloured
/// rows.
#[test]
fn render_tree_collection_snapshot() {
    use waterui_core::handler::AnyViewBuilder;
    use waterui_core::id::SelfId;

    fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: Vec<u8>) {
        let image = image::RgbaImage::from_raw(width, height, rgba)
            .expect("snapshot dimensions must match the rgba buffer");
        image.save(path).expect("snapshot png must be writable");
    }

    fn screen() -> AnyView {
        use waterui::prelude::*;
        let colors = ["#DC2626", "#16A34A", "#2563EB", "#CA8A04", "#9333EA"];
        let data: Vec<_> = (0..5).map(SelfId::new).collect();
        AnyView::new(scroll(VStack::for_each(data, move |item| {
            let index = item.into_inner();
            ().size(160.0, 40.0)
                .background(Color::srgb_hex(colors[index % colors.len()]))
        })))
    }

    let builder = AnyViewBuilder::<AnyView>::new(screen);
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());

    let snapshot = rt
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("collection frame must capture a snapshot");
    write_png(
        &export_path("collection", "review"),
        snapshot.width,
        snapshot.height,
        snapshot.rgba8,
    );
    eprintln!("wrote /tmp/waterui_tree_collection.png");
}

/// Regression for the wrapper-freeze bug: a `watch`-driven colour swap wrapped in
/// `.border(...)` must keep updating through the wrapper. On the old code a
/// `Metadata<Border>` fell through to a one-shot `Captured` node that froze every
/// reactive descendant, so mutating the binding produced an identical frame. With
/// the transparent `Wrapper` node the effect re-applies and the child `Dynamic`
/// node patches, so the two frames must DIFFER (red -> blue through the border).
#[test]
fn wrapper_keeps_reactive_descendant_live() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::dynamic::watch;
    use waterui_core::handler::AnyViewBuilder;

    let mode = binding(false);
    let builder = {
        let mode = mode.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let mode = mode.clone();
            // A border wrapping a `watch`-driven box: the box swaps colour when the
            // binding flips. The reactive box reaches a dedicated `Dynamic` node, so
            // the swap only takes effect if the `.border()` wrapper recurses into it
            // (rather than capturing-and-freezing it).
            AnyView::new(
                watch(mode, |selected| {
                    use waterui::prelude::*;
                    if selected {
                        AnyView::new(().size(120.0, 120.0).background(Color::srgb_hex("#2563EB")))
                    } else {
                        AnyView::new(().size(120.0, 120.0).background(Color::srgb_hex("#DC2626")))
                    }
                })
                .border(waterui::prelude::Color::srgb_hex("#000000"), 4.0),
            )
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");

    mode.set(true);
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("switched frame must capture a snapshot");

    assert_eq!(
        (before.width, before.height),
        (after.width, after.height),
        "snapshots must share dimensions to compare pixel-for-pixel"
    );
    assert_ne!(
        before.rgba8, after.rgba8,
        "the reactive box must update through the .border() wrapper: a Captured \
         wrapper would freeze the descendant and produce an identical frame"
    );
}

/// Regression for the wrapper-freeze bug on the gesture path: a `watch`-driven
/// colour swap wrapped in `.on_tap(...)` (a `Metadata<GestureObserver>`) must keep
/// updating through the wrapper. Before this conversion `Metadata<GestureObserver>`
/// fell through to a one-shot `Captured` node that froze every reactive descendant,
/// so mutating the binding produced an identical frame. With the transparent
/// `GestureObserver` `Wrapper` node the gesture re-registers and the child
/// `Dynamic` node patches, so the two frames must DIFFER (red -> blue under the
/// tap target).
#[test]
fn gesture_wrapper_keeps_reactive_descendant_live() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::dynamic::watch;
    use waterui_core::handler::AnyViewBuilder;

    let mode = binding(false);
    let builder = {
        let mode = mode.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let mode = mode.clone();
            // A tap target wrapping a `watch`-driven box: the box swaps colour when
            // the binding flips. The reactive box reaches a dedicated `Dynamic`
            // node, so the swap only takes effect if the `.on_tap()` gesture wrapper
            // recurses into it (rather than capturing-and-freezing it).
            AnyView::new(
                watch(mode, |selected| {
                    use waterui::prelude::*;
                    if selected {
                        AnyView::new(().size(120.0, 120.0).background(Color::srgb_hex("#2563EB")))
                    } else {
                        AnyView::new(().size(120.0, 120.0).background(Color::srgb_hex("#DC2626")))
                    }
                })
                .on_tap(|| {}),
            )
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");

    mode.set(true);
    let after = rt
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("switched frame must capture a snapshot");

    assert_eq!(
        (before.width, before.height),
        (after.width, after.height),
        "snapshots must share dimensions to compare pixel-for-pixel"
    );
    assert_ne!(
        before.rgba8, after.rgba8,
        "the reactive box must update through the .on_tap() gesture wrapper: a \
         Captured wrapper would freeze the descendant and produce an identical frame"
    );
}

/// Diagnostic: a 3-column `grid` of fixed-size colour cells must render all rows
/// (the chart example's mode-button grid regressed to a single overlapping row on
/// the tree path). The exported PNG must show three stacked rows of three cells.
#[test]
fn render_tree_grid_snapshot() {
    use waterui_core::handler::AnyViewBuilder;

    fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: Vec<u8>) {
        let image = image::RgbaImage::from_raw(width, height, rgba)
            .expect("snapshot dimensions must match the rgba buffer");
        image.save(path).expect("snapshot png must be writable");
    }

    fn screen() -> AnyView {
        use waterui::layout::grid::{grid, row};
        use waterui::prelude::*;
        fn cell(label: &str) -> impl View {
            button(label.to_owned()).width(92.0)
        }
        let controls = grid(
            3,
            [
                row((cell("Bar"), cell("Line"), cell("Pie"))),
                row((cell("Scatter"), cell("Candle"), cell("Depth"))),
                row((cell("Heatmap"), cell("Contour"), cell("Radar"))),
            ],
        )
        .spacing(10.0);
        AnyView::new(vstack((
            controls,
            spacer(),
            ().size(200.0, 100.0).background(Color::srgb_hex("#2563EB")),
            spacer(),
        )))
    }

    let builder = AnyViewBuilder::<AnyView>::new(screen);
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 440, 920, MinimalTestTheme::default());

    let snapshot = rt
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("grid frame must capture a snapshot");
    write_png(
        &export_path("grid", "review"),
        snapshot.width,
        snapshot.height,
        snapshot.rgba8,
    );
    eprintln!("wrote /tmp/waterui_tree_grid.png");
}

/// Lifecycle hooks are node-owned on the retained tree: `.on_appear` fires once
/// after the child's first flush, and `.on_disappear` fires from the node's `Drop`
/// when the retained tree is torn down. Structural presence/removal drives the
/// hooks — there is no frame-diff slot cursor to drift out of step with the tree, and
/// the disappear hook must not fire while the node is still retained.
#[test]
fn lifecycle_hooks_fire_after_first_flush_and_on_drop() {
    use core::cell::Cell;
    use std::rc::Rc;

    let appeared = Rc::new(Cell::new(false));
    let disappeared = Rc::new(Cell::new(false));
    let view = {
        let appeared = Rc::clone(&appeared);
        let disappeared = Rc::clone(&disappeared);
        AnyView::new(
            waterui_text::text("Lifecycle")
                .on_appear(move || appeared.set(true))
                .on_disappear(move || disappeared.set(true)),
        )
    };

    let env = test_environment();
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);

    renderer.prepare_window_tree(view, &env);
    assert!(
        !appeared.get(),
        "building the retained node must not fire on_appear before its child is flushed"
    );

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(()),
        &env,
        bounds,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();

    assert!(
        appeared.get(),
        "on_appear must fire after the lifecycle wrapper's child first flushes"
    );
    assert!(
        !disappeared.get(),
        "on_disappear must not fire while the node is retained in the tree"
    );

    // Tearing down the renderer drops the retained tree, so the disappear hook
    // fires from the lifecycle node's Drop — structural removal, not a slot diff.
    drop(renderer);
    assert!(
        disappeared.get(),
        "on_disappear must fire when the retained tree (the lifecycle node) is dropped"
    );
}

/// A Snackbar-style entrance keeps its hidden target through the first animated
/// signal sample, then changes that target from `on_appear`. This must create an
/// active animation instead of binding the already-settled value on the first
/// frame and popping directly to the final state.
#[test]
fn lifecycle_appear_updates_animate_after_initial_signal_binding() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::animation::Animation;
    use waterui::reactive::binding;

    let opacity = binding(0.0f32);
    let opacity_for_appear = opacity.clone();
    let view = AnyView::new(
        ().size(40.0, 40.0)
            .on_appear(move || opacity_for_appear.set(1.0))
            .opacity(opacity.with(Animation::linear(Duration::from_millis(250)))),
    );

    let env = test_environment();
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 120.0, 120.0);
    let start = Instant::now();
    renderer.set_frame_instant(start);
    renderer.prepare_window_tree(view, &env);

    assert_eq!(
        opacity.snapshot(),
        0.0,
        "the entrance target must remain hidden until the child first flushes"
    );
    assert!(
        !renderer.animations_active(),
        "building alone must not start an entrance animation"
    );

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(()),
        &env,
        bounds,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();

    assert_eq!(
        opacity.snapshot(),
        1.0,
        "on_appear must update the entrance target after the initial sample"
    );
    assert!(
        renderer.animations_active(),
        "the post-bind target update must leave the entrance animation active"
    );
}

/// A GPU filter (`.blur(...)`) is built and flushed through the retained tree as a
/// node-owned filtered mount, not the old dispatch capture/replay path. The node
/// owns its `FilteredRuntime`; a re-flush of the geometry-static tree keeps the
/// same runtime alive (it is pruned only when its node is dropped), so the filter
/// mount survives across frames without a cursor-bound effect slot.
#[test]
fn applied_filter_renders_through_retained_tree() {
    fn blurred_box() -> AnyView {
        use waterui::prelude::*;
        AnyView::new(
            ().size(48.0, 48.0)
                .background(Color::srgb_hex("#DC2626"))
                .blur(6.0f32),
        )
    }

    let env = test_environment();
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 120.0, 120.0);

    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        blurred_box(),
        &env,
        bounds,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();

    let flushed = renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    assert!(flushed, "the retained tree must re-flush");
    assert_eq!(
        renderer.render_layer_stats().filtered_subtrees,
        1,
        "a .blur() view must mount a node-owned filtered layer on the retained tree, \
         not fall through to a dispatch/capture path"
    );

    // A geometry-static re-flush keeps the node — and thus its filter runtime —
    // alive (pruned only on node drop, by Rc strong count).
    let flushed = renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    assert!(flushed, "the retained tree must re-flush a second time");
    assert_eq!(
        renderer.render_layer_stats().filtered_subtrees,
        1,
        "the node-owned filter mount must survive a geometry-static re-flush \
         (the retained FilteredView node keeps owning it across frames)"
    );
}

/// Contiguous runs of rows inside `rect` that contain a pixel differing from
/// the region's modal colour — the text bands a control draws inside it. Rows
/// separated by fewer than 6 blank rows merge, so a glyph's disconnected piece
/// (the dot of an 'i') cannot split its own text line into two bands.
fn text_ink_bands(snapshot: &crate::HeadlessSnapshot, rect: accesskit::Rect) -> Vec<(f64, f64)> {
    use std::collections::HashMap;
    let x0 = crate::num_cast::f64_as_usize(rect.x0.floor().max(0.0));
    let x1 = (crate::num_cast::f64_as_usize(rect.x1.ceil())).min(snapshot.width as usize);
    let y0 = crate::num_cast::f64_as_usize(rect.y0.floor().max(0.0));
    let y1 = (crate::num_cast::f64_as_usize(rect.y1.ceil())).min(snapshot.height as usize);
    let pixel = |x: usize, y: usize| {
        let i = (y * snapshot.width as usize + x) * 4;
        &snapshot.rgba8[i..i + 4]
    };
    let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
    for y in y0..y1 {
        for x in x0..x1 {
            *counts.entry(pixel(x, y).try_into().unwrap()).or_default() += 1;
        }
    }
    let bg = counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .expect("the field region is non-empty")
        .0;
    let mut bands: Vec<(f64, f64)> = Vec::new();
    for y in y0..y1 {
        if !(x0..x1).any(|x| pixel(x, y) != bg.as_slice()) {
            continue;
        }
        match bands.last_mut() {
            Some((_, end)) if crate::num_cast::usize_as_f64(y) - *end < 6.0 => {
                *end = crate::num_cast::usize_as_f64(y) + 1.0;
            }
            _ => bands.push((
                crate::num_cast::usize_as_f64(y),
                crate::num_cast::usize_as_f64(y) + 1.0,
            )),
        }
    }
    bands
}

/// The menu picker's field label is drawn: its ink band sits directly above the
/// selected value's, both inside the picker's field (the `ComboBox`'s bounds).
/// A hidden label draws nothing and takes no space — the value band alone
/// remains — while the accessibility node keeps the label exactly once.
#[cfg(feature = "accessibility")]
#[test]
fn menu_picker_draws_its_label_above_the_value() {
    use accesskit::Role;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_form::picker::{PickerStyle, picker};
    use waterui_layout::stack::vstack;
    use waterui_text::text;

    fn mount_labelled(hide_label: bool) -> crate::HeadlessRuntime {
        let selection = binding(0i32);
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let menu = picker(
                "Size",
                vec![text("Small").tag(0i32), text("Large").tag(1i32)],
                &selection,
            )
            .style(PickerStyle::Menu);
            let menu = if hide_label { menu.hide_label() } else { menu };
            AnyView::new(vstack((menu,)))
        });
        crate::HeadlessRuntime::new_for_tests(
            test_environment(),
            builder,
            320,
            120,
            MinimalTestTheme::default(),
        )
    }

    fn field_bounds(update: &accesskit::TreeUpdate) -> accesskit::Rect {
        update
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::ComboBox && node.label() == Some("Size"))
            .and_then(|(_, node)| node.bounds())
            .expect("the menu picker's field node must carry bounds")
    }

    let mut labelled = mount_labelled(false);
    let result = labelled.pump_at(true, Instant::now());
    let update = result
        .tree_update
        .expect("the labelled picker must publish a tree");
    let field = field_bounds(&update);
    let snapshot = result.snapshot.expect("a snapshot must be captured");
    let bands = text_ink_bands(&snapshot, field);
    assert_eq!(
        bands.len(),
        2,
        "a labelled menu picker draws two text bands inside its field — label above value, got {bands:?}"
    );
    let (label_band, value_band) = (bands[0], bands[1]);
    assert!(
        label_band.1 <= value_band.0,
        "the label band {label_band:?} must sit above the value band {value_band:?}"
    );
    assert!(
        label_band.0 >= field.y0 && value_band.1 <= field.y1,
        "both text bands must lie inside the field {field:?}"
    );

    let mut hidden = mount_labelled(true);
    let result = hidden.pump_at(true, Instant::now());
    let update = result
        .tree_update
        .expect("the hidden-label picker must publish a tree");
    let field = field_bounds(&update);
    let snapshot = result.snapshot.expect("a snapshot must be captured");
    let bands = text_ink_bands(&snapshot, field);
    assert_eq!(
        bands.len(),
        1,
        "a hidden label draws nothing — only the value band remains, got {bands:?}"
    );
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Size"))
            .count(),
        1,
        "a hidden label still names the picker's single node exactly once"
    );
}

/// The radio picker's group label is drawn: its ink band sits above the first
/// option row inside the group's bounds. A hidden label draws nothing and
/// takes no space — the option bands alone remain — while the group node
/// keeps the label exactly once.
#[cfg(feature = "accessibility")]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn radio_picker_draws_its_label_above_the_option_rows() {
    use accesskit::Role;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_form::picker::{PickerStyle, picker};
    use waterui_layout::stack::vstack;
    use waterui_text::text;

    fn mount_labelled(hide_label: bool) -> crate::HeadlessRuntime {
        let selection = binding(0i32);
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let group = picker(
                "Size",
                vec![text("Small").tag(0i32), text("Large").tag(1i32)],
                &selection,
            )
            .style(PickerStyle::Radio);
            let group = if hide_label {
                group.hide_label()
            } else {
                group
            };
            AnyView::new(vstack((group,)))
        });
        crate::HeadlessRuntime::new_for_tests(
            test_environment(),
            builder,
            320,
            200,
            MinimalTestTheme::default(),
        )
    }

    let mut labelled = mount_labelled(false);
    let result = labelled.pump_at(true, Instant::now());
    let update = result
        .tree_update
        .expect("the labelled picker must publish a tree");
    let group = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Group && node.label() == Some("Size"))
        .and_then(|(_, node)| node.bounds())
        .expect("the radio picker's group node must carry bounds");
    let first_row = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::RadioButton)
        .filter_map(|(_, node)| node.bounds())
        .reduce(|a, b| if a.y0 <= b.y0 { a } else { b })
        .expect("radio option nodes must carry bounds");
    let first_option_y = first_row.y0;
    let labelled_first_row_height = first_row.y1 - first_row.y0;
    let snapshot = result.snapshot.expect("a snapshot must be captured");
    let bands = text_ink_bands(&snapshot, group);
    assert_eq!(
        bands.len(),
        3,
        "a labelled radio picker draws three text bands — heading above two option labels, got {bands:?}"
    );
    assert!(
        bands[0].1 <= first_option_y,
        "the heading band {:?} must sit above the first option row at y0={first_option_y}",
        bands[0]
    );
    assert!(
        bands[0].0 >= group.y0 && bands[2].1 <= group.y1,
        "all text bands must lie inside the group {group:?}"
    );

    let mut hidden = mount_labelled(true);
    let result = hidden.pump_at(true, Instant::now());
    let update = result
        .tree_update
        .expect("the hidden-label picker must publish a tree");
    let group = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Group && node.label() == Some("Size"))
        .and_then(|(_, node)| node.bounds())
        .expect("the radio picker's group node must carry bounds");
    let hidden_first_row_height = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::RadioButton)
        .filter_map(|(_, node)| node.bounds())
        .reduce(|a, b| if a.y0 <= b.y0 { a } else { b })
        .map(|rect| rect.y1 - rect.y0)
        .expect("radio option nodes must carry bounds");
    assert_eq!(
        labelled_first_row_height, hidden_first_row_height,
        "the labelled first row must keep exactly the unlabelled row's height"
    );
    let snapshot = result.snapshot.expect("a snapshot must be captured");
    let bands = text_ink_bands(&snapshot, group);
    assert_eq!(
        bands.len(),
        2,
        "a hidden label draws nothing — only the two option bands remain, got {bands:?}"
    );
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Size"))
            .count(),
        1,
        "a hidden label still names the group's single node exactly once"
    );
}

/// The segmented picker's group label is drawn: its ink band sits above the
/// segment row inside the group's bounds. A hidden label draws nothing and
/// takes no space — the segment-label band alone remains — while the group
/// node keeps the label exactly once.
#[cfg(feature = "accessibility")]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn segmented_picker_draws_its_label_above_the_segment_row() {
    use accesskit::Role;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_form::picker::{PickerStyle, picker};
    use waterui_layout::stack::vstack;
    use waterui_text::text;

    fn mount_labelled(hide_label: bool) -> crate::HeadlessRuntime {
        let selection = binding(0i32);
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let group = picker(
                "Size",
                vec![text("Small").tag(0i32), text("Large").tag(1i32)],
                &selection,
            )
            .style(PickerStyle::Segmented);
            let group = if hide_label {
                group.hide_label()
            } else {
                group
            };
            AnyView::new(vstack((group,)))
        });
        crate::HeadlessRuntime::new_for_tests(
            test_environment(),
            builder,
            360,
            120,
            MinimalTestTheme::default(),
        )
    }

    let mut labelled = mount_labelled(false);
    let result = labelled.pump_at(true, Instant::now());
    let update = result
        .tree_update
        .expect("the labelled picker must publish a tree");
    let group = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Group && node.label() == Some("Size"))
        .and_then(|(_, node)| node.bounds())
        .expect("the segmented picker's group node must carry bounds");
    let segment_y = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::RadioButton)
        .filter_map(|(_, node)| node.bounds())
        .map(|rect| rect.y0)
        .reduce(f64::min)
        .expect("segment nodes must carry bounds");
    let snapshot = result.snapshot.expect("a snapshot must be captured");
    let bands = text_ink_bands(&snapshot, group);
    assert_eq!(
        bands.len(),
        2,
        "a labelled segmented picker draws two text bands — heading above the segment labels' row, got {bands:?}"
    );
    assert!(
        bands[0].1 <= segment_y,
        "the heading band {:?} must sit above the segment row at y0={segment_y}",
        bands[0]
    );
    assert!(
        bands[0].0 >= group.y0 && bands[1].1 <= group.y1,
        "both text bands must lie inside the group {group:?}"
    );

    let mut hidden = mount_labelled(true);
    let result = hidden.pump_at(true, Instant::now());
    let update = result
        .tree_update
        .expect("the hidden-label picker must publish a tree");
    let group = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Group && node.label() == Some("Size"))
        .and_then(|(_, node)| node.bounds())
        .expect("the segmented picker's group node must carry bounds");
    let hidden_segment_height = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::RadioButton)
        .filter_map(|(_, node)| node.bounds())
        .map(|rect| rect.y1 - rect.y0)
        .reduce(f64::min)
        .expect("segment nodes must carry bounds");
    let snapshot = result.snapshot.expect("a snapshot must be captured");
    let bands = text_ink_bands(&snapshot, group);
    assert_eq!(
        bands.len(),
        1,
        "a hidden label draws nothing — only the segment labels' band remains, got {bands:?}"
    );
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Size"))
            .count(),
        1,
        "a hidden label still names the group's single node exactly once"
    );

    let mut labelled = mount_labelled(false);
    let result = labelled.pump_at(true, Instant::now());
    let update = result.tree_update.expect("a tree");
    let labelled_segment_height = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::RadioButton)
        .filter_map(|(_, node)| node.bounds())
        .map(|rect| rect.y1 - rect.y0)
        .reduce(f64::min)
        .expect("segment nodes must carry bounds");
    assert_eq!(
        labelled_segment_height, hidden_segment_height,
        "the labelled segment row must keep exactly the unlabelled row's height"
    );
}

/// A signal whose `get()` writes `label` — so the write lands inside the
/// frame's leaf reads (measure/flush), after that frame's `tree.patch`, while
/// its `when` structural patch still sits in `pending`. `watch` never invokes
/// `get()`, so nothing fires early inside `set()` the way a `Map`'s `f` does.
#[derive(Clone)]
struct MidFlushWrite {
    armed: Rc<Cell<u8>>,
    label: nami::Binding<Option<waterui_core::Str>>,
}

impl nami::Signal for MidFlushWrite {
    type Output = waterui_core::Str;
    type Guard = ();

    fn snapshot(&self) -> Self::Output {
        // Fire the write once per arming: a `snapshot()` that re-set `label` on
        // every read would keep rewriting `pending` every frame — the mount
        // would never land while the churn continued.
        let armed = self.armed.replace(0);
        match armed {
            1 => self.label.set(None),
            2 => self.label.set(Some(waterui_core::Str::from("HELLO"))),
            _ => {}
        }
        waterui_core::Str::from_static("probe")
    }

    fn watch(&self, _watcher: impl Fn(nami::watcher::Context<Self::Output>) + 'static) {}
}

/// Issue #155 (`water-rs/hydrolysis`): `when` + `text` over one signal tear —
/// a `set()` flips the leaf's signal inside the same turn, while the `when`
/// structural patch (`Dynamic` pending view) lands later. Any frame presented
/// between the two points shows a mounted-but-empty subtree. Every presented
/// frame must be internally consistent: the subtree is either absent or shows
/// the leaf's current content.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn when_subtree_and_shared_signal_text_present_one_frame_state() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::graphics::color::Srgb;
    use waterui::reactive::binding;
    use waterui::widget::condition::when;
    use waterui_core::Str;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_text::text;

    // The issue's pair: `when` mounts the subtree on `is_some`; the leaf inside
    // it reads `unwrap_or_default` — both derive from the same `Binding`.
    let label = binding(Some(Str::from("HELLO")));
    // `armed` flags the write `MidFlushWrite::get` performs inside the flush;
    // `drive` only raises `patch_requested` so the pump runs a refresh frame.
    let armed = Rc::new(Cell::new(0u8));
    let drive = binding::<u32>(0u32);
    let builder = {
        let label = label.clone();
        let armed = Rc::clone(&armed);
        let drive = drive.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let leaf = label.clone();
            let label2 = label.clone();
            let label3 = label.clone();
            // Read into the view so `drive.set` schedules a refresh pump; the
            // mapped text is always empty so it adds no glyph ink to counts.
            let drive_text = drive.clone();
            AnyView::new(vstack((
                // Read first in flush order: its `get()` writes `label` while
                // this frame's `tree.patch` is already past.
                text(
                    MidFlushWrite {
                        armed: Rc::clone(&armed),
                        label: label3,
                    }
                    .computed(),
                ),
                when(label2.is_some(), move || {
                    text(leaf.map(Option::unwrap_or_default).computed())
                        .padding()
                        .background(Srgb::new_u8(0x22, 0x22, 0xEE))
                }),
                text(drive_text.map(|_| Str::from_static("")).computed()),
            )))
        })
    };
    let env = test_environment();
    let mut rt =
        crate::HeadlessRuntime::new_for_tests(env, builder, 320, 160, MinimalTestTheme::default());

    // Pill ink = saturated blue background fill; glyph ink = dark default
    // foreground. A mounted subtree paints pill pixels; a mounted subtree
    // showing the current text paints pill AND glyph pixels at the "HELLO"
    // level; a mounted-but-empty subtree paints pill pixels with only the
    // probe's glyph pixels.
    let classify = |snapshot: &crate::HeadlessSnapshot| -> (usize, usize) {
        let mut pill = 0usize;
        let mut ink = 0usize;
        let (pixels, _) = snapshot.rgba8.as_chunks::<4>();
        for px in pixels {
            let (r, g, b, a) = <(u8, u8, u8, u8)>::from(*px);
            if a > 0 && b > 170 && r < 100 && g < 100 {
                pill += 1;
            } else if a > 0 && r < 90 && g < 90 && b < 100 {
                ink += 1;
            }
        }
        (pill, ink)
    };

    let start = Instant::now();
    // Let the initial build settle; the first presented frame mounts the
    // subtree with "HELLO".
    let baseline = rt.pump_at(true, start);
    let (pill, ink) = classify(&baseline.snapshot.expect("baseline frame"));
    eprintln!("baseline: pill={pill} ink={ink}");
    assert!(pill > 0 && ink > 0, "baseline must show subtree + HELLO");

    let mut frames = Vec::new();
    // Some -> None through an ordinary between-pump `set()`.
    label.set(None);
    for i in 0..3u32 {
        let outcome = rt.pump_at(true, start + Duration::from_millis(16 * (u64::from(i) + 1)));
        if let Some(snapshot) = outcome.snapshot {
            let (pill, ink) = classify(&snapshot);
            eprintln!("plain None frame {i}: pill={pill} ink={ink}");
            frames.push(("plain", false, i, pill, ink));
        }
    }
    // Remount, then arm the mid-flush write — `MidFlushWrite::get` runs
    // `label.set(None)` inside `tree.flush`, after this frame's `tree.patch`.
    label.set(Some(Str::from("HELLO")));
    let _ = rt.pump_at(true, start + Duration::from_millis(64));
    armed.set(1);
    drive.set(1u32);
    for i in 0..3u32 {
        let outcome = rt.pump_at(true, start + Duration::from_millis(80 + 16 * u64::from(i)));
        if let Some(snapshot) = outcome.snapshot {
            let (pill, ink) = classify(&snapshot);
            eprintln!("midflush None frame {i}: pill={pill} ink={ink}");
            frames.push(("midflush", false, i, pill, ink));
        }
    }
    armed.set(2);
    drive.set(2u32);
    for i in 0..3u32 {
        let outcome = rt.pump_at(true, start + Duration::from_millis(128 + 16 * u64::from(i)));
        if let Some(snapshot) = outcome.snapshot {
            let (pill, ink) = classify(&snapshot);
            eprintln!("midflush Some frame {i}: pill={pill} ink={ink}");
            frames.push(("midflush", true, i, pill, ink));
        }
    }

    for (edge, label_is_some, i, pill, ink) in &frames {
        if *label_is_some {
            // `label` is Some: the subtree may be absent (its mount not yet
            // applied) but a mounted subtree must show HELLO — pill ink with
            // only the probe's glyph pixels is a mounted-but-empty tear.
            // Measured: probe-only ink ≈ 76, mounted-with-HELLO ink ≈ 125
            // (the Cherenkov rasterizer's glyph coverage is tighter than the
            // pre-cutover threshold of 150 assumed).
            assert!(
                *pill == 0 || *ink >= 100,
                "issue #155 torn frame on {edge}->Some (frame {i}): mounted subtree \
                 (pill={pill}) presented without the current text (ink={ink})"
            );
        } else {
            // `label` is None: any mounted subtree at all is the tear — an
            // unmount delivered mid-flush must not still present the subtree.
            assert_eq!(
                *pill, 0,
                "issue #155 torn frame on {edge}->None (frame {i}): subtree still \
                 mounted (pill={pill}, ink={ink}) after its content left"
            );
        }
    }
}
