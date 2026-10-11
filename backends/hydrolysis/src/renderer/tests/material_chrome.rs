//! `Recorder::backdrop_material` → `ChromeMaterial` member layers
//! (water-rs/waterui#1788): `draw_context` records layered, splits the
//! recording at each material into scene segments, and mounts the material
//! as a member layer between them — with its ancestry wrappers, the chrome
//! transform, the live clip and the live backdrop sample — keyed into
//! `(scope, class, canvas)` groups for `Solo` and `Shared` classes. The
//! group is built on its first visible member, rebuilt on a display-scale
//! change, and released at the commit of the first frame with no member.
//! An unregistered key panics at install; a CPU engine panics on attach
//! with a non-empty registry; a rejected source panics naming the key.

use std::rc::Rc;

use waterui_core::Binding;

use kurbo::{Affine, Rect, RoundedRect, Shape as _};
use nami::SignalExt as _;
use waterui::FilterViewExt as _;
use waterui::ViewExt as _;
use waterui::gesture::TapGesture;
use waterui::graphics::Color;
use waterui::interaction::InteractionState;
use waterui_backend_core::widget::{ButtonMetrics, InteractionStyle};
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::draw::{
    BackdropEffect, BackdropShaderSource, CaptureClass, CaptureLevels, CaptureScale, Live,
    MaterialCapture, MaterialEffect, MaterialGrouping, MaterialRegistry, MaterialShader,
    OuterExtent, ShapeData, UnionSmoothing,
};

use super::mirror::{mirror_engine, no_shader_mount};
use super::{
    ChromeDraw, ChromePlan, MinimalTestTheme, pumped_test_environment, test_renderer_with_theme,
};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::mount::backdrop::BackdropScope;
use crate::renderer::mount::target::attach_material_shaders;

/// The window, in points.
const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;
/// Device pixels per point.
const DISPLAY_SCALE: f64 = 2.0;

const GLASS_SHADER: MaterialShader = MaterialShader::new(0);
const OTHER_SHADER: MaterialShader = MaterialShader::new(1);
const GLASS: CaptureClass = CaptureClass::new(0);
const FLAT: CaptureClass = CaptureClass::new(1);
const UNION_GLASS: CaptureClass = CaptureClass::new(2);
/// `UNION_GLASS`'s logical smoothing, in points.
const UNION_SMOOTHING: f32 = 12.0;

/// A tint shader that samples the backdrop — the smallest valid
/// `backdrop_effect` for an engine registration.
const TINT_WGSL: &str = "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> { return backdrop_sample(px.p) + params[0]; }\n";

/// The chrome test plan: `GLASS` groups `Shared`, `FLAT` is `Solo`, and two
/// registered shaders.
fn glass_plan(draws: Vec<ChromeDraw>) -> ChromePlan {
    ChromePlan {
        shaders: vec![
            (GLASS_SHADER, BackdropShaderSource::wgsl(TINT_WGSL)),
            (OTHER_SHADER, BackdropShaderSource::wgsl(TINT_WGSL)),
        ],
        captures: vec![
            (
                GLASS,
                MaterialCapture {
                    scale: CaptureScale::new(0.5).expect("0.5 is a valid capture scale"),
                    levels: CaptureLevels::new(2).expect("2 is a valid level count"),
                    grouping: MaterialGrouping::Shared,
                    blend_space: cherenkov::BlendSpace::Linear,
                },
            ),
            (
                FLAT,
                MaterialCapture {
                    scale: CaptureScale::new(0.5).expect("0.5 is a valid capture scale"),
                    levels: CaptureLevels::ONE,
                    grouping: MaterialGrouping::Solo,
                    blend_space: cherenkov::BlendSpace::Linear,
                },
            ),
        ],
        draws,
        stateful_draws: None,
    }
}

/// The chrome test plan with `UNION_GLASS` — a `Union { smoothing }` class —
/// registered beside `GLASS` and `FLAT`.
fn union_plan(draws: Vec<ChromeDraw>) -> ChromePlan {
    let mut plan = glass_plan(draws);
    plan.captures.push((
        UNION_GLASS,
        MaterialCapture {
            scale: CaptureScale::new(0.5).expect("0.5 is a valid capture scale"),
            levels: CaptureLevels::ONE,
            grouping: MaterialGrouping::Union {
                smoothing: UnionSmoothing::new(UNION_SMOOTHING)
                    .expect("the test smoothing is valid"),
            },
            blend_space: cherenkov::BlendSpace::Linear,
        },
    ));
    plan
}

/// The rect every fixed-shape material in this file draws.
fn chrome_rect() -> RoundedRect {
    RoundedRect::from_rect(Rect::new(2.0, 3.0, 10.0, 11.0), 4.0)
}

/// A fixed rounded-rect material of `class` under `shader`, with `uniforms`.
fn chrome_draw(class: CaptureClass, shader: MaterialShader, uniforms: Vec<f32>) -> ChromeDraw {
    ChromeDraw {
        shape: Live::from(chrome_rect()).into_shared(),
        shader,
        capture: class,
        effect: Live::from(MaterialEffect::new(uniforms)).into_shared(),
    }
}

/// A tappable colour: the `draw_interaction_state_layer` hook the chrome
/// plan replays in, so the theme's materials record every frame.
fn tap_color(hex: &str) -> waterui_core::AnyView {
    waterui_core::AnyView::new(Color::srgb_hex(hex).gesture(TapGesture::new(), || {}))
}

fn tap_view() -> waterui_core::AnyView {
    tap_color("#3F3F46")
}

/// An environment with an `InteractionStyle` so the state-layer hook draws.
fn chrome_env() -> Environment {
    let mut env = pumped_test_environment();
    env.install(InteractionStyle::new(
        ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
        Color::srgb(0, 0, 0),
        8.0_f64,
    ));
    env
}

/// `view` pumped once at `scale` under `theme`'s chrome plan — the GPU mount.
fn rendered(
    view: impl Fn() -> waterui_core::AnyView + 'static,
    theme: MinimalTestTheme,
    scale: f64,
) -> HeadlessRuntime {
    let builder = AnyViewBuilder::<waterui_core::AnyView>::new(view);
    let mut runtime = HeadlessRuntime::new_for_tests(chrome_env(), builder, WIDTH, HEIGHT, theme)
        .with_scale_factor(scale);
    let _ = runtime.pump_snapshot();
    runtime
}

