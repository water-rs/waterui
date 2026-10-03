//! water-rs/hydrolysis#118: the context-menu popup painted only its fill.
//!
//! The window the popup asks the OS for carries an alpha-channel
//! background, so the winit runner realizes it as a transparent window —
//! a depth-32 ARGB window on X11. On an X11 display stack driven by a
//! Mesa software rasterizer older than 24.1, the WSI's software
//! `x11_present_to_x11_sw` sends `xcb_put_image` at a hardcoded depth of
//! 24 and discards the reply, so every present into a depth-32 window
//! answers `BadMatch` into a queue nobody reads: the window's own pixels
//! stay at their initial fill while hit-testing, item actions,
//! outside-click dismissal and edge clamping all still work. Mesa fixed
//! it upstream in 24.1 (commit 1e849b12); hydrolysis fails loudly at
//! surface creation when the adapter is that stack
//! (`mesa_x11_transparency_blocker` in platform.rs).
//!
//! The first assertion below is the contract the fix keeps: the popup
//! window still *requests* transparency — the background is genuinely
//! transparent and the true alpha-channel window is what gets created
//! everywhere the stack can present it. The second reads the popup's own
//! rendered frame — produced by `render_window_with_capture`, the path
//! the winit runner drives per frame, not the semantic tree — and
//! requires item pixels beyond the panel fill, the guard that makes a
//! fill-only frame fail loudly.

use std::time::Instant;

use waterui::ViewExt as _;
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::frame::Frame;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

fn secondary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
    ]
}

fn dominant_rgb(rgba8: &[u8]) -> [u8; 3] {
    let mut counts = std::collections::HashMap::<[u8; 3], u32>::new();
    for px in rgba8.as_chunks::<4>().0 {
        *counts.entry([px[0], px[1], px[2]]).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map_or([0, 0, 0], |(rgb, _)| rgb)
}

fn differs(pixel: &[u8], modal: [u8; 3], tolerance: u8) -> bool {
    pixel[..3]
        .iter()
        .zip(modal)
        .any(|(channel, fill)| channel.abs_diff(fill) > tolerance)
}

#[test]
fn the_context_menu_popup_presents_its_items_through_a_presentable_window() {
    let env = test_environment();
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(
            Frame::new(button("host").action(|| {}))
                .width(160.0)
                .height(160.0)
                .context_menu(vec![
                    "Copy".action(|| {}),
                    "Cut".action(|| {}),
                    "Paste".action(|| {}),
                ]),
        )
    });
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, 160, 160, MinimalTestTheme::default());
    let _ = runtime.pump_at(false, Instant::now());
    for event in secondary_click(80.0, 80.0) {
        runtime.push_input_event(event);
    }
    // The enter animation needs real-time frames to converge; item pixels
    // are present well before it does, so a still-running animation is
    // not a failure — only a missing popup is.
    for _ in 0..240 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }

    assert!(
        runtime.popup_window(0).is_some(),
        "the secondary press must mount the context-menu popup"
    );
    #[cfg(hydrolysis_winit)]
    assert!(
        crate::runner::window_requires_transparency(
            runtime
                .popup_window(0)
                .expect("the context-menu popup is mounted"),
            &env
        ),
        "the context-menu popup must keep requesting the transparent window \
         it needs for its rounded corners — the renderer keeps the true \
         alpha channel everywhere the stack can present it"
    );

    let frame = runtime
        .popup_frame(0)
        .expect("the mounted popup must render its own frame");
    let fill = dominant_rgb(&frame.rgba8);
    let item_pixels = frame
        .rgba8
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| differs(&px[..], fill, 48))
        .count();
    assert!(
        item_pixels > 100,
        "the popup's own rendered frame holds only the panel fill: \
         {item_pixels} item pixels in {}x{}",
        frame.width,
        frame.height,
    );
}
