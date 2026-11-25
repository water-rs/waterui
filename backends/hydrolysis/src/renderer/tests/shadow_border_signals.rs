use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use waterui::animation::Animation;
use waterui::border::Border;
use waterui::component::button;
use waterui::interaction::{InteractionState, StateValue};
use waterui::prelude::{Color, EdgeInsets};
use waterui::shape::{Circle, Rectangle};
use waterui::style::{FloatingElevation, FloatingStyle, Shadow};
use waterui::{AnyView, Binding, SignalExt as _, View, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::stack::{vstack, zstack};

use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerKind};
use crate::runner::HeadlessSnapshot;

const CANVAS: u32 = 160;
const CASTER: f32 = 80.0;

fn render_once<V: View + 'static>(view: impl Fn() -> V + 'static) -> HeadlessSnapshot {
    let builder = AnyViewBuilder::<AnyView>::new(move || AnyView::new(view()));
    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        CANVAS,
        CANVAS,
        MinimalTestTheme::default(),
    );
    runtime
        .pump(true)
        .into_snapshot()
        .expect("capturing pump must yield a snapshot")
}

fn pixel(snapshot: &HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
    let width = snapshot.width() as usize;
    let index = (y as usize * width + x as usize) * 4;
    let bytes = snapshot.rgba8();
    [
        bytes[index],
        bytes[index + 1],
        bytes[index + 2],
        bytes[index + 3],
    ]
}

fn clipped_avatar(shadow: Shadow) -> impl View {
    zstack((
        Color::srgb(255, 255, 255),
        ().size(CASTER, CASTER)
            .background(Color::srgb(245, 245, 245))
            .clip(Circle)
            .shadow(shadow),
    ))
}

#[test]
fn clipped_caster_inherits_circular_shadow_silhouette() {
    let inherited = render_once(|| clipped_avatar(Shadow::splat(2.0).offset(6.0, 6.0)));
    // Top-left notch outside the centered 80x80 circle stays clear white,
    // while bottom-right (110, 110) — outside the circle (d = 42.4 > 40) yet
    // inside the (6, 6)-shifted circle (d = 33.9 < 40) — receives shadow.
    let corner_notch = pixel(&inherited, 44, 44);
    let inside_offset_circle = pixel(&inherited, 110, 110);

    assert!(
        corner_notch[0] > 240 && corner_notch[1] > 240 && corner_notch[2] > 240,
        "corner outside `.clip(Circle)` must stay clear of shadow, got {corner_notch:?}"
    );
    assert!(
        inside_offset_circle[0] < 200,
        "circular offset shadow must darken bottom-right outside circle, got {inside_offset_circle:?}"
    );
}

#[test]
fn explicit_silhouette_overrides_inherited_clip_shape() {
    let overridden =
        render_once(|| clipped_avatar(Shadow::splat(2.0).offset(6.0, 6.0).silhouette(Rectangle)));
    // At (122, 122), shifted circle center (86, 86) is at d = 50.9 > 40 (clear),
    // whereas shifted Rectangle [46..126, 46..126] covers it with shadow.
    let rect_corner_shadow = pixel(&overridden, 122, 122);
    let inherited = render_once(|| clipped_avatar(Shadow::splat(2.0).offset(6.0, 6.0)));
    let circle_corner = pixel(&inherited, 122, 122);

    assert!(
        u16::from(rect_corner_shadow[0]) + 20 < u16::from(circle_corner[0]),
        "explicit `Rectangle` silhouette must darken corner {rect_corner_shadow:?} vs circular {circle_corner:?}"
    );
}

