// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::sync::Arc;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    /// Thread-safe text shaping/measurement, shared with worker-thread layout
    /// measurement via `Arc`. See [`TextMeasureService`].
    pub(crate) text: Arc<TextMeasureService>,
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
}

impl HydroState {
    pub(crate) fn new(family_resolution: FontFamilyResolution) -> Self {
        Self {
            text: Arc::new(TextMeasureService::new(family_resolution)),
            measurement: MeasurementCaches::default(),
            counters: FrameWorkCounters::default(),
            layout_dependencies: None,
        }
    }
}

impl Default for HydroState {
    fn default() -> Self {
        Self::new(FontFamilyResolution::Lenient)
    }
}

impl HydroState {
    /// Mutable access to the registered fonts for startup font registration.
    ///
    /// Requires that no worker has cloned the [`TextMeasureService`] yet, which
    /// holds during single-threaded setup before the first render/measure.
    pub(crate) fn text_fonts_mut(&mut self) -> &mut parley::FontContext {
        Arc::get_mut(&mut self.text)
            .expect(
                "hydrolysis font registration requires unique TextMeasureService ownership \
                 before rendering",
            )
            .fonts_mut()
    }
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}
