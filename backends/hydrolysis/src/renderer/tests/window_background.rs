//! A window whose `background` carries alpha must reach the framebuffer:
//! the exported frame keeps the translucent backdrop and draws content
//! opaquely on top — the #96 contract the offscreen path can observe.

use super::{MinimalTestTheme, test_environment};
use waterui::ViewExt as _;
use waterui::prelude::*;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;

fn pixel_at(rgba8: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = (y * width + x) as usize * 4;
    rgba8[offset..offset + 4]
        .try_into()
        .expect("pixel offset must be in range")
}

#[test]
fn a_translucent_window_frame_keeps_its_background_alpha_under_opaque_content() {
    let env = test_environment();
    // A 160x160 window whose only content is an opaque 80x80 red box.
    let content_builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(waterui_layout::stack::vstack((()
            .size(80.0, 80.0)
            .background(Color::srgb(255, 0, 0)),)))
    });
    let window = waterui::window::Window::new(
        "",
        waterui_core::binding(waterui::window::WindowState::Normal),
        move || content_builder.build(),
    )
    .background(Color::srgb(51, 51, 51).with_opacity(0.5));
    let mut rt = crate::HeadlessRuntime::new_for_tests_with_window(
        env,
        window,
        160,
        160,
        MinimalTestTheme::default(),
    );
    let snapshot = rt
        .pump_snapshot()
        .snapshot
        .expect("a captured frame must carry pixels");

    // The backdrop, read wherever it peeks out from behind the box.
    // Offscreen targets store straight alpha, so the translucent background
    // reads as its srgb colour at half alpha — never the fully-transparent
    // texel a missing alpha channel would leave.
    let backdrop_is_kept = (0..snapshot.height).any(|y| {
        (0..snapshot.width).any(|x| {
            let pixel = pixel_at(&snapshot.rgba8, snapshot.width, x, y);
            (124..=132).contains(&pixel[3])
                && pixel[..3].iter().all(|channel| (45..=60).contains(channel))
        })
    });
    assert!(
        backdrop_is_kept,
        "no pixel keeps the translucent backdrop: alpha or colour was lost"
    );

    // The box: wherever it lands, its pixels are fully opaque red.
    let mut drew_opaque_content = false;
    for y in 0..snapshot.height {
        for x in 0..snapshot.width {
            let pixel = pixel_at(&snapshot.rgba8, snapshot.width, x, y);
            if pixel[3] == 255 {
                assert!(
                    pixel[0] >= 200 && pixel[1] <= 60 && pixel[2] <= 60,
                    "the drawn content pixel must be the box's red, got {pixel:?} at ({x},{y})"
                );
                drew_opaque_content = true;
            }
        }
    }
    assert!(
        drew_opaque_content,
        "the translucent window frame drew no opaque content at all"
    );
}