/// The chrome member layers the window mounted, in `visit` order.
fn chrome_layers(runtime: &HeadlessRuntime) -> Vec<cherenkov::LayerId> {
    use crate::renderer::mount::layers::{LayerVisitor, visit};
    struct Members(Vec<cherenkov::LayerId>);
    impl LayerVisitor for Members {
        fn node(
            &mut self,
            layers: &crate::renderer::mount::layers::NodeLayers,
            _world: kurbo::Affine,
            _alphas: &[f32],
        ) {
            self.0
                .extend(layers.chrome_members().into_iter().map(|(_, id)| id));
        }
    }
    let mut members = Members(Vec::new());
    visit(&runtime.renderer().mount_roots(), &mut members);
    members.0
}

/// The same walk over the mirror mount.
fn mirror_chrome_layers(renderer: &crate::renderer::HydrolysisRenderer) -> Vec<cherenkov::LayerId> {
    use crate::renderer::mount::layers::{LayerVisitor, visit};
    struct Members(Vec<cherenkov::LayerId>);
    impl LayerVisitor for Members {
        fn node(
            &mut self,
            layers: &crate::renderer::mount::layers::NodeLayers,
            _world: kurbo::Affine,
            _alphas: &[f32],
        ) {
            self.0
                .extend(layers.chrome_members().into_iter().map(|(_, id)| id));
        }
    }
    let mut members = Members(Vec::new());
    visit(&renderer.mount_roots(), &mut members);
    members.0
}

/// The mount's cells whose staged program records chrome members, in
/// mount order — the cells a partial commit can stage independently.
fn chrome_cells(
    renderer: &crate::renderer::HydrolysisRenderer,
) -> Vec<Rc<crate::renderer::NodeCell>> {
    fn has_chrome(items: &[crate::renderer::mount::program::Item]) -> bool {
        items.iter().any(|item| match item {
            crate::renderer::mount::program::Item::Chrome(_) => true,
            crate::renderer::mount::program::Item::Scope { items, .. } => has_chrome(items),
            _ => false,
        })
    }
    fn walk(cell: &Rc<crate::renderer::NodeCell>, out: &mut Vec<Rc<crate::renderer::NodeCell>>) {
        let retained = cell.retained();
        if retained
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|program| has_chrome(&program.items))
        {
            out.push(Rc::clone(cell));
        }
        let mut children = Vec::new();
        cell.children(&mut children);
        for child in children {
            walk(&child, out);
        }
    }
    let mut cells = Vec::new();
    for root in renderer.mount_roots() {
        walk(&root, &mut cells);
    }
    cells
}

/// The window's chrome group table on the GPU mount.
fn chrome_mount(
    runtime: &HeadlessRuntime,
) -> &crate::renderer::mount::backdrop::ChromeBackdropGroups<
    cherenkov::BackdropGroup,
    cherenkov::BackdropShader,
> {
    super::window_mount(runtime).chrome_groups()
}

/// The shared group `member` samples — `None` while it holds no membership.
fn member_group(runtime: &HeadlessRuntime, member: cherenkov::LayerId) -> cherenkov::BackdropId {
    chrome_mount(runtime)
        .chrome_group(member)
        .expect("the member holds a group membership")
        .id()
}

/// One frame through the mirror mount — `view` only matters on the first.
fn mirror_frame(renderer: &mut crate::renderer::HydrolysisRenderer, view: waterui_core::AnyView) {
    let env = chrome_env();
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        view,
        &env,
        Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    renderer.commit_mirror();
}

/// One frame through the mirror mount at `scale` device pixels per point.
fn mirror_frame_at(
    renderer: &mut crate::renderer::HydrolysisRenderer,
    view: waterui_core::AnyView,
    scale: f64,
) {
    let env = chrome_env();
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        view,
        &env,
        Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    renderer.commit_mirror_at(scale);
}

/// `view` mirrored under `theme`'s chrome plan.
fn mirrored(
    view: impl waterui_core::View,
    theme: MinimalTestTheme,
) -> crate::renderer::HydrolysisRenderer {
    let mut renderer = test_renderer_with_theme(theme);
    mirror_frame(&mut renderer, waterui_core::AnyView::new(view));
    renderer
}

/// The layer `id`'s committed node in the mirror.
fn mirrored_node(
    renderer: &crate::renderer::HydrolysisRenderer,
    id: cherenkov::LayerId,
) -> std::cell::Ref<'_, cherenkov::LayerNode> {
    renderer
        .mirror()
        .ancestry(id)
        .pop()
        .expect("the member is mounted")
}

/// Two `GLASS` members at the same bounds inside `.material_group()`.
fn grouped_pair() -> impl Fn() -> waterui_core::AnyView {
    move || {
        waterui_core::AnyView::new(
            waterui_layout::stack::zstack((tap_color("#18181B"), tap_color("#27272A")))
                .material_group(),
        )
    }
}

/// Two `GLASS` members at the same bounds with no group.
fn ungrouped_pair() -> impl Fn() -> waterui_core::AnyView {
    move || {
        waterui_core::AnyView::new(waterui_layout::stack::zstack((
            tap_color("#18181B"),
            tap_color("#27272A"),
        )))
    }
}

/// The member layers commit as children of their node's frame layer, in
/// the order the layered recording split the node's scene — a member's
/// ordinal is its material's position in the node's own paint sequence.
#[test]
fn a_chrome_member_mounts_between_the_scene_segments_it_split() {
    let theme = MinimalTestTheme {
        chrome: ChromePlan {
            draws: vec![
                chrome_draw(GLASS, GLASS_SHADER, vec![]),
                chrome_draw(FLAT, GLASS_SHADER, vec![]),
            ],
            ..glass_plan(vec![])
        },
        ..Default::default()
    };
    let renderer = mirrored(tap_view, theme);
    let members = mirror_chrome_layers(&renderer);
    assert_eq!(members.len(), 2, "each material mounts its member layer");
    let siblings = renderer.mirror().siblings(members[0]);
    assert!(
        siblings.contains(&members[1]),
        "both members are children of the node's frame layer",
    );
    let first = siblings
        .iter()
        .position(|child| *child == members[0])
        .expect("the first member is committed");
    let second = siblings
        .iter()
        .position(|child| *child == members[1])
        .expect("the second member is committed");
    assert!(
        first < second,
        "the members mount in the order their materials split the scene",
    );
    assert_eq!(
        renderer.mirror().ancestry(members[0]).len(),
        renderer.mirror().ancestry(members[1]).len(),
        "siblings share their node's frame layer",
    );
}

