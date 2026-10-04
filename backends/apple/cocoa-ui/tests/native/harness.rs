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

/// Pumps the main run loop in small turns until `until` answers or
/// `seconds` elapse — how a synchronous case awaits work enqueued on the
/// main queue, like a Metal completion's main-thread hop or a later-turn
/// lifecycle check. Answers whether `until` was reached.
pub fn pump_main_until(seconds: f64, until: impl Fn() -> bool) -> bool {
    use cocoa_ui::objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
    let deadline = NSDate::dateWithTimeIntervalSinceNow(seconds);
    while !until() && deadline.timeIntervalSinceNow() > 0.0 {
        // SAFETY: `NSDefaultRunLoopMode` is a system-owned run-loop mode.
        NSRunLoop::currentRunLoop().runMode_beforeDate(
            unsafe { NSDefaultRunLoopMode },
            &NSDate::dateWithTimeIntervalSinceNow(0.02),
        );
    }
    until()
}

/// One idle turn on the main queue — events enqueued between statements
/// of a case land before the next assertion.
pub fn pump_main_turn() {
    pump_main_until(0.1, || false);
}

/// A shared Metal destination for the capture cases — `.shared` so a
/// case reads the rendered pixels back. `None` on a runner without Metal.
pub fn capture_target() -> Option<
    cocoa_ui::objc2::rc::Retained<
        cocoa_ui::objc2::runtime::ProtocolObject<dyn cocoa_ui::objc2_metal::MTLTexture>,
    >,
> {
    use cocoa_ui::objc2_metal::{
        MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat, MTLStorageMode,
        MTLTextureDescriptor, MTLTextureUsage,
    };
    let device = MTLCreateSystemDefaultDevice()?;
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
    Some(
        device
            .newTextureWithDescriptor(&descriptor)
            .expect("a capture texture"),
    )
}

/// The rendered texels of `target` — nonzero means real content landed.
pub fn nonzero_texels(
    target: &cocoa_ui::objc2::runtime::ProtocolObject<dyn cocoa_ui::objc2_metal::MTLTexture>,
) -> usize {
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
    bytes.iter().filter(|byte| **byte != 0).count()
}
