//! `.background(Glass…)` realized through the widget theme
//! (water-rs/waterui#4495): the theme's [`GlassLook`] becomes a backdrop
//! material member behind the content, in the nearest enclosing
//! `.material_group()` scope, on the glass's outline over the content's
//! frame — live as the frame changes — and takes no input.

use core::time::Duration;
use std::cell::Cell;
use std::rc::Rc;

use cherenkov_record::{BackdropEffect, MaterialEffect, ShapeData};
use kurbo::{Affine, Rect};
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::background::Glass;
use waterui::graphics::Color;
use waterui::interaction::InteractionState;
use waterui::shape::{Ellipse, PathCommand, RoundedRectangle, Shape};
use waterui_controls::button::button;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment};
use waterui_layout::stack::zstack;

use super::material_chrome::{
    GLASS, GLASS_SHADER, HEIGHT, WIDTH, chrome_layers, chrome_mount, glass_plan, member_group,
    mirror_chrome_layers, mirrored_node,
};
use super::{MinimalTestTheme, pumped_test_environment, test_renderer_with_theme};
use crate::HeadlessRuntime;
use crate::platform::{PointerButton, PointerKind};
use crate::renderer::HydrolysisRenderer;
use crate::renderer::mount::backdrop::BackdropScope;
use crate::renderer::mount::layers::MountStats;
use crate::text::ParleyEngine;
use crate::theme::{Backdrop, Driven, GlassLook, Inputs, Motion};

/// Device pixels per point of the rendered runtimes.
const DISPLAY_SCALE: f64 = 2.0;
/// The uniforms of the theme's resting effect.
const UNIFORMS: [f32; 2] = [0.25, 0.5];
const POINTER: u64 = 7;

/// The glass look of a theme: the registered shader and class, `effect`.
fn look(effect: Driven<MaterialEffect>) -> GlassLook {
    GlassLook::new(
        Backdrop::new(GLASS_SHADER, GLASS, effect),
        Motion::uniform(Animation::linear(Duration::from_millis(15))),
    )
}

/// A theme whose glass is `look`'s material under `effect`.
fn glass_theme(effect: Driven<MaterialEffect>) -> MinimalTestTheme {
    MinimalTestTheme {
        chrome: glass_plan(vec![]),
        ..MinimalTestTheme::default()
    }
    .with_glass_background(move |_, _| Some(look(effect.clone())))
}

fn resting() -> Driven<MaterialEffect> {
    Driven::constant(MaterialEffect::new(UNIFORMS.to_vec()))
}

/// Content that fills its proposal, with `glass` behind it.
fn glass_view(glass: Glass) -> AnyView {
    AnyView::new(Color::srgb_hex("#3F3F46").background(glass))
}

/// One frame of `view` through the mirror mount at `window`.
fn frame(
    renderer: &mut HydrolysisRenderer<ParleyEngine>,
    env: &Environment,
    view: AnyView,
    window: Rect,
) -> MountStats {
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(view, env, window, Affine::IDENTITY, Affine::IDENTITY);
    renderer.finish_rebuild_frame();
    renderer.commit_mirror()
}

/// The window the mirror frames lay out in.
fn window() -> Rect {
    Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT))
}

/// The only member the glass mounted, and the clip it binds.
fn member_clip(renderer: &HydrolysisRenderer<ParleyEngine>) -> ShapeData {
    let [member] = <[_; 1]>::try_from(mirror_chrome_layers(renderer))
        .expect("the glass mounts exactly one member");
    mirrored_node(renderer, member)
        .clip
        .clone()
        .expect("the member binds the glass's outline as its clip")
}

/// The uniforms the only member's effect binds.
fn member_uniforms(renderer: &HydrolysisRenderer<ParleyEngine>) -> Vec<f32> {
    let [member] = <[_; 1]>::try_from(mirror_chrome_layers(renderer))
        .expect("the glass mounts exactly one member");
    let node = mirrored_node(renderer, member);
    let Some(BackdropEffect::Shader(effect)) = node
        .backdrop
        .as_ref()
        .expect("the member binds a backdrop sample")
        .effect()
    else {
        panic!("the member's effect is its shader effect")
    };
    effect.uniforms.clone()
}

