//! The HWUI painter's per-frame work counters under `frame-profile`.
//!
//! The Android bench harness (`bench/android`) attributes a failed
//! HWUI-versus-Compose metric to the work the painter did: JNI calls and the
//! command bytes crossing them, nodes re-recorded, text layouts shaped and
//! fonts registered. [`FrameStats`] accumulates those between frames and
//! [`FrameStats::report`] logs and zeroes them once per presented frame.
//!
//! Without the feature the type is zero-sized and every method is empty, so
//! the call sites carry no `cfg` and a shipped build pays nothing.

#[cfg(feature = "frame-profile")]
mod counting {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug, Default)]
    struct Counts {
        jni_calls: AtomicU64,
        command_bytes: AtomicU64,
        nodes_rerecorded: AtomicU64,
        text_shape_misses: AtomicU64,
        font_registrations: AtomicU64,
    }

    /// The counters one HWUI surface's clients share. Text shaping and font
    /// registration run on worker threads, so the counts are atomic;
    /// ordering is irrelevant to a count read once per frame.
    #[derive(Clone, Debug, Default)]
    pub struct FrameStats(Arc<Counts>);

    impl FrameStats {
        /// One call into the JVM.
        pub fn jni_call(&self) {
            self.0.jni_calls.fetch_add(1, Ordering::Relaxed);
        }

        /// `bytes` of encoded command buffer handed to the replay.
        pub fn command_bytes(&self, bytes: usize) {
            self.0
                .command_bytes
                .fetch_add(bytes as u64, Ordering::Relaxed);
        }

        /// One node's display list re-recorded.
        pub fn node_rerecorded(&self) {
            self.0.nodes_rerecorded.fetch_add(1, Ordering::Relaxed);
        }

        /// One text layout shaped through the platform: a shape-cache miss.
        pub fn text_shape_miss(&self) {
            self.0.text_shape_misses.fetch_add(1, Ordering::Relaxed);
        }

        /// One font registered with the platform.
        pub fn font_registration(&self) {
            self.0.font_registrations.fetch_add(1, Ordering::Relaxed);
        }

        /// Logs the counts since the previous report as one logcat line the
        /// harness parses, and zeroes them.
        pub fn report(&self) {
            let counts = &*self.0;
            tracing::info!(
                target: "hydrolysis::frame_profile",
                jni_calls = counts.jni_calls.swap(0, Ordering::Relaxed),
                command_bytes = counts.command_bytes.swap(0, Ordering::Relaxed),
                nodes_rerecorded = counts.nodes_rerecorded.swap(0, Ordering::Relaxed),
                text_shape_misses = counts.text_shape_misses.swap(0, Ordering::Relaxed),
                font_registrations = counts.font_registrations.swap(0, Ordering::Relaxed),
                "hwui frame"
            );
        }
    }
}

#[cfg(not(feature = "frame-profile"))]
mod counting {
    /// The counters, compiled out: a zero-sized handle. Not `Copy`, so call
    /// sites clone it the same way they clone the counting handle.
    #[derive(Clone, Debug, Default)]
    pub struct FrameStats;

    impl FrameStats {
        /// One call into the JVM.
        pub const fn jni_call(&self) {}

        /// `bytes` of encoded command buffer handed to the replay.
        pub const fn command_bytes(&self, _bytes: usize) {}

        /// One node's display list re-recorded.
        pub const fn node_rerecorded(&self) {}

        /// One text layout shaped through the platform.
        pub const fn text_shape_miss(&self) {}

        /// One font registered with the platform.
        pub const fn font_registration(&self) {}

        /// Reports nothing.
        pub const fn report(&self) {}
    }
}

pub use counting::FrameStats;
