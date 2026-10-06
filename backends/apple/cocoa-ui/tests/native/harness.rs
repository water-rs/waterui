//! Shared scaffolding for the native cases: the real main-thread marker.

use cocoa_ui::MainThreadMarker;
use libtest_mimic::Trial;

/// The suite's trial list, prefixed with the check the whole harness
/// depends on: the marker is real.
pub fn trials(mut cases: Vec<Trial>) -> Vec<Trial> {
    cases.insert(
        0,
        Trial::test("the_main_thread_marker_is_real", || {
            assert!(
                cocoa_ui::MainThreadMarker::new().is_some(),
                "a native case ran off the process's main thread"
            );
            Ok(())
        }),
    );
    cases
}

/// The marker a case runs under — `MainThreadMarker::new()` on the thread
/// the case executes on, so a case that drifted off the main thread fails
/// here instead of being handed a forged token.
///
/// # Panics
///
/// If the calling thread is not the process's main thread.
pub fn marker() -> MainThreadMarker {
    MainThreadMarker::new().expect("the native suite runs every case on the process's main thread")
}

pub use cocoa_ui::native_test::pump_main_until;

/// One real turn of the main queue — enqueues a marker through the same
/// dispatch channel completion hops use and returns only once the
/// queue actually ran it. A dead main queue fails the case instead of
/// passing as an idle wait.
///
/// # Panics
///
/// If the marker never runs within the deadline.
pub fn pump_main_turn() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    cocoa_ui::main_queue::enqueue(move |_mtm| flag.store(true, Ordering::Relaxed));
    assert!(
        pump_main_until(2.0, || ran.load(Ordering::Relaxed)),
        "an enqueued main-queue marker never ran — the main queue is not servicing"
    );
}

/// A shared Metal destination for the capture cases — `.shared` so a
/// case reads the rendered pixels back.
///
/// # Panics
///
/// If this runner has no Metal device — the capture suite cannot check
/// what it exists to check, so a missing device is a reported
/// environment block, not a silently skipped case.
pub fn capture_target() -> cocoa_ui::objc2::rc::Retained<
    cocoa_ui::objc2::runtime::ProtocolObject<dyn cocoa_ui::objc2_metal::MTLTexture>,
> {
    use cocoa_ui::objc2_metal::{
        MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat, MTLStorageMode,
        MTLTextureDescriptor, MTLTextureUsage,
    };
    let device = MTLCreateSystemDefaultDevice()
        .expect("native capture cases require a Metal device on this runner");
    // SAFETY: a 2D texture descriptor is always valid to construct.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::BGRA8Unorm,
            400,
            400,
            false,
        )
    };
    descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
    descriptor.setStorageMode(MTLStorageMode::Shared);
    device
        .newTextureWithDescriptor(&descriptor)
        .expect("a capture texture")
}

/// The raw texels of `target` — 4 bytes each, BGRA order, row-major.
/// A case then counts the pixels it placed in the fixture rather than
/// asking whether anything at all landed.
pub fn readback(
    target: &cocoa_ui::objc2::runtime::ProtocolObject<dyn cocoa_ui::objc2_metal::MTLTexture>,
) -> Vec<u8> {
    use cocoa_ui::objc2_metal::{MTLOrigin, MTLRegion, MTLSize, MTLTexture};
    let (width, height) = (target.width(), target.height());
    let mut bytes = vec![0u8; width * height * 4];
    // SAFETY: `bytes` is `width*4` per row for `height` rows and `target`
    // is `.shared` — the region read stays in bounds.
    unsafe {
        target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
            std::ptr::NonNull::new(bytes.as_mut_ptr().cast())
                .expect("the texel buffer is non-null"),
            width * 4,
            MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width,
                    height,
                    depth: 1,
                },
            },
            0,
        );
    }
    bytes
}

/// Texels whose B, G, R, A channels all fall inside the inclusive
/// `min`/`max` ranges — the broad content predicate a capture case uses
/// to prove the fixture's own colors landed (an opaque all-black or
/// all-alpha frame cannot satisfy a colored predicate).
pub fn count_pixels(bytes: &[u8], min: [u8; 4], max: [u8; 4]) -> usize {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|texel| {
            texel
                .iter()
                .zip(min)
                .zip(max)
                .all(|((c, lo), hi)| *c >= lo && *c <= hi)
        })
        .count()
}

/// A capture fence split into "completed" and "succeeded": 0 = pending,
/// 1 = ok, 2 = completed-but-failed — so a case can never misread an
/// unsuccessful completion as a fence that never arrived.
pub fn fence_flag() -> (
    std::sync::Arc<std::sync::atomic::AtomicU8>,
    impl Fn(bool) + Send + 'static,
) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    let flag = Arc::new(AtomicU8::new(0));
    let f = Arc::clone(&flag);
    (flag, move |ok| {
        f.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
    })
}

/// Pumps until `flag` leaves pending and answers its state — the caller
/// names which capture the fence belonged to in the failure.
pub fn await_fence(flag: &std::sync::atomic::AtomicU8, what: &str) {
    use std::sync::atomic::Ordering;
    assert!(
        pump_main_until(5.0, || flag.load(Ordering::Relaxed) != 0),
        "the {what} capture fence never completed"
    );
    assert_eq!(
        flag.load(Ordering::Relaxed),
        1,
        "the {what} capture completed unsuccessfully"
    );
}
