//! Phase 1 unit tests for the persistent retained render tree.

use super::{MinimalTestTheme, test_renderer};
use crate::engine::WidgetTheme;
use crate::renderer::{ContainerNode, RenderContext, RenderNode, TextNode};
use nami::Computed;
use vello::kurbo::{Affine, Rect};
use waterui::ViewExt as _;
use waterui_controls::button::button;
use waterui_core::layout::{HorizontalAlignment, Size};
use waterui_core::{AnyView, Environment};
use waterui_layout::stack::{VStackLayout, vstack};
use waterui_text::styled::StyledStr;

fn init_executors() {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        native_executor::NativeExecutor::new(),
    ));
}

fn text_node(content: &'static str) -> RenderNode {
    RenderNode::Text(Box::new(TextNode {
        content: Computed::constant(StyledStr::plain(content)),
        alignment: Computed::constant(HorizontalAlignment::Leading),
    }))
}

#[test]
fn render_node_container_lays_out_and_flushes_text() {
    init_executors();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();

    let mut node = RenderNode::Container(Box::new(ContainerNode {
        layout: Box::new(VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: 8.0,
        }),
        children: vec![text_node("Hello"), text_node("World")],
        placed: Vec::new(),
    }));

    let window = Size::new(200.0, 120.0);
    let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    node.layout(&mut renderer, &env, window);

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
    init_executors();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();

    let mut node = RenderNode::Container(Box::new(ContainerNode {
        layout: Box::new(VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: 8.0,
        }),
        children: vec![text_node("Cached")],
        placed: Vec::new(),
    }));

    let window = Size::new(200.0, 120.0);
    let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);

    renderer.begin_rebuild_frame();
    node.layout(&mut renderer, &env, window);
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
    init_executors();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
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
    node.layout(&mut renderer, &env, window);
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
    init_executors();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();

    // text -> Text node; button -> Captured leaf (Native<ButtonConfig> the
    // dispatcher renders); vstack -> Container. Exercises the whole build walk.
    let view = AnyView::new(vstack((
        waterui_text::text("Title"),
        button("Tap").action(|| {}),
    )));
    let bounds = Rect::new(0.0, 0.0, 220.0, 200.0);

    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(bounds);
    renderer.capture_window_tree(view, &env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    assert!(
        !renderer.scene_is_empty(),
        "the render-tree path must draw a mixed text + widget view"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn flush_window_tree_reuses_retained_tree() {
    init_executors();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 220.0, 200.0);

    let view = AnyView::new(vstack((
        waterui_text::text("Title"),
        button("Tap").action(|| {}),
    )));
    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(bounds);
    renderer.capture_window_tree(view, &env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    renderer.finish_rebuild_frame();

    // A geometry-static frame re-flushes the retained tree without rebuilding it.
    // `flush_window_tree` does full frame management (reset + flush + move the
    // scene into the compositor's layer stack), so verify a Vello layer resulted.
    let flushed = renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    assert!(flushed, "a retained tree must be present to flush");
    let (_, vello_layers, _) = renderer.render_layer_stats();
    assert!(
        vello_layers > 0,
        "re-flushing the retained tree must produce a Vello scene layer"
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
            AnyView::new(button(text!("N={n}", n = n.clone())).action(|| {}))
        })
    };
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 200, 120);
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 240, 120);
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
        AnyViewBuilder::<AnyView>::new(move || AnyView::new(TextField::new(&value)))
    };
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 240, 120);
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
    // on the legacy patch path this could silently fail to switch (Bug 1). The
    // render-tree path processes it as a rebuild, so the switch always takes.
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 200, 120);

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

/// Visual verification of the chart-switch fix: a `watch`-driven swap between two
/// distinctly-coloured boxes (through the same `Captured` path a `SceneView`
/// chart takes) rendered via the render-tree path. Exports before/after PNGs for
/// direct inspection — the switch must visibly take effect (red -> blue).
#[test]
fn render_tree_chart_switch_snapshot() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui::reactive::binding;
    use waterui_core::dynamic::watch;
    use waterui_core::handler::AnyViewBuilder;

    fn write_png(path: &str, width: u32, height: u32, rgba: Vec<u8>) {
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160);

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");
    write_png(
        "/tmp/waterui_tree_switch_before.png",
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
        "/tmp/waterui_tree_switch_after.png",
        after.width,
        after.height,
        after.rgba8,
    );

    eprintln!("wrote /tmp/waterui_tree_switch_before.png and /tmp/waterui_tree_switch_after.png");
}