/// A theme whose glass draws one member behind the content, with the
/// theme's shader, class and effect; a theme that answers `None` records no
/// member and draws the content as it would without the background.
#[test]
fn a_glass_background_records_the_themes_look_behind_the_content() {
    let env = pumped_test_environment();
    let mut renderer = test_renderer_with_theme(glass_theme(resting()));
    frame(&mut renderer, &env, glass_view(Glass::regular()), window());
    let [member] = <[_; 1]>::try_from(mirror_chrome_layers(&renderer))
        .expect("the glass mounts exactly one member");
    assert_eq!(member_uniforms(&renderer), UNIFORMS, "the theme's effect");
    let shader = renderer
        .mirror()
        .materials()
        .shaders
        .get(&GLASS_SHADER)
        .expect("the theme registered its shader");
    let node = mirrored_node(&renderer, member);
    let Some(BackdropEffect::Shader(effect)) = node
        .backdrop
        .as_ref()
        .expect("the member binds a backdrop sample")
        .effect()
    else {
        panic!("the member's effect is its shader effect")
    };
    assert_eq!(effect.shader, shader.id(), "the theme's shader");
    drop(node);

    let without_glass = |view: AnyView, theme: MinimalTestTheme| {
        let mut renderer = test_renderer_with_theme(theme);
        let stats = frame(&mut renderer, &env, view, window());
        (stats, mirror_chrome_layers(&renderer))
    };
    let (plain_stats, _) = without_glass(
        AnyView::new(Color::srgb_hex("#3F3F46")),
        MinimalTestTheme::default(),
    );
    let (default_stats, default_members) = without_glass(
        glass_view(Glass::regular()),
        MinimalTestTheme {
            chrome: glass_plan(vec![]),
            ..MinimalTestTheme::default()
        },
    );
    assert!(
        default_members.is_empty(),
        "the default theme draws no glass"
    );
    assert_eq!(
        default_stats.created, plain_stats.created,
        "the content renders as it does without the background"
    );
    assert_eq!(default_stats.scene_layers, plain_stats.scene_layers);
}

/// The members of two glass backgrounds under one `.material_group()` share
/// one group; outside every scope each is a group of its own.
#[test]
fn glass_backgrounds_share_a_group_only_inside_a_material_group() {
    let rendered = |view: AnyView| {
        let view = Rc::new(std::cell::RefCell::new(Some(view)));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the fixture view is built once")
        });
        let mut runtime = HeadlessRuntime::new_for_tests(
            pumped_test_environment(),
            builder,
            WIDTH,
            HEIGHT,
            glass_theme(resting()),
        )
        .with_scale_factor(DISPLAY_SCALE);
        let _ = runtime.pump_snapshot();
        runtime
    };
    let pair = || zstack((glass_view(Glass::regular()), glass_view(Glass::regular())));

    let grouped = rendered(AnyView::new(pair().material_group()));
    let members = chrome_layers(&grouped);
    assert_eq!(members.len(), 2, "each glass mounts a member");
    assert_eq!(
        member_group(&grouped, members[0]),
        member_group(&grouped, members[1]),
        "the scope's members share one group",
    );
    assert!(
        matches!(
            chrome_mount(&grouped).chrome_scope(members[0]),
            Some(BackdropScope::Scoped(_)),
        ),
        "the glass joins the enclosing material scope",
    );

    let ungrouped = rendered(AnyView::new(pair()));
    let members = chrome_layers(&ungrouped);
    assert_eq!(members.len(), 2, "each glass mounts a member");
    assert_ne!(
        member_group(&ungrouped, members[0]),
        member_group(&ungrouped, members[1]),
        "outside a scope each glass is a group of its own",
    );
}

/// The member's clip is the glass's outline on the content's frame: a
/// capsule, a rounded rectangle and an ellipse each bind the matching shape.
#[test]
fn the_outline_follows_the_glasss_shape() {
    let env = pumped_test_environment();
    let outline = |glass: Glass| {
        let mut renderer = test_renderer_with_theme(glass_theme(resting()));
        frame(&mut renderer, &env, glass_view(glass), window());
        member_clip(&renderer)
    };
    let frame_rect = window();

    let ShapeData::RoundedRect(capsule) = outline(Glass::regular()) else {
        panic!("a capsule is a rounded rectangle")
    };
    assert_eq!(capsule.rect(), frame_rect, "on the content's frame");
    assert_eq!(
        capsule.radii().top_left,
        frame_rect.height() / 2.0,
        "half the shorter side"
    );

    let ShapeData::RoundedRect(rounded) =
        outline(Glass::regular().shape(RoundedRectangle::new(0.2)))
    else {
        panic!("a rounded rectangle is a rounded rectangle")
    };
    assert_eq!(rounded.rect(), frame_rect, "on the content's frame");
    assert!(
        0.2f64
            .mul_add(-frame_rect.height(), rounded.radii().top_left)
            .abs()
            < 1e-3,
        "the normalized radius resolves against the shorter side: {:?}",
        rounded.radii()
    );

    let ShapeData::Ellipse(ellipse) = outline(Glass::regular().shape(Ellipse)) else {
        panic!("an ellipse is an ellipse")
    };
    assert_eq!(ellipse.center(), frame_rect.center());
    assert_eq!(ellipse.radii().x, frame_rect.width() / 2.0);
    assert_eq!(ellipse.radii().y, frame_rect.height() / 2.0);

    let custom = outline(Glass::regular().shape(Triangle));
    assert!(
        matches!(custom, ShapeData::Path { .. }),
        "a custom path binds its commands as a path: {custom:?}"
    );
    assert_eq!(
        custom.bounds(),
        frame_rect,
        "the path's unit-space commands resolve against the content's frame"
    );
}

