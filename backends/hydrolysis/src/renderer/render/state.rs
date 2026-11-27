// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::sync::Arc;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState<E: crate::text::TextEngine> {
    /// Thread-safe text service built on the session's text engine, shared
    /// with worker-thread layout measurement via `Arc`. See [`TextService`].
    pub(crate) text: Arc<TextService<E>>,
    pub(crate) text_scenes: crate::text::TextScenes,
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
    pub(in crate::renderer) layout_dependencies: Option<LayoutDependencies<E>>,
    /// Nesting depth of active [`RenderNode::layout_in`] calls; size reports
    /// queued during layout flush once the outermost pass completes so watchers
    /// never re-enter an in-flight layout or borrow active tree `RefCell`s.
    pub(in crate::renderer) layout_depth: usize,
    /// Committed placed sizes recorded by `WrapperEffect::ReportSize` during
    /// the current layout pass, drained when `layout_depth` returns to zero.
    pub(in crate::renderer) pending_size_reports:
        Vec<(waterui_layout::ReportSize, waterui_core::layout::Size)>,
    /// Advances whenever the session's fonts gain faces after text was
    /// shaped — a web page's faces arriving after its first frame. Every text
    /// measurement reads it through [`Self::measuring_text`], so a cached
    /// layout that measured text is laid out again when a face arrives.
    pub(crate) font_revision: nami::Binding<u64>,
    #[cfg(all(target_vendor = "apple", hydrolysis_cherenkov))]
    pub(crate) sf_symbols: crate::widgets::visual::sf_symbol::SfSymbolCache,
}

impl<E: crate::text::TextEngine> HydroState<E> {
    pub(crate) fn new(text: E) -> Self {
        Self {
            text: Arc::new(TextService::new(text)),
            text_scenes: crate::text::TextScenes::default(),
            measurement: MeasurementCaches::default(),
            counters: FrameWorkCounters::default(),
            layout_dependencies: None,
            layout_depth: 0,
            pending_size_reports: Vec::new(),
            font_revision: nami::Binding::new(0),
            #[cfg(all(target_vendor = "apple", hydrolysis_cherenkov))]
            sf_symbols: crate::widgets::visual::sf_symbol::SfSymbolCache::new(),
        }
    }

    /// The text service, for a measurement: the read records the session's
    /// font revision as a dependency of the layout pass in flight.
    pub(crate) fn measuring_text(&mut self) -> &Arc<TextService<E>> {
        let font_revision = self.font_revision.clone();
        let _ = self.measure_signal(&font_revision);
        &self.text
    }
}

impl<E: crate::text::TextEngine> core::fmt::Debug for HydroState<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}