/// The member binds the recorded clip, the chrome transform and the live
/// backdrop sample — the group's id and the effect's shader + uniforms.
#[test]
fn a_member_binds_clip_transform_shader_and_uniforms() {
    let uniforms = vec![0.25, 0.5, 0.75, 1.0];
    let renderer = mirrored(
        tap_view(),
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, uniforms.clone())]),
            ..Default::default()
        },
    );
    let member = mirror_chrome_layers(&renderer)[0];
    let node = mirrored_node(&renderer, member);
    assert_eq!(
        node.clip.as_ref().map(ShapeData::bounds),
        Some(chrome_rect().bounding_box()),
        "the member's clip is the recorded shape",
    );
    let sample = node
        .backdrop
        .as_ref()
        .expect("the member binds a backdrop sample");
    let Some(BackdropEffect::Shader(effect)) = sample.effect() else {
        panic!("the member's effect is the shader effect it recorded")
    };
    assert_eq!(
        effect.uniforms, uniforms,
        "the member binds the live effect"
    );
    let shader = renderer
        .mirror()
        .materials()
        .shaders
        .get(&GLASS_SHADER)
        .expect("the mirror's terms carry the registered shader");
    assert_eq!(
        effect.shader,
        shader.id(),
        "the effect names the shader's engine handle",
    );
}

/// A re-flush of the same program rebinds the recorded terms onto the same
/// member layer and the same group — no new layer, group or capture.
#[test]
fn a_reflush_reuses_the_member_and_its_group() {
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
        ..Default::default()
    });
    mirror_frame(&mut renderer, tap_view());
    let before = mirror_chrome_layers(&renderer);
    let group_before = mirrored_node(&renderer, before[0])
        .backdrop
        .as_ref()
        .expect("the member binds a sample")
        .group();
    mirror_frame(&mut renderer, tap_view());
    let after = mirror_chrome_layers(&renderer);
    assert_eq!(before, after, "the re-flush reuses the member layer");
    let group_after = mirrored_node(&renderer, after[0])
        .backdrop
        .as_ref()
        .expect("the member binds a sample")
        .group();
    assert_eq!(group_before, group_after, "the re-flush reuses the group");
}

/// A signal-driven shape rebinds the member's clip with no flush — the
/// member layer and its group stay.
#[test]
fn a_signal_shape_change_needs_no_flush() {
    let shape = nami::binding(chrome_rect());
    let renderer = mirrored(
        tap_view(),
        MinimalTestTheme {
            chrome: glass_plan(vec![ChromeDraw {
                shape: Live::from(shape.map(|rect| rect)).into_shared(),
                shader: GLASS_SHADER,
                capture: GLASS,
                effect: Live::from(MaterialEffect::new(vec![])).into_shared(),
            }]),
            ..Default::default()
        },
    );
    let member = mirror_chrome_layers(&renderer)[0];
    let group_before = mirrored_node(&renderer, member)
        .backdrop
        .as_ref()
        .expect("the member binds a sample")
        .group();
    shape.set(RoundedRect::from_rect(
        Rect::new(0.0, 0.0, 40.0, 40.0),
        12.0,
    ));
    let node = mirrored_node(&renderer, member);
    assert_eq!(
        node.clip.as_ref().map(ShapeData::bounds),
        Some(Rect::new(0.0, 0.0, 40.0, 40.0)),
        "the signal's new shape reaches the member's clip with no flush",
    );
    assert_eq!(
        node.backdrop
            .as_ref()
            .expect("the member binds a sample")
            .group(),
        group_before,
        "the shape change kept the group",
    );
}

/// A remount re-lowers the cached program: the member's clip rebinds
/// from the shape signal's value now, not the value the recording stored
/// (water-rs/waterui#1788).
#[test]
fn a_remount_rebinds_the_clip_from_the_shapes_value_now() {
    let shape = nami::binding(chrome_rect());
    let mut renderer = mirrored(
        tap_view(),
        MinimalTestTheme {
            chrome: glass_plan(vec![ChromeDraw {
                shape: Live::from(shape.map(|rect| rect)).into_shared(),
                shader: GLASS_SHADER,
                capture: GLASS,
                effect: Live::from(MaterialEffect::new(vec![])).into_shared(),
            }]),
            ..Default::default()
        },
    );
    let moved = Rect::new(0.0, 0.0, 40.0, 40.0);
    shape.set(RoundedRect::from_rect(moved, 12.0));

    renderer.remount_mirror();
    renderer.commit_mirror();
    let member = mirror_chrome_layers(&renderer)[0];
    assert_eq!(
        mirrored_node(&renderer, member)
            .clip
            .as_ref()
            .map(ShapeData::bounds),
        Some(moved),
        "the remounted member's clip starts from the shape's value now",
    );
}

/// A press re-records the chrome and rebinds shape and effect onto the same
/// member and the same group — no new layer, group or capture.
#[test]
fn a_press_rebinds_the_same_member_and_group() {
    let theme = MinimalTestTheme {
        chrome: glass_plan(vec![chrome_draw(
            GLASS,
            GLASS_SHADER,
            vec![0.5, 0.0, 0.0, 0.0],
        )]),
        ..Default::default()
    };
    let builder = AnyViewBuilder::<waterui_core::AnyView>::new(tap_view);
    let mut runtime = HeadlessRuntime::new_for_tests(chrome_env(), builder, WIDTH, HEIGHT, theme)
        .with_scale_factor(DISPLAY_SCALE);
    let _ = runtime.pump_snapshot();
    let before = chrome_layers(&runtime);
    let group_before = member_group(&runtime, before[0]);
    runtime.push_input_event(InputEvent::PointerDown {
        id: 9,
        kind: PointerKind::Mouse,
        x: 80.0, // WIDTH / 2
        y: 60.0, // HEIGHT / 2
        button: PointerButton::Primary,
    });
    let _ = runtime.pump(false);
    let after = chrome_layers(&runtime);
    assert_eq!(before, after, "the press rebinds the same member layer");
    assert_eq!(
        member_group(&runtime, after[0]),
        group_before,
        "the press rebinds the same group",
    );
}

/// `Shared` members under one `.material_group()` scope share one capture.
#[test]
fn shared_members_in_one_scope_share_a_group() {
    let runtime = rendered(
        grouped_pair(),
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
    let members = chrome_layers(&runtime);
    assert_eq!(members.len(), 2, "the pair mounts a member each");
    assert_eq!(
        member_group(&runtime, members[0]),
        member_group(&runtime, members[1]),
        "the scope's members share one group",
    );
    assert!(
        matches!(
            chrome_mount(&runtime).chrome_scope(members[0]),
            Some(BackdropScope::Scoped(_)),
        ),
        "the members key under the enclosing material scope",
    );
}

/// `Solo` members never share — each is a group of its own, keyed by its
/// own layer, even inside one scope.
#[test]
fn solo_members_never_share() {
    let runtime = rendered(
        grouped_pair(),
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(FLAT, GLASS_SHADER, vec![])]),
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
    let members = chrome_layers(&runtime);
    assert_eq!(members.len(), 2);
    assert!(
        member_group(&runtime, members[0]) != member_group(&runtime, members[1]),
        "a Solo class keys each member by its own layer",
    );
    assert!(
        matches!(
            chrome_mount(&runtime).chrome_scope(members[0]),
            Some(BackdropScope::Solo(id)) if id == members[0],
        ),
        "the solo key carries the member's own layer",
    );
}