/// A custom shape: the triangle spanning the unit square's top edge and
/// bottom corners.
struct Triangle;

impl Shape for Triangle {
    type Iter = [PathCommand; 4];

    fn path(&self) -> Self::Iter {
        [
            PathCommand::MoveTo { x: 0.5, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 1.0 },
            PathCommand::LineTo { x: 0.0, y: 1.0 },
            PathCommand::Close,
        ]
    }
}

/// A frame change moves the member's outline through its live operand: the
/// layout runs, the member and the group stay, and nothing re-records.
#[test]
fn a_frame_change_moves_the_outline_without_re_recording() {
    let env = pumped_test_environment();
    let mut renderer = test_renderer_with_theme(glass_theme(resting()));
    frame(&mut renderer, &env, glass_view(Glass::regular()), window());
    let [member] = <[_; 1]>::try_from(mirror_chrome_layers(&renderer))
        .expect("the glass mounts exactly one member");
    assert_eq!(member_clip(&renderer).bounds(), window());

    let smaller = Rect::new(0.0, 0.0, 100.0, 50.0);
    assert!(renderer.flush_window_tree(&env, smaller, Affine::IDENTITY, Affine::IDENTITY));
    let stats = renderer.commit_mirror();
    assert_eq!(
        mirror_chrome_layers(&renderer),
        [member],
        "the member stays mounted"
    );
    assert_eq!(
        member_clip(&renderer).bounds(),
        smaller,
        "the outline follows the content's frame"
    );
    assert_eq!(stats.created, 0, "no layer is created");
    assert_eq!(stats.live_records, 0, "no source is recorded again");
}

/// A circle is inscribed in the frame and centred in it, whatever its
/// aspect, and follows the frame as it changes.
#[test]
fn a_circle_outline_is_inscribed_and_follows_the_frame() {
    let env = pumped_test_environment();
    let mut renderer = test_renderer_with_theme(glass_theme(resting()));
    frame(
        &mut renderer,
        &env,
        glass_view(Glass::regular().shape(waterui::shape::Circle)),
        window(),
    );
    let ShapeData::Circle(circle) = member_clip(&renderer) else {
        panic!("a circle is a circle")
    };
    assert_eq!(circle.center, window().center());
    assert_eq!(circle.radius, window().height() / 2.0);

    let tall = Rect::new(0.0, 0.0, 40.0, 90.0);
    assert!(renderer.flush_window_tree(&env, tall, Affine::IDENTITY, Affine::IDENTITY));
    let _ = renderer.commit_mirror();
    let ShapeData::Circle(circle) = member_clip(&renderer) else {
        panic!("a circle is a circle")
    };
    assert_eq!(circle.center, tall.center());
    assert_eq!(circle.radius, 20.0);
}

/// An effect that reads the interaction state: `1` while pressed.
fn pressed_effect() -> Driven<MaterialEffect> {
    Driven::new(|inputs: &Inputs<'_, ()>| {
        MaterialEffect::new(vec![if inputs.state.contains(InteractionState::PRESSED) {
            1.0
        } else {
            0.0
        }])
    })
}

/// A window point the press helpers use.
fn point_of(x: f64, y: f64) -> (f32, f32) {
    (
        crate::num_cast::f64_as_f32(x),
        crate::num_cast::f64_as_f32(y),
    )
}

fn press(renderer: &mut HydrolysisRenderer<ParleyEngine>, env: &Environment, (x, y): (f32, f32)) {
    renderer.handle_pointer_down_with_source(
        POINTER,
        PointerKind::Mouse,
        x,
        y,
        PointerButton::Primary,
        env,
    );
}

fn release(renderer: &mut HydrolysisRenderer<ParleyEngine>, env: &Environment, (x, y): (f32, f32)) {
    renderer.handle_pointer_up_with_source(
        POINTER,
        PointerKind::Mouse,
        x,
        y,
        PointerButton::Primary,
        env,
    );
}

/// The window point at the centre of the first pointer target.
fn first_target(renderer: &HydrolysisRenderer<ParleyEngine>) -> (f32, f32) {
    let center = renderer
        .hit_test
        .pointer_targets
        .first()
        .expect("the button registers a pointer target")
        .bounds
        .center();
    point_of(center.x, center.y)
}

/// A button counting its taps, with `glass` behind it.
fn tapped(glass: Glass, taps: &Rc<Cell<u32>>) -> AnyView {
    let taps = Rc::clone(taps);
    AnyView::new(
        button("Tap")
            .action(move || taps.set(taps.get() + 1))
            .background(glass),
    )
}

