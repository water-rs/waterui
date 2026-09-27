//! Pixel regression for `Metadata<Shadow>` corner geometry.
//!
//! `apply_shadow` rasterizes the shadow from `Shadow::silhouette` — the
//! caster's shape — not from a corner radius detached from it and certainly
//! not from the blur radius. A rounded surface whose shadow fell back to a
//! square silhouette renders a blob that pokes into the surface's rounded
//! notch: along the corner diagonal an `R`-radius arc starts covering at
//! `t = R(√2−1)`, so a probe ~4px inside the corner is covered by a square
//! silhouette but cleanly outside the true arc.

use std::time::Instant;

use waterui::graphics::Color;
use waterui::style::{FloatingStyle, Shadow, Vector};
use waterui::{Binding, View, ViewExt as _};
use waterui_core::dynamic::watch;
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_shape::{Ellipse, FixedRoundedRectangle, ShapeExt as _};

use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;

const SURFACE_RGB: [u8; 3] = [60, 120, 200];

/// A 56×56 surface with 16px corners at (20,20)–(76,76), casting a tight black
/// shadow (blur 1, no offset) whose silhouette is the caster's shape.
fn caster() -> impl View {
    ().size(56.0, 56.0)
        .background(FixedRoundedRectangle::new(16.0).fill(Color::srgb(
            SURFACE_RGB[0],
            SURFACE_RGB[1],
            SURFACE_RGB[2],
        )))
        .shadow(Shadow::new(
            Color::srgb(0, 0, 0),
            Vector::new(0.0, 0.0),
            1.0,
            FixedRoundedRectangle::new(16.0),
        ))
}

/// A 56×56 floating surface at (20,20)–(76,76) whose clip and shadows all carry
/// the same normalized `RoundedRect` silhouette (0.3 × 56 ≈ 16.8px corners).
fn floating_caster() -> impl View {
    ().size(56.0, 56.0).floating_with(FloatingStyle {
        container_color: Color::srgb(SURFACE_RGB[0], SURFACE_RGB[1], SURFACE_RGB[2]),
        clip_radius: 0.3,
        minimum_width: 0.0,
        minimum_height: 0.0,
        ambient_shadow_color: Color::srgb(0, 0, 0),
        ambient_shadow_radius: 1.0,
        ambient_shadow_offset_y: 0.0,
        key_shadow_color: Color::srgb(0, 0, 0),
        key_shadow_radius: 1.0,
        key_shadow_offset_y: 0.0,
        ..FloatingStyle::default()
    })
}

/// The padded block fills the 120×120 window exactly, so the caster lands at
/// (20,20)–(76,76) no matter how the content is distributed.
fn scene(view: impl View) -> impl View {
    Padding::new(EdgeInsets::new(20.0, 44.0, 20.0, 44.0), view)
}

fn pixel(snapshot: &crate::runner::HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * snapshot.width + x) * 4) as usize;
    snapshot.rgba8[offset..offset + 4]
        .try_into()
        .expect("pixel in bounds")
}

fn snapshot<V: View + 'static>(view: impl Fn() -> V + 'static) -> crate::runner::HeadlessSnapshot {
    let value = Binding::container(0_i32);
    let builder = AnyViewBuilder::<waterui_core::AnyView>::new(move || {
        let _ = &value;
        waterui_core::AnyView::new(scene(view()))
    });
    let env = pumped_test_environment();
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, 120, 120, MinimalTestTheme::default());
    runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("frame must produce a snapshot")
}

/// (21,21) sits inside the surface rect but ~5px beyond the diagonal reach of a
/// ~16px corner arc, past the 1σ blur's 2.5px falloff: the surface is
/// transparent there, so the pixel shows only whatever the shadow paints. A
/// shadow that ignored the silhouette still covers the notch and darkens it to
/// near-black.
fn assert_notch_stays_clear(snapshot: &crate::runner::HeadlessSnapshot) {
    let edge = pixel(snapshot, 48, 21);
    assert_eq!(
        edge[..3],
        SURFACE_RGB,
        "the surface must paint its top edge; got {edge:?}"
    );

    let notch = pixel(snapshot, 21, 21);
    let background = pixel(snapshot, 110, 110);
    let darkening = background[0].abs_diff(notch[0]);
    assert!(
        darkening < 40,
        "corner-notch pixel must stay near the background: the shadow silhouette \
         must follow the caster's corner radius (notch={notch:?} \
         background={background:?} darkening={darkening})"
    );
}

#[test]
fn shadow_silhouette_follows_caster_shape() {
    assert_notch_stays_clear(&snapshot(caster));
}

#[test]
fn floating_surface_shadow_follows_clip_silhouette() {
    assert_notch_stays_clear(&snapshot(floating_caster));
}

/// An ellipse (or any silhouette `kind_clip_shape` cannot express as a uniform
/// rounded rect) rasterizes on the CPU — once. The raster is keyed on shape,
/// size, the transform's linear part, blur and colour; a changed `.offset`
/// moves only the draw translation, so the second frame must reuse the cached
/// `Blob` rather than rasterize again.
#[test]
fn translated_shadow_reuses_the_cached_silhouette() {
    let dx = Binding::container(0.0_f32);
    let view = {
        let dx = dx.clone();
        move || {
            let dx = dx.clone();
            // `watch` re-dispatches the subtree when `dx` changes, so the
            // second pump re-flushes the shadow at the new translation. A
            // signal-driven `.offset` would instead animate and — its frame
            // subscription being frame-scoped — never observe the `set`.
            watch(dx, |v| {
                ().size(56.0, 56.0)
                    .background(Ellipse.fill(Color::srgb(
                        SURFACE_RGB[0],
                        SURFACE_RGB[1],
                        SURFACE_RGB[2],
                    )))
                    .shadow(Shadow::new(
                        Color::srgb(0, 0, 0),
                        Vector::new(0.0, 0.0),
                        2.0,
                        Ellipse,
                    ))
                    .offset(v, 0.0)
            })
        }
    };
    let value = Binding::container(0_i32);
    let builder = AnyViewBuilder::<waterui_core::AnyView>::new(move || {
        let _ = &value;
        waterui_core::AnyView::new(scene(view()))
    });
    let env = pumped_test_environment();
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, 120, 120, MinimalTestTheme::default());
    let first = runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("first frame must produce a snapshot");
    assert_eq!(
        runtime.renderer().blurred_silhouette_rasterizations(),
        1,
        "the ellipse silhouette rasterizes once on the first frame"
    );

    dx.set(30.0);
    let second = runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("moved frame must produce a snapshot");
    assert_eq!(
        runtime.renderer().blurred_silhouette_rasterizations(),
        1,
        "a pure translation must hit the silhouette cache"
    );

    // Guard the assertion above against a false pass: if the frame had not
    // re-rendered or the image had not moved, the pixel the shadow slides
    // into would still read background.
    let background = pixel(&first, 90, 48);
    let moved = pixel(&second, 90, 48);
    assert!(
        background[0].abs_diff(moved[0]) > 20,
        "the shadow must actually move with the offset (before={background:?} \
         after={moved:?})"
    );
}