/// Members outside every scope never share — `SOLO` records a group of
/// their own even for a `Shared` class.
#[test]
fn members_outside_every_scope_never_share() {
    let runtime = rendered(
        ungrouped_pair(),
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
    let members = chrome_layers(&runtime);
    assert_eq!(members.len(), 2);
    assert_ne!(
        member_group(&runtime, members[0]),
        member_group(&runtime, members[1]),
        "scope-less members are groups of their own",
    );
}

/// The nearest enclosing scope is the key — members inside different
/// nested groups never share, even in the same class.
#[test]
fn members_in_different_scopes_never_share() {
    let view = || {
        waterui_core::AnyView::new(waterui_layout::stack::vstack((
            waterui_core::AnyView::new(tap_color("#18181B").material_group()),
            waterui_core::AnyView::new(tap_color("#27272A").material_group()),
        )))
    };
    let runtime = rendered(
        view,
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
    let members = chrome_layers(&runtime);
    assert_eq!(members.len(), 2);
    assert_ne!(
        member_group(&runtime, members[0]),
        member_group(&runtime, members[1]),
        "each member's nearest scope is a different group",
    );
}

/// Members naming different capture classes never share a group.
#[test]
fn members_of_different_classes_never_share() {
    let theme = MinimalTestTheme {
        chrome: ChromePlan {
            draws: vec![
                chrome_draw(GLASS, GLASS_SHADER, vec![]),
                chrome_draw(FLAT, GLASS_SHADER, vec![]),
            ],
            ..glass_plan(vec![])
        },
        ..Default::default()
    };
    let runtime = rendered(grouped_pair(), theme, DISPLAY_SCALE);
    let members = chrome_layers(&runtime);
    assert_eq!(members.len(), 4, "each member draws both materials");
    assert_ne!(
        member_group(&runtime, members[0]),
        member_group(&runtime, members[1]),
        "the GLASS member never shares the FLAT member's group",
    );
}

/// A second `Shared` class: in one scope its group is a different group
/// from `GLASS`'s, yet both capture beneath the same scope anchor.
const LATER_GLASS: CaptureClass = CaptureClass::new(3);

/// Every chrome group a `.material_group()` scope keys captures beneath
/// the scope's anchor layer (water-rs/waterui#2097), whatever its class:
/// a `GLASS` member and then a `LATER_GLASS` member at the same bounds
/// sample what lay behind the scope, so the later member's capture never
/// holds the earlier one. The earlier member's tint is the probe — it
/// reaches the later member's output only through the later capture.
#[test]
fn a_scopes_chrome_classes_capture_beneath_its_anchor() {
    let theme = |earlier_tint: f32| {
        let mut plan = glass_plan(vec![
            chrome_draw(GLASS, GLASS_SHADER, vec![0.0, earlier_tint, 0.0, 0.0]),
            chrome_draw(LATER_GLASS, GLASS_SHADER, vec![0.0, 0.0, 0.0, 0.0]),
        ]);
        plan.captures.push((
            LATER_GLASS,
            MaterialCapture {
                scale: CaptureScale::FULL,
                levels: CaptureLevels::ONE,
                grouping: MaterialGrouping::Shared,
                blend_space: cherenkov::BlendSpace::Linear,
            },
        ));
        MinimalTestTheme {
            chrome: plan,
            ..Default::default()
        }
    };
    let scoped = || waterui_core::AnyView::new(tap_view().material_group());
    // The later member covers the earlier one's whole shape, so a pixel
    // at the shape's centre is the later member's own output.
    let centre = |runtime: &mut HeadlessRuntime| {
        let snap = runtime.pump_snapshot().snapshot.expect("a snapshot");
        let device = snap.width / WIDTH;
        let (x, y) = (6 * device, 7 * device);
        let i = ((y * snap.width + x) * 4) as usize;
        <[u8; 4]>::try_from(&snap.rgba8[i..i + 4]).expect("one pixel")
    };

    let mut tinted = rendered(scoped, theme(0.6), DISPLAY_SCALE);
    let members = chrome_layers(&tinted);
    assert_eq!(members.len(), 2, "the view draws one member per class");
    assert_ne!(
        member_group(&tinted, members[0]),
        member_group(&tinted, members[1]),
        "the two classes are two groups",
    );
    let anchor = |member| {
        chrome_mount(&tinted)
            .chrome_group(member)
            .expect("the member holds a group membership")
            .spec()
            .anchor_layer()
    };
    assert!(
        anchor(members[0]).is_some(),
        "a scoped chrome group anchors"
    );
    assert_eq!(
        anchor(members[0]),
        anchor(members[1]),
        "both classes' groups anchor at the scope's one anchor layer",
    );
    let tinted_centre = centre(&mut tinted);
    let mut plain = rendered(scoped, theme(0.0), DISPLAY_SCALE);
    assert_eq!(
        tinted_centre,
        centre(&mut plain),
        "the earlier member's tint never enters the later member's capture",
    );

    // Outside every scope each member captures at itself, so the earlier
    // member's tint does reach the later capture: the probe is live.
    let unscoped = || tap_view();
    let mut tinted = rendered(unscoped, theme(0.6), DISPLAY_SCALE);
    assert_eq!(
        chrome_mount(&tinted)
            .chrome_group(chrome_layers(&tinted)[1])
            .map(|group| group.spec().anchor_layer()),
        Some(None)
    );
    let tinted_centre = centre(&mut tinted);
    let mut plain = rendered(unscoped, theme(0.0), DISPLAY_SCALE);
    assert_ne!(
        tinted_centre,
        centre(&mut plain),
        "a first-member capture holds the earlier member",
    );
}

/// Members under different install canvases — one inside a filtered
/// node's frame layer, one at the surface root — never share a group.
#[test]
fn members_under_different_canvases_never_share() {
    let view = || {
        waterui_core::AnyView::new(waterui_layout::stack::zstack((
            waterui_core::AnyView::new(tap_color("#18181B").material_group()),
            waterui_core::AnyView::new(tap_color("#27272A").blur(2.0f32).material_group()),
        )))
    };
    let runtime = rendered(
        view,
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
    let members = chrome_layers(&runtime);
    assert_eq!(members.len(), 2);
    assert_ne!(
        member_group(&runtime, members[0]),
        member_group(&runtime, members[1]),
        "the filtered member's canvas keys it away from the root's",
    );
}

/// A commit that carries no program re-keys a filtered node's members
/// under the canvas their lowering keyed them with — the node's own
/// filtered frame — so an idle commit neither moves them to the parent's
/// canvas nor rebuilds their group (water-rs/waterui#1788).
#[test]
fn a_no_program_commit_keeps_a_filtered_members_canvas() {
    let mut renderer = mirrored(
        tap_color("#27272A").blur(2.0f32),
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
            ..Default::default()
        },
    );
    let member = mirror_chrome_layers(&renderer)[0];
    let group = |renderer: &crate::renderer::HydrolysisRenderer| {
        *renderer
            .mirror()
            .chrome_groups()
            .chrome_group(member)
            .expect("the member holds a group membership")
    };
    let before = group(&renderer);
    assert_eq!(renderer.mirror().union_log().len(), 1);

    renderer.commit_mirror();
    assert_eq!(
        group(&renderer),
        before,
        "the member stays keyed under its filtered canvas",
    );
    assert_eq!(
        renderer.mirror().union_log().len(),
        1,
        "the idle commit built no group",
    );
    assert_eq!(renderer.mirror().chrome_groups().chrome_group_count(), 1);
}

/// A capture class's member blend space reaches its group's spec, so an
/// `SrgbEncoded` class's members composite in encoded space.
#[test]
fn a_capture_classes_blend_space_reaches_its_group() {
    const ENCODED_GLASS: CaptureClass = CaptureClass::new(4);
    let mut plan = glass_plan(vec![chrome_draw(ENCODED_GLASS, GLASS_SHADER, vec![])]);
    plan.captures.push((
        ENCODED_GLASS,
        MaterialCapture {
            scale: CaptureScale::FULL,
            levels: CaptureLevels::ONE,
            grouping: MaterialGrouping::Solo,
            blend_space: cherenkov::BlendSpace::SrgbEncoded,
        },
    ));
    let renderer = mirrored(
        tap_color("#27272A"),
        MinimalTestTheme {
            chrome: plan,
            ..Default::default()
        },
    );
    let specs = renderer.mirror().spec_log();
    assert_eq!(specs.len(), 1, "one group for the one member");
    assert_eq!(
        specs[0].member_blend_space(),
        cherenkov::BlendSpace::SrgbEncoded,
    );
}

/// A display-scale change rebuilds the group: the capture terms are in
/// capture texels, so the members re-key under a fresh group for the new
/// scale.
#[test]
fn a_display_scale_change_rebuilds_the_group() {
    let theme = MinimalTestTheme {
        chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
        ..Default::default()
    };
    let mut runtime = rendered(tap_view, theme, DISPLAY_SCALE);
    let member = chrome_layers(&runtime)[0];
    let group_before = member_group(&runtime, member);
    assert_eq!(
        chrome_mount(&runtime).chrome_display_scale(member),
        Some(DISPLAY_SCALE),
    );
    runtime.set_scale_factor(4.0);
    let _ = runtime.pump(false);
    assert_eq!(
        chrome_mount(&runtime).chrome_display_scale(member),
        Some(4.0),
        "the member re-keys under a group built for the new scale",
    );
    assert_ne!(
        member_group(&runtime, member),
        group_before,
        "the group was rebuilt for the new scale",
    );
}

/// The group is released at the commit of the first frame with no member.
#[test]
fn the_group_releases_when_its_members_leave() {
    let theme = MinimalTestTheme {
        chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
        ..Default::default()
    };
    let flag = Binding::container(true);
    let shown = flag.clone();
    let builder = AnyViewBuilder::<waterui_core::AnyView>::new(move || {
        waterui_core::AnyView::new(waterui_layout::stack::zstack((
            Color::srgb_hex("#000000"),
            waterui::widget::condition::when(shown.clone(), tap_view),
        )))
    });
    let mut runtime = HeadlessRuntime::new_for_tests(chrome_env(), builder, WIDTH, HEIGHT, theme)
        .with_scale_factor(DISPLAY_SCALE);
    let _ = runtime.pump_snapshot();
    assert_eq!(chrome_mount(&runtime).chrome_group_count(), 1);
    flag.set(false);
    let _ = runtime.pump(false);
    assert_eq!(
        chrome_mount(&runtime).chrome_group_count(),
        0,
        "the first memberless commit releases the group",
    );
}

/// A member whose ancestry is fully transparent mounts no group: the group
/// belongs to the frame's visible members.
#[test]
fn a_fully_transparent_member_mounts_no_group() {
    let view = || waterui_core::AnyView::new(tap_view().opacity(0.0));
    let runtime = rendered(
        view,
        MinimalTestTheme {
            chrome: glass_plan(vec![chrome_draw(GLASS, GLASS_SHADER, vec![])]),
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
    assert_eq!(
        chrome_mount(&runtime).chrome_group_count(),
        0,
        "no visible member: no group is built",
    );
}

/// Attaching an engine registers every shader in the registry: the
/// per-engine table the chrome members' groups bind.
#[test]
fn engine_attach_registers_each_shader() {
    let registry = {
        let mut registry = MaterialRegistry::new();
        for (key, source) in glass_plan(vec![]).shaders {
            registry.register_shader(key, source);
        }
        Rc::new(registry)
    };
    let first = mirror_engine(|engine| attach_material_shaders(engine, &registry));
    let second = mirror_engine(|engine| attach_material_shaders(engine, &registry));
    assert_eq!(first.len(), 2);
    assert_ne!(
        first[&GLASS_SHADER].id(),
        second[&GLASS_SHADER].id(),
        "each attach registers a fresh engine handle",
    );
}

/// A source the engine rejects panics at attach, naming the key.
#[test]
#[should_panic(expected = "backdrop shader MaterialShader(9) failed to register")]
fn a_rejected_source_panics_at_attach_naming_the_key() {
    let mut registry = MaterialRegistry::new();
    registry.register_shader(
        MaterialShader::new(9),
        BackdropShaderSource::wgsl("fn backdrop_effect( { this is not wgsl"),
    );
    mirror_engine(|engine| attach_material_shaders(engine, &registry));
}

/// A material naming a capture class the registry lacks panics at install,
/// naming the key.
#[test]
#[should_panic(expected = "capture class CaptureClass(7) is not registered")]
fn an_unregistered_class_panics_at_install() {
    let _runtime = rendered(
        tap_view,
        MinimalTestTheme {
            chrome: ChromePlan {
                draws: vec![chrome_draw(CaptureClass::new(7), GLASS_SHADER, vec![])],
                ..glass_plan(vec![])
            },
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
}

/// A CPU engine — a layer target with no backdrop shaders — panics on
/// attach when the theme's registry is not empty.
#[test]
#[should_panic(expected = "no backdrop shaders")]
fn a_cpu_engine_panics_on_attach_with_a_nonempty_registry() {
    let mut registry = MaterialRegistry::new();
    for (key, source) in glass_plan(vec![]).shaders {
        registry.register_shader(key, source);
    }
    let _mount = no_shader_mount(&registry);
}

/// …but an empty registry mounts on it fine — a theme with no materials
/// draws none.
#[test]
fn a_cpu_engine_accepts_an_empty_registry() {
    let _mount = no_shader_mount(&MaterialRegistry::new());
}

/// A material naming a shader the registry lacks panics at install.
#[test]
#[should_panic(expected = "backdrop shader MaterialShader(7) is not registered")]
fn an_unregistered_shader_panics_at_install() {
    let _runtime = rendered(
        tap_view,
        MinimalTestTheme {
            chrome: ChromePlan {
                draws: vec![chrome_draw(GLASS, MaterialShader::new(7), vec![])],
                ..glass_plan(vec![])
            },
            ..Default::default()
        },
        DISPLAY_SCALE,
    );
}

/// A `Union` class's members share one group under one scope — like
/// `Shared` — and the group carries the class's union field, its logical
/// smoothing converted to device pixels at the group's display scale.
#[test]
fn union_members_share_one_group_on_one_union_field() {
    let theme = MinimalTestTheme {
        chrome: union_plan(vec![chrome_draw(UNION_GLASS, GLASS_SHADER, vec![])]),
        ..Default::default()
    };
    let mut renderer = test_renderer_with_theme(theme);
    mirror_frame_at(
        &mut renderer,
        waterui_core::AnyView::new(
            waterui_layout::stack::zstack((tap_color("#18181B"), tap_color("#27272A")))
                .material_group(),
        ),
        DISPLAY_SCALE,
    );
    let members = mirror_chrome_layers(&renderer);
    assert_eq!(members.len(), 2, "the pair mounts a member each");
    let groups = renderer.mirror().chrome_groups();
    assert_eq!(
        groups.chrome_group(members[0]).copied(),
        groups.chrome_group(members[1]).copied(),
        "the scope's union members share one group",
    );
    assert!(
        matches!(
            groups.chrome_scope(members[0]),
            Some(BackdropScope::Scoped(_)),
        ),
        "a Union class keys under the enclosing material scope",
    );
    assert_eq!(
        renderer.mirror().union_log().as_slice(),
        &[Some(
            cherenkov::BackdropUnion::new(
                UNION_SMOOTHING * crate::num_cast::f64_as_f32(DISPLAY_SCALE)
            )
            .expect("in range")
        )],
        "one group, its field built from the smoothing in device pixels",
    );
}

/// A `Union` class under no scope solos — each member is a group of its
/// own, its own union field, even inside one class.
#[test]
fn union_members_outside_every_scope_never_share() {
    let theme = MinimalTestTheme {
        chrome: union_plan(vec![chrome_draw(UNION_GLASS, GLASS_SHADER, vec![])]),
        ..Default::default()
    };
    let mut renderer = test_renderer_with_theme(theme);
    mirror_frame_at(&mut renderer, ungrouped_pair()(), DISPLAY_SCALE);
    let members = mirror_chrome_layers(&renderer);
    assert_eq!(members.len(), 2);
    let groups = renderer.mirror().chrome_groups();
    assert_ne!(
        groups.chrome_group(members[0]).copied(),
        groups.chrome_group(members[1]).copied(),
        "scope-less union members are groups of their own",
    );
    assert_eq!(
        renderer.mirror().union_log().len(),
        2,
        "each solo member builds its own union field",
    );
}

/// A display-scale change rebuilds the union field's smoothing and every
/// member's outer extent and recording scale at the new scale — the
/// conversions run at the scale the group is built for.
#[test]
fn a_scale_change_rebuilds_union_smoothing_and_member_outer() {
    let theme = MinimalTestTheme {
        chrome: union_plan(vec![ChromeDraw {
            effect: Live::from(
                MaterialEffect::new(vec![0.5]).outer(OuterExtent::new(3.0).expect("non-negative")),
            )
            .into_shared(),
            ..chrome_draw(UNION_GLASS, GLASS_SHADER, vec![])
        }]),
        ..Default::default()
    };
    let mut renderer = test_renderer_with_theme(theme);
    mirror_frame_at(&mut renderer, tap_view(), DISPLAY_SCALE);
    let member = mirror_chrome_layers(&renderer)[0];
    let sample = mirrored_node(&renderer, member)
        .backdrop
        .clone()
        .expect("the member binds a sample");
    assert_eq!(
        sample.outer_extent(),
        cherenkov::BackdropOuter::new(6.0).expect("in range"),
        "3 logical px at display scale 2 binds as 6 device px",
    );
    assert_eq!(
        sample.recording_scale(),
        cherenkov::RecordingScale::new(2.0).expect("in range"),
        "the member's shader reads the display scale as `px.scale`",
    );
    drop(sample);

    mirror_frame_at(&mut renderer, tap_view(), 4.0);
    assert_eq!(
        mirror_chrome_layers(&renderer)[0],
        member,
        "the re-keyed member reuses its layer",
    );
    assert_eq!(
        renderer.mirror().union_log().as_slice(),
        &[
            Some(cherenkov::BackdropUnion::new(24.0).expect("in range")),
            Some(cherenkov::BackdropUnion::new(48.0).expect("in range")),
        ],
        "the rebuild re-converts the smoothing at the new scale",
    );
    let sample = mirrored_node(&renderer, member)
        .backdrop
        .clone()
        .expect("the member rebinds its sample");
    assert_eq!(
        sample.outer_extent(),
        cherenkov::BackdropOuter::new(12.0).expect("in range"),
        "the member's outer extent is re-bound at the new scale",
    );
    assert_eq!(
        sample.recording_scale(),
        cherenkov::RecordingScale::new(4.0).expect("in range"),
        "the member's recording scale is re-bound at the new scale",
    );
}

/// A union smoothing that is not finite or not positive is rejected by
/// `UnionSmoothing` itself — an invalid value cannot reach a
/// `MaterialGrouping::Union` registration.
#[test]
fn an_invalid_union_smoothing_is_rejected_by_the_logical_type() {
    assert_eq!(
        UnionSmoothing::new(0.0),
        Err(cherenkov_record::UnionSmoothingError::OutOfRange)
    );
    assert_eq!(
        UnionSmoothing::new(f32::NAN),
        Err(cherenkov_record::UnionSmoothingError::NonFinite)
    );
    assert_eq!(
        UnionSmoothing::new(f32::INFINITY),
        Err(cherenkov_record::UnionSmoothingError::NonFinite)
    );
}

/// A valid logical union smoothing whose device-pixel conversion fails —
/// here, overflowing `f32` — panics at group build inside the mounted
/// commit, naming the class and the scale. The error is surfaced, never
/// clamped.
#[test]
#[should_panic(expected = "invalid at display scale 4: the union smoothing is not finite")]
fn a_union_smoothing_that_overflows_device_pixels_panics_at_group_build() {
    let theme = MinimalTestTheme {
        chrome: {
            let mut plan = glass_plan(vec![chrome_draw(UNION_GLASS, GLASS_SHADER, vec![])]);
            plan.captures.push((
                UNION_GLASS,
                MaterialCapture {
                    scale: CaptureScale::FULL,
                    levels: CaptureLevels::ONE,
                    grouping: MaterialGrouping::Union {
                        smoothing: UnionSmoothing::new(1e38).expect("finite and positive"),
                    },
                    blend_space: cherenkov::BlendSpace::Linear,
                },
            ));
            plan
        },
        ..Default::default()
    };
    let mut renderer = test_renderer_with_theme(theme);
    mirror_frame_at(&mut renderer, tap_view(), 4.0);
}

/// A non-finite or negative logical outer extent is rejected by
/// `OuterExtent` itself — the same rule [`BackdropOuter`] enforces on the
/// device-pixel value — so `MaterialEffect::outer` only takes valid ones.
#[test]
fn an_invalid_outer_extent_is_rejected_by_the_logical_type() {
    assert_eq!(
        OuterExtent::new(-2.0),
        Err(cherenkov_record::OuterExtentError::Negative)
    );
    assert_eq!(
        OuterExtent::new(f32::NAN),
        Err(cherenkov_record::OuterExtentError::NonFinite)
    );
    assert_eq!(
        OuterExtent::new(f32::INFINITY),
        Err(cherenkov_record::OuterExtentError::NonFinite)
    );
}

/// A finite logical outer extent whose device-pixel conversion overflows
/// `f32` panics at the member's bind inside the mounted commit, naming
/// the extent and the scale. The error is surfaced, never clamped.
#[test]
#[should_panic(
    expected = "outer extent 340282350000000000000000000000000000000 is invalid at display scale 2: the outer extent is not finite"
)]
fn an_outer_extent_that_overflows_device_pixels_panics_at_bind() {
    let theme = MinimalTestTheme {
        chrome: glass_plan(vec![ChromeDraw {
            effect: Live::from(
                MaterialEffect::new(vec![]).outer(OuterExtent::new(f32::MAX).expect("finite")),
            )
            .into_shared(),
            ..chrome_draw(GLASS, GLASS_SHADER, vec![])
        }]),
        ..Default::default()
    };
    let mut renderer = test_renderer_with_theme(theme);
    mirror_frame_at(&mut renderer, tap_view(), DISPLAY_SCALE);
}

/// Two members of one shared group carry different uniforms; a display-scale
/// Two members of one shared group carry different uniforms; a commit at
/// the new display scale that re-lowers neither member — the members of
/// one group live in the one program cell their `record_layered` wrote,
/// so a no-new-program commit is the partial commit where neither
/// member's fresh payload reaches `join` — rebuilds the group and
/// rebinds each member's own stored payload, never the joiner's
/// (water-rs/waterui#1788).
#[test]
fn a_scale_rebuild_keeps_each_members_own_effect() {
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        chrome: glass_plan(vec![
            chrome_draw(GLASS, GLASS_SHADER, vec![1.0, 0.0]),
            chrome_draw(GLASS, GLASS_SHADER, vec![0.0, 1.0]),
        ]),
        ..Default::default()
    });
    mirror_frame_at(&mut renderer, grouped_pair()(), DISPLAY_SCALE);
    let members = mirror_chrome_layers(&renderer);
    assert_eq!(members.len(), 4, "the pair mounts both draws per member");

    // Re-capture stages a fresh program on every cell; unstaging the
    // chrome cell's leaves it COMMIT-marked with no program, so the
    // new-scale commit joins the members without re-lowering any — the
    // rebuild itself is what rebinds each member.
    let env = chrome_env();
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        grouped_pair()(),
        &env,
        Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    let cells = chrome_cells(&renderer);
    assert_eq!(cells.len(), 1, "the group's members share one program cell");
    *cells[0].retained().pending.borrow_mut() = None;
    // The mirror's group id is stable across rebuilds, so the union log
    // — one entry per group `create` — is what proves the rebuild ran:
    // the install's one entry, plus one from the scale-change rebuild.
    assert_eq!(renderer.mirror().union_log().len(), 1);
    renderer.commit_mirror_at(DISPLAY_SCALE * 2.0);
    assert_eq!(
        renderer.mirror().union_log().len(),
        2,
        "the scale change rebuilt the shared group",
    );

    let rebuilt = mirror_chrome_layers(&renderer);
    let uniforms: Vec<Vec<f32>> = rebuilt
        .iter()
        .map(|member| {
            let node = mirrored_node(&renderer, *member);
            let sample = node.backdrop.as_ref().expect("the member binds a sample");
            let Some(BackdropEffect::Shader(effect)) = sample.effect() else {
                panic!("the member's effect is its shader effect")
            };
            effect.uniforms.clone()
        })
        .collect();
    assert_eq!(
        uniforms,
        [
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0]
        ],
        "each member kept its own uniforms across the rebuild",
    );
}

/// A display-scale rebuild re-points members whose cells the commit does
/// not have open: two cells under one `.material_group()` scope and one
/// canvas, each holding both of the plan's members. A commit at a new
/// scale with no program staged anywhere joins no member with a fresh
/// payload, so the first joiner's rebuild is the only bind the other
/// cell's members get — and each must keep its own uniforms, never the
/// joiner's (water-rs/waterui#1788).
#[test]
fn a_scale_rebuild_rebinds_members_of_other_cells_with_their_own_effect() {
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        chrome: glass_plan(vec![
            chrome_draw(GLASS, GLASS_SHADER, vec![1.0, 0.0]),
            chrome_draw(GLASS, GLASS_SHADER, vec![0.0, 1.0]),
        ]),
        ..Default::default()
    });
    let view = || {
        waterui_core::AnyView::new(
            waterui_layout::stack::zstack((
                waterui_core::AnyView::new(tap_color("#18181B").opacity(0.5)),
                waterui_core::AnyView::new(tap_color("#27272A").opacity(0.5)),
            ))
            .material_group(),
        )
    };
    let env = chrome_env();
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        view(),
        &env,
        Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    assert_eq!(
        chrome_cells(&renderer).len(),
        2,
        "each member records into a cell of its own",
    );
    renderer.commit_mirror_at(DISPLAY_SCALE);
    let members = mirror_chrome_layers(&renderer);
    assert_eq!(members.len(), 4, "each cell mounts both draws");
    let groups = renderer.mirror().chrome_groups();
    assert_eq!(
        groups.chrome_group_count(),
        1,
        "one scope, one class, one canvas: one group",
    );
    assert_eq!(renderer.mirror().union_log().len(), 1);
    let group = |renderer: &crate::renderer::HydrolysisRenderer| {
        *renderer
            .mirror()
            .chrome_groups()
            .chrome_group(members[0])
            .expect("the member holds a group membership")
    };
    let before = group(&renderer);

    // Nothing is staged: every cell commits with no program.
    renderer.commit_mirror_at(DISPLAY_SCALE * 2.0);
    assert_eq!(
        renderer.mirror().union_log().len(),
        2,
        "the scale change rebuilt the shared group",
    );
    let rebuilt = group(&renderer);
    assert_ne!(rebuilt, before, "the rebuilt group has a new id");
    let (groups, uniforms): (Vec<cherenkov::BackdropId>, Vec<Vec<f32>>) =
        mirror_chrome_layers(&renderer)
            .iter()
            .map(|member| {
                let node = mirrored_node(&renderer, *member);
                let sample = node.backdrop.as_ref().expect("the member binds a sample");
                let Some(BackdropEffect::Shader(effect)) = sample.effect() else {
                    panic!("the member's effect is its shader effect")
                };
                (sample.group(), effect.uniforms.clone())
            })
            .unzip();
    assert_eq!(
        groups, [rebuilt; 4],
        "every member samples the rebuilt group, none the released one",
    );
    assert_eq!(
        uniforms,
        [
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0]
        ],
        "each member kept its own uniforms across the rebuild",
    );
}