/// A press on plain content inside an interactive glass drives the glass's
/// effect through its live operand — no source is recorded — and the release
/// restores it.
#[test]
fn a_press_on_content_drives_an_interactive_glass_live() {
    let env = pumped_test_environment();
    let mut renderer = test_renderer_with_theme(glass_theme(pressed_effect()));
    frame(
        &mut renderer,
        &env,
        glass_view(Glass::regular().interactive(true)),
        window(),
    );
    let [member] = <[_; 1]>::try_from(mirror_chrome_layers(&renderer))
        .expect("the glass mounts exactly one member");
    assert_eq!(member_uniforms(&renderer), [0.0], "at rest");

    press(&mut renderer, &env, (20.0, 20.0));
    assert_eq!(member_uniforms(&renderer), [1.0], "the press drives it");
    let held = renderer.commit_mirror();
    assert_eq!(held.live_records, 0, "the press recorded nothing");
    assert_eq!(held.created, 0);
    assert_eq!(mirror_chrome_layers(&renderer), [member]);

    release(&mut renderer, &env, (20.0, 20.0));
    assert_eq!(member_uniforms(&renderer), [0.0], "the release restores it");
}

/// The press point follows the pointer while the press is held.
#[test]
fn an_interactive_glass_follows_the_press_point() {
    let env = pumped_test_environment();
    let at = Driven::new(|inputs: &Inputs<'_, ()>| {
        let point = inputs.press_point.unwrap_or(kurbo::Point::ZERO);
        MaterialEffect::new(vec![
            crate::num_cast::f64_as_f32(point.x),
            crate::num_cast::f64_as_f32(point.y),
        ])
    });
    let mut renderer = test_renderer_with_theme(glass_theme(at));
    frame(
        &mut renderer,
        &env,
        glass_view(Glass::regular().interactive(true)),
        window(),
    );
    press(&mut renderer, &env, (20.0, 30.0));
    assert_eq!(member_uniforms(&renderer), [20.0, 30.0], "where it landed");
    renderer.handle_pointer_move_with_source(POINTER, PointerKind::Mouse, 50.0, 60.0, &env);
    assert_eq!(
        member_uniforms(&renderer),
        [50.0, 60.0],
        "where the pointer is now"
    );
}

/// A press on a button inside an interactive glass reaches the button, which
/// fires on release, and drives the glass while held.
#[test]
fn a_press_on_a_button_inside_an_interactive_glass_reaches_both() {
    let taps = Rc::new(Cell::new(0_u32));
    let env = pumped_test_environment();
    let mut renderer = test_renderer_with_theme(glass_theme(pressed_effect()));
    frame(
        &mut renderer,
        &env,
        tapped(Glass::regular().interactive(true), &taps),
        window(),
    );
    let target = first_target(&renderer);
    press(&mut renderer, &env, target);
    assert_eq!(member_uniforms(&renderer), [1.0], "the glass is held");
    assert_eq!(taps.get(), 0, "the button commits on release");
    release(&mut renderer, &env, target);
    assert_eq!(taps.get(), 1, "the button received the tap");
    assert_eq!(member_uniforms(&renderer), [0.0], "the glass is released");
}

/// A non-interactive glass never changes, and takes no input either.
#[test]
fn a_glass_that_is_not_interactive_never_changes() {
    let taps = Rc::new(Cell::new(0_u32));
    let env = pumped_test_environment();
    let mut renderer = test_renderer_with_theme(glass_theme(pressed_effect()));
    frame(
        &mut renderer,
        &env,
        tapped(Glass::regular(), &taps),
        window(),
    );
    assert!(
        renderer.hit_test.pointer_observers.is_empty(),
        "a non-interactive glass registers no observer"
    );
    let target = first_target(&renderer);
    press(&mut renderer, &env, target);
    assert_eq!(member_uniforms(&renderer), [0.0]);
    release(&mut renderer, &env, target);
    assert_eq!(taps.get(), 1, "the tap reaches the button inside the glass");
}

/// An observer never takes a press from the targets around it: a button
/// beneath an interactive glass still receives its tap, and the glass is
/// held meanwhile.
#[test]
fn a_target_beneath_an_interactive_glass_still_receives_the_press() {
    let taps = Rc::new(Cell::new(0_u32));
    let env = pumped_test_environment();
    let mut renderer = test_renderer_with_theme(glass_theme(pressed_effect()));
    let view = AnyView::new(zstack((
        tapped(Glass::regular().shape(waterui::shape::Rectangle), &taps),
        glass_view(Glass::regular().interactive(true)),
    )));
    frame(&mut renderer, &env, view, window());
    let target = first_target(&renderer);
    press(&mut renderer, &env, target);
    release(&mut renderer, &env, target);
    assert_eq!(taps.get(), 1, "the button beneath received the tap");
}
