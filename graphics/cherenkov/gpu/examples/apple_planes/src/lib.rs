//! On-device verification harness for cherenkov's Apple
//! system-compositor planes (issue #90) — the iOS counterpart of
//! `gpu/examples/android_planes`.
//!
//! A full-screen `CAMetalLayer`-backed view hosts a cherenkov window
//! surface. One 1920×1080 synthetic video plays on it: the same picture
//! `pattern` produces for Android, here written into `IOSurface`-backed
//! `CVPixelBuffer`s and installed as BGRA `ExternalFrame`s — the frame
//! class `LayerPlanes::shows` admits (opaque `BGRA8Unorm`, sRGB, BT.709).
//! The planner promotes it to an `AVSampleBufferDisplayLayer` plane.
//!
//! Scenarios (`--scenario <name>` launch argument):
//!
//! - `overlay`: the video alone fills the screen and promotes to a
//!   plane once the display layer reports `readyForDisplay`.
//! - `in-engine`: the identical video — same buffers, same producer
//!   cost — plus one empty layer painted above it at reduced opacity.
//!   An empty layer draws nothing, so the picture is pixel-identical,
//!   but a layer above the video that is not known to be opaque is
//!   `Ineligible::TranslucentAbove`: the platform compositor blends in
//!   its own space, which cannot reproduce the engine's linear blend,
//!   so the planner keeps the video composited in-engine. This is the
//!   Apple analogue of `android_planes`' timeline-semaphore acquire — on
//!   Android the *frame contract* blocks promotion with identical
//!   pixels; on Apple every frame-level check lives in `shows`, which
//!   only gates candidacy and emits no verdict, so the harness uses
//!   the smallest tree-level ineligibility that leaves every pixel
//!   untouched.
//!
//! `--paused` produces one frame then stops, leaving a static video.
//! Anything else on the command line is rejected — no silent default.
//!
//! The heartbeat (`log.rs`) logs once per second through `os_log`
//! subsystem `dev.cherenkov.planes`: scenario, frames presented that
//! second, each video layer's plane decision, producer timings, stalls
//! and `NSProcessInfo` thermal state. At `.serious` the producer pauses
//! until the device cools.

mod log;
#[cfg(target_os = "ios")]
mod measurement;
pub mod observe;
#[path = "../../android_planes/src/pattern.rs"]
pub mod pattern;
#[path = "../../planes_common/recorded.rs"]
pub mod recorded;
pub mod scenario;

#[cfg(target_vendor = "apple")]
mod app;
#[cfg(target_vendor = "apple")]
pub mod producer;

#[cfg(target_vendor = "apple")]
pub use app::{
    cherenkov_planes_brightness_dim, cherenkov_planes_brightness_restore, cherenkov_planes_resize,
    cherenkov_planes_start, cherenkov_planes_tick,
};