/// A signal-driven effect changed after recording: the next commit with
/// no new program, and a commit at a new display scale, must both leave
/// the member bound to the signal's current value — the rebind reads a
/// fresh snapshot (`Live::rebound`), never the recording's
/// (water-rs/waterui#1788).
#[test]
fn a_rebound_member_tracks_the_signals_value_after_recording() {
    let signal = nami::binding(1.0_f32);
    let effect = Live::from(signal.clone())
        .map(|v: f32| MaterialEffect::new(vec![v]))
        .into_shared();
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        chrome: glass_plan(vec![ChromeDraw {
            effect,
            ..chrome_draw(GLASS, GLASS_SHADER, vec![])
        }]),
        ..Default::default()
    });
    mirror_frame(&mut renderer, tap_view());
    let member = mirror_chrome_layers(&renderer)[0];
    let bound = |renderer: &crate::renderer::HydrolysisRenderer| {
        let node = mirrored_node(renderer, member);
        let Some(BackdropEffect::Shader(effect)) = node
            .backdrop
            .as_ref()
            .expect("the member binds a sample")
            .effect()
        else {
            panic!("the member's effect is its shader effect")
        };
        effect.uniforms.clone()
    };
    assert_eq!(bound(&renderer), [1.0]);

    // The change after recording: already delivered once — a rebind must
    // not restart from 1.
    signal.set(2.0);
    assert_eq!(bound(&renderer), [2.0], "the live binding landed it");

    // A commit with no new program: membership is unchanged, no rebind —
    // and the kept binding still reads 2.
    renderer.commit_mirror();
    assert_eq!(
        bound(&renderer),
        [2.0],
        "an unchanged commit keeps the current value",
    );

    // A display-scale rebuild: the member's stored payload rebinds from
    // the signal's value now.
    renderer.commit_mirror_at(DISPLAY_SCALE * 2.0);
    assert_eq!(
        bound(&renderer),
        [2.0],
        "the rebuild's rebind starts from the signal's value now",
    );
}

