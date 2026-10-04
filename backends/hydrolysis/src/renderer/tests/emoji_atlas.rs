//! Regression for <https://github.com/water-rs/hydrolysis/issues/225>: a
//! subtree that encodes without any late-bound patches — here the pure-color
//! capture an applied filter renders every frame — went through
//! `Resolver::resolve`'s `patches.is_empty()` early return, which reported an
//! `Images::default()` (0x0) atlas. The renderer then freed its persistent
//! image atlas and substituted a 1x1 proxy, while the resolver's `ImageCache`
//! still believed every resident image was uploaded at its old coordinates —
//! so color-emoji glyphs (and decoded images) never painted again once a
//! patch-free encode had run on their renderer.

use std::time::{Duration, Instant};

use waterui::prelude::*;
use waterui_core::{AnyView, handler::AnyViewBuilder};

use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::runner::HeadlessSnapshot;

fn pixel(snapshot: &HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * snapshot.width + x) * 4) as usize;
    snapshot.rgba8[index..index + 4]
        .try_into()
        .expect("four channels per pixel")
}

/// Counts pixels that can only come from the color-emoji bitmap: strongly
/// saturated RGB against a low-saturation scene (the chat background, the
/// plain text face, and the dark blurred chip are all near-neutral).
fn count_emoji_pixels(snapshot: &HeadlessSnapshot) -> usize {
    let mut count = 0;
    for y in 0..snapshot.height {
        for x in 0..snapshot.width {
            let [r, g, b, a] = pixel(snapshot, x, y);
            if a == 255 {
                let max = r.max(g).max(b);
                let min = r.min(g).min(b);
                if max.saturating_sub(min) > 48 && max > 120 {
                    count += 1;
                }
            }
        }
    }
    count
}

/// The emoji line beside a filtered solid box. The filter captures its
/// subtree every frame, and the capture scene carries no patches at all — the
/// encode that wiped the resident image atlas before this fix.
fn emoji_beside_blurred_box() -> AnyView {
    AnyView::new(
        vstack((
            text("\u{1F600}").size(20.0),
            ().size(48.0, 24.0)
                .background(Color::srgb_hex("#36343B"))
                .blur(2.0f32),
        ))
        .spacing(12.0)
        .padding()
        .background(Color::srgb_hex("#141218")),
    )
}

fn runtime_for(view: AnyView, width: u32, height: u32) -> HeadlessRuntime {
    let view = std::cell::RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    // The filter's async setup needs the runtime's own draining executor, so
    // this thread's executor slot is left open for it.
    HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        width,
        height,
        MinimalTestTheme::default(),
    )
}

#[test]
fn patch_free_capture_does_not_blank_color_emoji() {
    let mut runtime = runtime_for(emoji_beside_blurred_box(), 160, 96);
    let start = Instant::now();

    // The first painted frame proves the emoji rasterized at all — the
    // assertion would be vacuous if the subset face never drew.
    let first = runtime
        .pump_at(true, start)
        .snapshot
        .expect("a pumped frame must produce a snapshot");
    assert!(
        count_emoji_pixels(&first) > 0,
        "the emoji glyph must paint before the bug can blank it"
    );

    // Every later frame re-runs the patch-free capture on the shared
    // renderer. On the buggy atlas bookkeeping each of those frames frees the
    // persistent atlas while the image cache still claims residency, so the
    // emoji's atlas texels are gone for good.
    let mut snapshot: Option<HeadlessSnapshot> = None;
    for frame in 1..12 {
        let result = runtime.pump_at(true, start + Duration::from_millis(16 * frame));
        snapshot = result.snapshot.or(snapshot);
    }
    let later = snapshot.expect("a pumped frame must produce a snapshot");
    assert!(
        count_emoji_pixels(&later) > 0,
        "a patch-free subtree capture must not wipe the emoji's atlas images"
    );
}
