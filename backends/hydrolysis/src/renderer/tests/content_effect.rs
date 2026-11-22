//! Surface content-effect shaders (water-rs/waterui#2431): a widget theme can
//! apply a content-effect shader to a widget's own isolated content through
//! [`Surface::content_effect`] and [`GlassLook::content_effect`], leaving
//! backdrop refraction independent, keeping `push_above` overlays undistorted,
//! and emitting zero extra isolation layers when `None`.

use core::time::Duration;
use std::time::Instant;

use cherenkov_record::{
    ContentShader, ContentShaderSource, MaterialEffect, MaterialRegistry, WorkingColor,
};
use kurbo::{Affine, Rect};
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::background::Glass;
use waterui::graphics::Color;
use waterui_controls::Label;
use waterui_controls::button::button;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment};
use waterui_layout::stack::{hstack, zstack};

use super::material_chrome::{GLASS, GLASS_SHADER, glass_plan};
use super::mirror::no_shader_mount;
use super::{MinimalTestTheme, pumped_test_environment, test_renderer_with_theme};
#[cfg(hydrolysis_cpu_painter)]
use crate::engine::RasterEngineState;
#[cfg(hydrolysis_cpu_painter)]
use crate::platform::OffscreenPainter;
use crate::platform::{InputEvent, PointerKind};
use crate::theme::{
    Backdrop, Chrome, ContentEffect, CornerShape, Driven, FocusIndicator, GlassLook, Motion,
    PressFeedback, StateLayer, Surface,
};
use crate::{HeadlessRuntime, HeadlessSnapshot};

const CONTENT_SHADER: ContentShader = ContentShader::new(0);
const WIDTH: u32 = 160;
const HEIGHT: u32 = 80;

/// Shifts isolated content right by `params[0].x * px.scale` device pixels.
const SHIFT_X_WGSL: &str = "
fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
    return content_sample(px.p - vec2<f32>(params[0].x * px.scale, 0.0));
}
";

/// Edge-following SDF displacement of isolated content along `px.normal`.
#[cfg(hydrolysis_cpu_painter)]
const SDF_DISPLACE_WGSL: &str = "
fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
    let t = clamp(1.0 + px.sdf / (12.0 * px.scale), 0.0, 1.0);
    return content_sample(px.p - px.normal * params[0].x * px.scale * t * t);
}
";

fn capture_at(runtime: &mut HeadlessRuntime, at: Instant) -> HeadlessSnapshot {
    runtime
        .pump_at(true, at)
        .into_snapshot()
        .expect("headless snapshot")
}

/// Samples logical point `(x, y)` in `snapshot`, accounting for display scale.
fn rgba_at(snapshot: &HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
    let scale = (snapshot.width() / WIDTH).max(1);
    let px = x * scale;
    let py = y * scale;
    let idx = ((py * snapshot.width() + px) * 4) as usize;
    let bytes = snapshot.rgba8();
    [bytes[idx], bytes[idx + 1], bytes[idx + 2], bytes[idx + 3]]
}

fn button_chrome(content_effect: Option<ContentEffect>, hover_blue: bool) -> Chrome {
    let mut surface = Surface::new(CornerShape::radius(8.0), WorkingColor::TRANSPARENT);
    if let Some(effect) = content_effect {
        surface = surface.content_effect(effect);
    }
    let state_layer = if hover_blue {
        let mut layer = StateLayer::new(Color::srgb_hex("#0000FF"));
        layer.hover = 0.5;
        layer
    } else {
        StateLayer::new(Color::transparent())
    };
    Chrome::new(
        surface,
        state_layer,
        PressFeedback::new(),
        FocusIndicator::Tint,
        Motion::uniform(Animation::linear(Duration::ZERO)),
    )
}

fn button_scene() -> AnyViewBuilder<AnyView> {
    AnyViewBuilder::new(|| {
        AnyView::new(
            zstack((
                // Two-tone green background behind the button: dark left, bright right.
                hstack((
                    Color::srgb_hex("#004400").size(80.0, 80.0),
                    Color::srgb_hex("#00CC00").size(80.0, 80.0),
                ))
                .spacing(0.0),
                // Button spanning [20..140, 16..64], containing a red bar on its left side.
                button(Label::new("Action", || {
                    hstack((
                        Color::srgb_hex("#FF0000").size(20.0, 40.0),
                        Color::transparent().size(100.0, 40.0),
                    ))
                    .spacing(0.0)
                }))
                .action(|| {})
                .width(120.0)
                .height(48.0),
            ))
            .size(160.0, 80.0),
        )
    })
}