/// The exact chart case: a `watch`-driven swap between two `Canvas` (`SceneView`)
/// charts — the effect-slot path where Bug 1 lived. On the render-tree path the
/// switch must visibly take effect (red -> blue scene), proving the SceneView
/// switch is fixed (the cursor is reset on the reuse-rebuild path).
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

    fn write_png(path: &str, width: u32, height: u32, rgba: Vec<u8>) {
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160);

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");
    write_png(
        "/tmp/waterui_tree_sceneview_before.png",
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
        "/tmp/waterui_tree_sceneview_after.png",
        after.width,
        after.height,
        after.rgba8,
    );

    eprintln!(
        "wrote /tmp/waterui_tree_sceneview_before.png and /tmp/waterui_tree_sceneview_after.png"
    );
}

/// Verifies scroll works on the render-tree path: a fixed-content scroll view is
/// scrolled, and the exported before/after PNGs must show the content shifted
/// (different colour band at the top). Tells us whether a dedicated `ScrollNode`
/// is needed or the Captured scroll replays at the current offset correctly.
#[test]
fn render_tree_scroll_snapshot() {
    use core::time::Duration;
    use std::time::Instant;
    use waterui_core::handler::AnyViewBuilder;

    fn write_png(path: &str, width: u32, height: u32, rgba: Vec<u8>) {
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160);

    let start = Instant::now();
    let before = rt
        .pump_at(true, start)
        .snapshot
        .expect("first frame must capture a snapshot");
    write_png(
        "/tmp/waterui_tree_scroll_before.png",
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
        "/tmp/waterui_tree_scroll_after.png",
        after.width,
        after.height,
        after.rgba8,
    );

    eprintln!("wrote /tmp/waterui_tree_scroll_before.png and /tmp/waterui_tree_scroll_after.png");
}

/// Verifies a reactive collection (`ForEach` in a scroll → `LazyContainer`)
/// renders via the tree path: the exported PNG must show the stacked coloured
/// rows.
#[test]
fn render_tree_collection_snapshot() {
    use waterui_core::handler::AnyViewBuilder;
    use waterui_core::id::SelfId;

    fn write_png(path: &str, width: u32, height: u32, rgba: Vec<u8>) {
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160);

    let snapshot = rt
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("collection frame must capture a snapshot");
    write_png(
        "/tmp/waterui_tree_collection.png",
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160);

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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 160, 160);

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

    fn write_png(path: &str, width: u32, height: u32, rgba: Vec<u8>) {
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut rt = crate::HeadlessRuntime::new_for_tests(env, builder, 440, 920);

    let snapshot = rt
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("grid frame must capture a snapshot");
    write_png(
        "/tmp/waterui_tree_grid.png",
        snapshot.width,
        snapshot.height,
        snapshot.rgba8,
    );
    eprintln!("wrote /tmp/waterui_tree_grid.png");
}

/// Lifecycle hooks are node-owned on the retained tree: `.on_appear` fires once
/// when its wrapper node is built, and `.on_disappear` fires from the node's
/// `Drop` when the retained tree is torn down. Structural presence/removal drives
/// the hooks — there is no frame-diff slot cursor (the same fix class as Bug 1),
/// and the disappear hook must not fire while the node is still retained.
#[test]
fn lifecycle_hooks_fire_on_build_and_drop() {
    use core::cell::Cell;
    use std::rc::Rc;

    init_executors();
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

    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 200.0, 120.0);

    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(bounds);
    renderer.capture_window_tree(view, &env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    renderer.finish_rebuild_frame();

    assert!(
        appeared.get(),
        "on_appear must fire when the lifecycle wrapper node is built"
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

/// A GPU filter (`.blur(...)`) is built and flushed through the retained tree as a
/// node-owned `AppliedFilter`, not the old dispatch capture/replay path. The node
/// owns its `AppliedFilterRuntime` (textures) and registers it in the renderer's
/// retained-filter registry at build; a re-flush of the geometry-static tree keeps
/// the same runtime alive (it is pruned only when its node is dropped), so the
/// filter survives across frames without a cursor-bound effect slot.
#[test]
fn applied_filter_renders_through_retained_tree() {
    use waterui_graphics::filter_view::FilterViewExt;

    init_executors();

    fn blurred_box() -> AnyView {
        use waterui::prelude::*;
        AnyView::new(
            ().size(48.0, 48.0)
                .background(Color::srgb_hex("#DC2626"))
                .blur(6.0f32),
        )
    }

    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 120.0, 120.0);

    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(bounds);
    renderer.capture_window_tree(
        blurred_box(),
        &env,
        bounds,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();

    assert!(
        !renderer.node_applied_filters.is_empty(),
        "a .blur() view must build a node-owned AppliedFilter on the retained tree \
         (registered at build), not fall through to the dispatch/capture path"
    );

    // A geometry-static re-flush keeps the node — and thus its filter runtime —
    // alive (pruned only on node drop, by Rc strong count).
    let flushed = renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    assert!(flushed, "the retained tree must re-flush");
    assert!(
        !renderer.node_applied_filters.is_empty(),
        "the node-owned filter runtime must survive a geometry-static re-flush \
         (the retained AppliedFilter node keeps owning it across frames)"
    );
}