#[test]
fn reactive_shadow_and_border_update_without_rebuilds() {
    let phase = Binding::container(false);
    let builds = Arc::new(AtomicUsize::new(0));
    let view_builds = Arc::clone(&builds);
    let anim = Animation::linear(Duration::from_millis(160));

    let view_phase = phase.clone();
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view_builds.fetch_add(1, Ordering::Relaxed);
        let radius = view_phase
            .map(|on| if on { 10.0_f32 } else { 1.0 })
            .with(anim.clone());
        let width = view_phase
            .map(|on| if on { 6.0_f32 } else { 1.0 })
            .with(anim.clone());
        let corner = view_phase
            .map(|on| if on { 16.0_f32 } else { 0.0 })
            .with(anim.clone());
        let border_color = view_phase
            .map(|on| {
                if on {
                    Color::srgb(20, 90, 220)
                } else {
                    Color::srgb(220, 40, 40)
                }
            })
            .with(anim.clone());
        let shadow_color = view_phase
            .map(|on| Color::srgb(0, 0, 0).with_opacity(if on { 0.85 } else { 0.15 }))
            .with(anim.clone());

        AnyView::new(
            vstack((()
                .size(CASTER, CASTER)
                .background(Color::srgb(250, 250, 250))
                .border_with(Border::new(border_color, width).corner_radius(corner))
                .shadow(Shadow::splat(radius).offset(4.0, 4.0).color(shadow_color)),))
            .padding_with(EdgeInsets::all(20.0)),
        )
    });

    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        CANVAS,
        CANVAS,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let first = runtime.pump_at(true, start);
    let first_snap = first.into_snapshot().expect("initial frame snapshot");
    let first_edge = pixel(&first_snap, 42, 42);

    phase.set(true);
    let mid = runtime.pump_at(true, start + Duration::from_millis(80));
    let mid_work = mid.profile().counters().frame_work();
    let mid_snap = mid.into_snapshot().expect("mid animation snapshot");

    let settled = runtime.pump_at(true, start + Duration::from_millis(220));
    let settled_work = settled.profile().counters().frame_work();
    let settled_snap = settled.into_snapshot().expect("settled frame snapshot");
    let settled_edge = pixel(&settled_snap, 42, 42);

    assert_eq!(
        builds.load(Ordering::Relaxed),
        1,
        "animated Border + Shadow metrics must build the view once"
    );

    assert_eq!(
        mid_work.semantic_builds, 0,
        "mid-animation must not rebuild"
    );
    assert_eq!(
        mid_work.structural_patches, 0,
        "mid-animation must not structurally patch"
    );
    assert_eq!(
        settled_work.semantic_builds, 0,
        "settled frame must not rebuild"
    );
    assert_eq!(
        settled_work.structural_patches, 0,
        "settled frame must not structurally patch"
    );
    assert_ne!(
        first_snap.rgba8(),
        mid_snap.rgba8(),
        "animated Border + Shadow metrics must change pixels mid-flight"
    );
    assert_ne!(
        first_edge, settled_edge,
        "Border width/color/corner update must change border pixels ({first_edge:?} vs {settled_edge:?})"
    );
}

#[test]
fn floating_surface_updates_shadow_on_hover_without_rebuilds() {
    let style = FloatingStyle {
        elevation: StateValue::new(FloatingElevation {
            ambient_shadow_color: Color::srgb(0, 0, 0).with_opacity(0.08),
            ambient_shadow_radius: 1.0,
            ambient_shadow_offset_y: 1.0,
            key_shadow_color: Color::srgb(0, 0, 0).with_opacity(0.14),
            key_shadow_radius: 2.0,
            key_shadow_offset_y: 1.0,
        })
        .when(
            InteractionState::HOVERED,
            FloatingElevation {
                ambient_shadow_color: Color::srgb(0, 0, 0).with_opacity(0.45),
                ambient_shadow_radius: 10.0,
                ambient_shadow_offset_y: 6.0,
                key_shadow_color: Color::srgb(0, 0, 0).with_opacity(0.85),
                key_shadow_radius: 14.0,
                key_shadow_offset_y: 8.0,
            },
        ),
        ..FloatingStyle::default()
    };

    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(
            button("Float")
                .action(|| {})
                .floating_with(style.clone())
                .padding_with(EdgeInsets::all(24.0)),
        )
    });

    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        CANVAS,
        CANVAS,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let rest = runtime
        .pump_at(true, start)
        .into_snapshot()
        .expect("rest frame snapshot");

    runtime.push_input_event(InputEvent::PointerMove {
        id: 1,
        kind: PointerKind::Mouse,
        x: 64.0,
        y: 44.0,
    });
    let hovered = runtime.pump_at(true, start + Duration::from_millis(16));
    let work = hovered.profile().counters().frame_work();
    let hovered_snap = hovered.into_snapshot().expect("hovered frame snapshot");

    assert_eq!(work.semantic_builds, 0, "hover shadow must not rebuild");
    assert_eq!(
        work.structural_patches, 0,
        "hover shadow must not structurally patch"
    );
    assert_ne!(
        rest.rgba8(),
        hovered_snap.rgba8(),
        "hover elevation must change rendered shadow pixels"
    );
}
