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
use waterui_backend_core::widget::{ButtonMetrics, InteractionStyle};
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::draw::{
    BackdropEffect, BackdropShaderSource, CaptureClass, CaptureLevels, CaptureScale, Live,
    MaterialCapture, MaterialEffect, MaterialGrouping, MaterialRegistry, MaterialShader, ShapeData,
};

use super::mirror::{mirror_engine, no_shader_mount};
use super::{
    ChromeDraw, ChromePlan, MinimalTestTheme, pumped_test_environment, test_renderer_with_theme,
};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::mount::backdrop::ChromeScope;
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
                },
            ),
            (
                FLAT,
                MaterialCapture {
                    scale: CaptureScale::new(0.5).expect("0.5 is a valid capture scale"),
                    levels: CaptureLevels::ONE,
                    grouping: MaterialGrouping::Solo,
                },
            ),
        ],
        draws,
    }
}

/// The rect every fixed-shape material in this file draws.
fn chrome_rect() -> RoundedRect {
    RoundedRect::from_rect(Rect::new(2.0, 3.0, 10.0, 11.0), 4.0)
}

/// A fixed rounded-rect material of `class` under `shader`, with `uniforms`.
fn chrome_draw(class: CaptureClass, shader: MaterialShader, uniforms: Vec<f32>) -> ChromeDraw {
    ChromeDraw {
        shape: Live::from(chrome_rect()),
        shader,
        capture: class,
        effect: Live::from(MaterialEffect::new(uniforms)),
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

/// The window's chrome group table on the GPU mount.
fn chrome_mount(
    runtime: &HeadlessRuntime,
) -> &crate::renderer::mount::backdrop::ChromeBackdropGroups<cherenkov::BackdropGroup> {
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
                shape: Live::from(shape.map(|rect| rect)),
                shader: GLASS_SHADER,
                capture: GLASS,
                effect: Live::from(MaterialEffect::new(vec![])),
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
            Some(ChromeScope::Scoped(_)),
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
            Some(ChromeScope::Solo(id)) if id == members[0],
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