#[test]
fn content_effect_none_creates_zero_extra_layers() {
    let env = pumped_test_environment();
    let window = Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT));

    let mut plain_renderer = test_renderer_with_theme(
        MinimalTestTheme::default().with_button_chrome(button_chrome(None, false)),
    );
    plain_renderer.reset_scene();
    plain_renderer.begin_rebuild_frame();
    plain_renderer.capture_window_tree(
        button_scene().build(),
        &env,
        window,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    plain_renderer.finish_rebuild_frame();
    let plain_stats = plain_renderer.commit_mirror();
    assert!(
        plain_renderer.mirror().content_effect_layers().is_empty(),
        "no layer should carry a content effect"
    );

    let effect = ContentEffect::new(
        CONTENT_SHADER,
        Driven::constant(MaterialEffect::new(vec![10.0])),
    );
    let mut shaded_renderer = test_renderer_with_theme(
        MinimalTestTheme::default()
            .with_content_shader(
                CONTENT_SHADER,
                ContentShaderSource::wgsl(SHIFT_X_WGSL).reach(64.0),
            )
            .with_button_chrome(button_chrome(Some(effect), false)),
    );
    shaded_renderer.reset_scene();
    shaded_renderer.begin_rebuild_frame();
    shaded_renderer.capture_window_tree(
        button_scene().build(),
        &env,
        window,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    shaded_renderer.finish_rebuild_frame();
    let shaded_stats = shaded_renderer.commit_mirror();
    assert_eq!(
        shaded_renderer.mirror().content_effect_layers().len(),
        1,
        "exactly one isolated scope layer should carry the content effect"
    );
    assert_eq!(
        shaded_stats.created,
        plain_stats.created + 1,
        "content_effect: None must create zero extra isolation layers"
    );
}

#[test]
fn content_effect_displaces_button_content_without_refracting_backdrop_and_keeps_above_overlay_undistorted()
 {
    let env = pumped_test_environment();
    let effect = ContentEffect::new(
        CONTENT_SHADER,
        Driven::constant(MaterialEffect::new(vec![24.0])),
    );
    let theme = MinimalTestTheme::default()
        .with_content_shader(
            CONTENT_SHADER,
            ContentShaderSource::wgsl(SHIFT_X_WGSL).reach(96.0),
        )
        .with_button_chrome(button_chrome(Some(effect), true));

    let mut runtime = HeadlessRuntime::new_for_tests(env, button_scene(), WIDTH, HEIGHT, theme);
    let now = Instant::now();
    let rest = capture_at(&mut runtime, now);

    // Button spans x = 20..140. Its red bar originally sits near x = 20..40,
    // shifted right by +24 pt to x = 44..64.
    let shifted = rgba_at(&rest, 54, 40);
    assert!(
        shifted[0] > 200 && shifted[1] < 30 && shifted[2] < 30,
        "shifted red bar should land at x=54, got {shifted:?}"
    );
    // At x = 30 (originally red), content_sample(6) is outside the isolated
    // button bounds [20..140], returning transparent so the un-refracted
    // dark-green backdrop (#004400) shows cleanly through.
    let vacated = rgba_at(&rest, 30, 40);
    assert!(
        vacated[0] < 20 && vacated[1] > 45 && vacated[1] < 90 && vacated[2] < 20,
        "vacated region should show un-refracted dark-green backdrop, got {vacated:?}"
    );
    // At x = 110 (inside button, right of shifted bar), bright-green backdrop (#00CC00)
    // is completely undistorted.
    let right_bg = rgba_at(&rest, 110, 40);
    assert!(
        right_bg[0] < 20 && right_bg[1] > 180 && right_bg[2] < 20,
        "right backdrop should remain bright green, got {right_bg:?}"
    );

    // Hover over the button so `push_above` paints its blue state layer.
    runtime.push_input_event(InputEvent::PointerMove {
        id: 1,
        kind: PointerKind::Mouse,
        x: 80.0,
        y: 40.0,
    });
    let hovered = capture_at(&mut runtime, now + Duration::from_millis(20));
    // Both x = 30 (left edge of button, where content_sample returned transparent)
    // and x = 54 (over shifted red bar) must receive the undistorted blue state
    // layer painted by `push_above`.
    let hovered_left = rgba_at(&hovered, 30, 40);
    let hovered_red = rgba_at(&hovered, 54, 40);
    assert!(
        hovered_left[2] > 100,
        "push_above hover state layer must paint undistorted at left edge, got {hovered_left:?}"
    );
    assert!(
        hovered_red[2] > 100 && hovered_red[0] > 100,
        "push_above hover state layer must composite above shifted red content, got {hovered_red:?}"
    );
}

#[test]
fn glass_look_content_effect_shades_view_content() {
    let env = pumped_test_environment();
    let content_effect = ContentEffect::new(
        CONTENT_SHADER,
        Driven::constant(MaterialEffect::new(vec![20.0])),
    );
    let theme = MinimalTestTheme {
        chrome: glass_plan(vec![]),
        ..MinimalTestTheme::default()
    }
    .with_content_shader(
        CONTENT_SHADER,
        ContentShaderSource::wgsl(SHIFT_X_WGSL).reach(80.0),
    )
    .with_glass_background(move |_, _: &Environment| {
        Some(
            GlassLook::new(
                Backdrop::new(
                    GLASS_SHADER,
                    GLASS,
                    Driven::constant(MaterialEffect::new(vec![0.0, 0.0, 0.0, 0.0])),
                ),
                Motion::uniform(Animation::linear(Duration::ZERO)),
            )
            .content_effect(content_effect.clone()),
        )
    });

    let view = AnyViewBuilder::new(|| {
        AnyView::new(
            zstack((
                Color::srgb_hex("#004400").size(160.0, 80.0),
                hstack((
                    Color::srgb_hex("#FF0000").size(20.0, 48.0),
                    Color::transparent().size(100.0, 48.0),
                ))
                .spacing(0.0)
                .size(120.0, 48.0)
                .background(Glass::regular()),
            ))
            .size(160.0, 80.0),
        )
    });

    let mut runtime = HeadlessRuntime::new_for_tests(env, view, WIDTH, HEIGHT, theme);
    let snap = capture_at(&mut runtime, Instant::now());

    // Red bar originally at x = 20..40 shifts right by +20 pt to x = 40..60.
    let shifted = rgba_at(&snap, 50, 40);
    assert!(
        shifted[0] > 200 && shifted[1] < 30 && shifted[2] < 30,
        "GlassLook content_effect must shift view content to x=50, got {shifted:?}"
    );
    let vacated = rgba_at(&snap, 28, 40);
    assert!(
        vacated[0] < 20 && vacated[1] > 45 && vacated[2] < 20,
        "vacated region under GlassLook must show dark-green backdrop, got {vacated:?}"
    );
}

#[cfg(hydrolysis_cpu_painter)]
#[test]
fn content_effect_gpu_and_cpu_parity() {
    let make_theme = || {
        let effect = ContentEffect::new(
            CONTENT_SHADER,
            Driven::constant(MaterialEffect::new(vec![6.0])),
        );
        MinimalTestTheme::default()
            .with_content_shader(
                CONTENT_SHADER,
                ContentShaderSource::wgsl(SDF_DISPLACE_WGSL).reach(32.0),
            )
            .with_button_chrome(button_chrome(Some(effect), true))
    };

    let mut gpu_runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        button_scene(),
        WIDTH,
        HEIGHT,
        make_theme(),
    );
    let cpu_painter = OffscreenPainter::Cpu(RasterEngineState::new().expect("CPU raster engine"));
    let mut cpu_runtime = HeadlessRuntime::new_for_tests_on_context(
        cpu_painter,
        pumped_test_environment(),
        button_scene(),
        WIDTH,
        HEIGHT,
        make_theme(),
    );

    let now = Instant::now();
    let gpu = capture_at(&mut gpu_runtime, now);
    let cpu = capture_at(&mut cpu_runtime, now);
    assert_eq!((gpu.width(), gpu.height()), (cpu.width(), cpu.height()));

    let mut max_diff = 0u8;
    let mut total_diff = 0u64;
    for (&g, &c) in gpu.rgba8().iter().zip(cpu.rgba8().iter()) {
        let diff = g.abs_diff(c);
        max_diff = max_diff.max(diff);
        total_diff += u64::from(diff);
    }
    let channels = u64::try_from(gpu.rgba8().len()).expect("channel count fits u64");
    assert!(
        max_diff <= 6 && total_diff * 20 < channels,
        "GPU and CPU content_effect renders diverged: max channel delta {max_diff}, total {total_diff}/{channels}"
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(expected = "this engine has no backdrop shaders")
)]
fn no_shader_target_rejects_registry_with_content_shader() {
    let mut registry = MaterialRegistry::new();
    registry.register_content_shader(
        CONTENT_SHADER,
        ContentShaderSource::wgsl(SHIFT_X_WGSL).reach(16.0),
    );
    let _mount = no_shader_mount(&registry);
}
