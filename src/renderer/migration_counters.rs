//! Per-frame counters for the Cherenkov migration acceptance fixtures.
//!
//! The migration (water-rs/hydrolysis#205) changes *where* drawing work is
//! recorded: the Vello era walked the retained tree and re-encoded every
//! view into a fresh scene on every awake frame; the target records static
//! structure once and pushes only live-operand updates. These counters make
//! both ends of that spectrum measurable from a
//! [`crate::runner::FrameCounters`] — the Vello-era values are nonzero in
//! ways the engine-era values must not be
//! (`semantic builds`, `recorded view contents`, `font and image
//! registrations`, `host wakeups`), and the engine-era values start at zero
//! and become the acceptance signal (`live operand updates`, `layer
//! creations`, `layer removals`).
//!
//! A counter counts the operation the runner performed this frame, at the
//! point the work was done — not what a future engine would do. Sites that
//! cannot reach [`HydroState`](crate::renderer::HydroState) (detached wake
//! tasks, free functions) are counted where their effect is drained.

/// One frame's migration counters, accumulated during the frame and snapshotted
/// into [`crate::runner::FrameCounters`] at pump end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrationCounters {
    /// View-body dispatches into retained nodes this frame
    /// ([`RenderNode::build`] calls). A full rebuild counts every mounted
    /// subtree once; a frame that only patches counts zero.
    pub(crate) semantic_builds: u64,
    /// Structural patch applications against the retained tree this frame: a
    /// `Dynamic` subtree swap, a collection reconcile, a lazy mount or
    /// unmount. Scoped to the actual mutation, not to the nodes that carried
    /// the `changed` bit back up.
    pub(crate) structural_patches: u64,
    /// Uncached view measurements this frame — the `measure_body` calls that
    /// actually ran a view's size probe. Cache hits answer without one.
    pub(crate) measure_calls: u64,
    /// Node placement passes this frame ([`RenderNode::layout`] entries).
    pub(crate) layout_calls: u64,
    /// View drawing contents recorded into the frame this frame: each leaf
    /// draw emission (colour fill, text encode, scene-view build, GPU-surface
    /// flush, theme widget render). The retained-engine name for this is
    /// "content recordings", and the R-work must drive it to zero on steady
    /// frames.
    pub(crate) recorded_view_contents: u64,
    /// Live-operand updates reaching the engine this frame. Always zero until
    /// a recording engine exists — the signal becomes meaningful with the
    /// retained-engine activation.
    pub(crate) live_operand_updates: u64,
    /// Retained engine layer mounts this frame. Zero until retained engine
    /// layers exist.
    pub(crate) layer_creations: u64,
    /// Retained engine layer removals this frame. Zero until retained engine
    /// layers exist.
    pub(crate) layer_removals: u64,
    /// Font/glyph payloads registered into the frame's encoding this frame —
    /// each glyph-run submission registers a font entry. The engine-era
    /// acceptance is that a font is not re-registered per frame, so a frame
    /// with unchanged text drives this to zero.
    pub(crate) font_registrations: u64,
    /// Image payloads registered into the frame this frame — validated
    /// image-brush ingests and direct image draws. Same engine-era
    /// acceptance: an image is not re-registered per frame.
    pub(crate) image_registrations: u64,
    /// GPU submissions this frame: every `wgpu::Queue::submit` the render
    /// path issues (filter encoders, effect inputs and the like; the
    /// engine's own submissions are its internals, not counted here).
    pub(crate) gpu_submissions: u64,
    /// Wake requests consumed by the host plumbing this frame: each
    /// `signals.take_*` drain that found a pending frame request, each GPU
    /// surface whose out-of-band dirty flag drained, and each
    /// `platform.request_redraw` issued to the host OS. Counting the drained
    /// side deduplicates bursts (many requests set one flag) and counts the
    /// wakes a detached task raised where it cannot reach this struct — which
    /// is what "no work while idle" measures.
    pub(crate) host_wakeups: u64,
}

impl MigrationCounters {
    /// Zero every counter at a frame boundary.
    pub fn reset_frame(&mut self) {
        *self = Self::default();
    }

    /// Serializes the counters into a metrics map for frame-profile output.
    pub fn record_into(&self, metrics: &mut std::collections::BTreeMap<&'static str, u64>) {
        metrics.extend([
            ("semantic_builds", self.semantic_builds),
            ("structural_patches", self.structural_patches),
            ("measure_calls", self.measure_calls),
            ("layout_calls", self.layout_calls),
            ("recorded_view_contents", self.recorded_view_contents),
            ("live_operand_updates", self.live_operand_updates),
            ("layer_creations", self.layer_creations),
            ("layer_removals", self.layer_removals),
            ("font_registrations", self.font_registrations),
            ("image_registrations", self.image_registrations),
            ("gpu_submissions", self.gpu_submissions),
            ("host_wakeups", self.host_wakeups),
        ]);
    }
}
