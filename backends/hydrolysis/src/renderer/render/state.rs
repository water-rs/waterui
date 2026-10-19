// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::sync::Arc;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    /// Thread-safe text service built on the session's text engine, shared
    /// with worker-thread layout measurement via `Arc`. See [`TextService`].
    pub(crate) text: Arc<TextService<SessionTextEngine>>,
    pub(crate) measurement: MeasurementCaches,
    /// Per-frame work counters for the fine-grained frame model; see
    /// [`crate::renderer::FrameWorkCounters`].
    pub(crate) counters: FrameWorkCounters,
    /// The layout-signal dependency set being collected by the one
    /// `BuiltSubview` layout pass in flight — `Some` only for the duration of
    /// that pass, installed by `layout_if_needed` and taken back before it
    /// returns. `watch_signal` and `measure_signal` record every signal the
    /// pass reads into it; outside a pass it stays `None` (a pass never
    /// nests: `RenderNode::layout` does not lay out a retained sub-view).
    pub(in crate::renderer) layout_dependencies: Option<LayoutDependencies>,
    /// Advances whenever the session's fonts gain faces after text was
    /// shaped — a web page's faces arriving after its first frame. Every text
    /// measurement reads it through [`Self::measuring_text`], so a cached
    /// layout that measured text is laid out again when a face arrives.
    pub(crate) font_revision: nami::Binding<u64>,
}

impl HydroState {
    pub(crate) fn new(text: SessionTextEngine) -> Self {
        Self {
            text: Arc::new(TextService::new(text)),
            measurement: MeasurementCaches::default(),
            counters: FrameWorkCounters::default(),
            layout_dependencies: None,
            font_revision: nami::Binding::container(0),
        }
    }

    /// The text service, for a measurement: the read records the session's
    /// font revision as a dependency of the layout pass in flight.
    pub(crate) fn measuring_text(&mut self) -> &Arc<TextService<SessionTextEngine>> {
        let font_revision = self.font_revision.clone();
        let _ = self.measure_signal(&font_revision);
        &self.text
    }
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}
