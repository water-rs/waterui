//! Renders the component offscreen and decodes the pixels back.
//!
//! This is the only assertion that says what matters about a QR code. Geometry
//! tests say the modules are where the encoder put them; nothing but a decoder
//! says whether a camera pointed at the screen would read the payload — and the
//! ways a drawn code stops being readable (module edges antialiased into grey
//! seams, a quiet zone that is not there, a light-on-dark polarity most
//! detectors do not look for) all leave a picture that still looks like a QR
//! code.
//!
//! The PNGs land in `WaterUI`'s canonical artifact layout, so the same run that
//! asserts a code decodes also leaves the image to look at.

use core::time::Duration;

use image::GrayImage;
use nami::Binding;
use waterui::Environment;
use waterui_graphics::color::Color;
use waterui_qr::{ErrorCorrection, qr_code};
use waterui_str::Str;
use waterui_testing::{OffscreenApp, Snapshot, install_default_theme, ui};

const PAYLOAD: &str = "https://waterui.dev";
const OTHER_PAYLOAD: &str = "https://waterui.dev/docs/qr";

/// Every payload the frame currently shows, as a decoder reads it.
///
/// The snapshot is the whole window rather than the code alone, which is what a
/// camera sees too: the detector has to find the symbol before it can read it,
/// and that is exactly what the quiet zone is for.
fn decode(snapshot: &Snapshot) -> Vec<String> {
    let luma = GrayImage::from_fn(snapshot.width, snapshot.height, |x, y| {
        let index = ((y * snapshot.width + x) * 4) as usize;
        let [red, green, blue, alpha] = [
            f32::from(snapshot.rgba8[index]),
            f32::from(snapshot.rgba8[index + 1]),
            f32::from(snapshot.rgba8[index + 2]),
            f32::from(snapshot.rgba8[index + 3]) / 255.0,
        ];
        // Composited over white, because that is what an uncomposited frame
        // reaches a viewer's eye over and the alternative is reading a
        // transparent pixel as black.
        let over_white = |channel: f32| 255.0f32.mul_add(1.0 - alpha, channel * alpha);
        let value = 0.2126f32.mul_add(
            over_white(red),
            0.7152f32.mul_add(over_white(green), 0.0722 * over_white(blue)),
        );
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=255 before the cast"
        )]
        let level = value.round().clamp(0.0, 255.0) as u8;
        image::Luma([level])
    });

    rqrr::PreparedImage::prepare(luma)
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok())
        .map(|(_meta, content)| content)
        .collect()
}

/// Asserts the frame decodes to exactly `expected`, and leaves the PNG behind.
fn assert_decodes(app: &mut OffscreenApp, case: &str, stage: &str, expected: &str) {
    let captured = app.capture_snapshot("waterui-qr", case, stage);
    let decoded = decode(captured.snapshot());
    assert_eq!(
        decoded,
        vec![String::from(expected)],
        "{case}/{stage}: the rendered code must decode to its payload; \
         the frame is at {}",
        captured.path().display()
    );
}

/// Two pixels a module — small enough that a module edge landing inside a pixel
/// would put a grey seam on most rows of the symbol, which is the case snapping
/// exists for.
///
/// The root view is offered the whole window, so the viewport is what sets the
/// module size in these tests: this payload encodes to a 25-module symbol at the
/// default correction, 33 modules with its quiet zone, and 80 units across 33
/// modules snaps to 2.
#[test]
fn a_code_drawn_two_pixels_to_the_module_decodes() {
    let mut app = ui().viewport(80, 80).mount_offscreen(|| qr_code(PAYLOAD));

    assert_decodes(&mut app, "small", "00_two_pixels_a_module", PAYLOAD);
}

/// The same payload with room to spare, at the correction level that makes the
/// largest symbol: 29 modules, 37 with the quiet zone, snapping to 11 units a
/// module in a 420-unit window.
#[test]
fn a_large_code_decodes() {
    let mut app = ui()
        .viewport(420, 420)
        .mount_offscreen(|| qr_code(PAYLOAD).correction(ErrorCorrection::High));

    assert_decodes(&mut app, "large", "00_eleven_pixels_a_module", PAYLOAD);
}

/// The payload is a signal, so changing it re-encodes and redraws the existing
/// surface. What is asserted is not that some pixels moved but that the *new*
/// payload comes back out of the frame.
#[test]
fn the_drawn_code_follows_its_payload_signal() {
    let payload = Binding::container(Str::from_static(PAYLOAD));
    let shown = payload.clone();
    let mut app = ui()
        .viewport(320, 320)
        .mount_offscreen(move || qr_code(shown.clone()));

    assert_decodes(&mut app, "reactive", "00_initial", PAYLOAD);

    payload.set(Str::from_static(OTHER_PAYLOAD));
    // The change reaches the surface as a scene invalidation, so let the frames
    // it schedules run before reading the pixels back.
    app.pump_for(Duration::from_millis(64));

    assert_decodes(&mut app, "reactive", "01_after_change", OTHER_PAYLOAD);
}

/// A dark theme must not invert the code.
///
/// `Foreground` on `Surface` taken verbatim would draw light modules on a dark
/// ground under this theme, and `rqrr` — like `quirc` and `ZXing`, which is what
/// the phone pointed at the screen is running — does not look for the
/// reflectance-reversed form at all. This test is the reason the default pair
/// is ordered by lightness rather than by role.
#[test]
fn a_dark_theme_keeps_the_code_scannable() {
    let mut app = ui()
        .theme(|env: &mut Environment| {
            install_default_theme(env);
            hydrolysis_m3::install_dark(env);
        })
        .viewport(320, 320)
        .mount_offscreen(|| qr_code(PAYLOAD));

    assert_decodes(&mut app, "dark", "00_dark_theme", PAYLOAD);
}

/// Why the default pair is ordered by lightness rather than by role.
///
/// Drawn deliberately reflectance-reversed — light modules on a dark ground,
/// which is exactly what the theme's `Foreground` on `Surface` produces under a
/// dark scheme — the code the tests above read back is not found at all. This
/// is the evidence behind [`a_dark_theme_keeps_the_code_scannable`]: without
/// the ordering, that test would be reading this frame.
#[test]
fn a_reflectance_reversed_code_is_not_found_by_a_decoder() {
    let mut app = ui().viewport(320, 320).mount_offscreen(|| {
        qr_code(PAYLOAD)
            .module_color(Color::srgb(255, 255, 255))
            .background_color(Color::srgb(0, 0, 0))
    });

    let captured = app.capture_snapshot("waterui-qr", "inverted", "00_light_on_dark");
    assert!(
        decode(captured.snapshot()).is_empty(),
        "a light-on-dark code decoding would mean this crate's colour ordering \
         guards against nothing; the frame is at {}",
        captured.path().display()
    );
}