/// A theme whose uniforms depend on `WidgetInteractionState`: a press
/// re-records the chrome with the pressed uniforms, and the member's
/// rebound sample carries them (water-rs/waterui#1788). The mirror mount
/// has no runtime window to dispatch an `InputEvent` through, so the test
/// calls the renderer entry that dispatch calls for
/// `InputEvent::PointerDown`; `a_press_rebinds_the_same_member_and_group`
/// covers the dispatched event on a runtime.
#[test]
fn a_press_rebinds_the_members_new_effect() {
    let mut renderer = test_renderer_with_theme(MinimalTestTheme {
        chrome: ChromePlan {
            stateful_draws: Some(std::rc::Rc::new(|state| {
                vec![chrome_draw(
                    GLASS,
                    GLASS_SHADER,
                    vec![if state.state.contains(InteractionState::PRESSED) {
                        1.0
                    } else {
                        0.0
                    }],
                )]
            })),
            ..glass_plan(vec![])
        },
        ..Default::default()
    });
    mirror_frame(&mut renderer, tap_view());
    let member = mirror_chrome_layers(&renderer)[0];
    let bound = |renderer: &crate::renderer::HydrolysisRenderer| {
        let node = mirrored_node(renderer, member);
        let Some(BackdropEffect::Shader(effect)) = node
            .backdrop
            .as_ref()
            .expect("the member binds a sample")
            .effect()
        else {
            panic!("the member's effect is its shader effect")
        };
        effect.uniforms.clone()
    };
    assert_eq!(bound(&renderer), [0.0]);

    // What the window's dispatch calls for an `InputEvent::PointerDown`
    // with these fields.
    renderer.handle_pointer_down_with_source(
        9,
        PointerKind::Mouse,
        80.0,
        60.0,
        PointerButton::Primary,
        &chrome_env(),
    );
    mirror_frame(&mut renderer, tap_view());
    assert_eq!(
        bound(&renderer),
        [1.0],
        "the press re-record rebound the member's new effect",
    );
    assert_eq!(
        mirror_chrome_layers(&renderer),
        [member],
        "the press re-record kept the member layer",
    );
}
