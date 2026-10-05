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
    /// The §7.1 context each node was last laid out against, keyed by its
    /// `RenderId`. Side-stored here rather than on the node so node payloads
    /// keep their previous size: `insert` overwrites the existing slot in
    /// place, so a per-frame re-layout pays no new allocation. Cleared as
    /// each whole-tree layout pass begins (`window_root_layout`), so dropped
    /// nodes leave nothing behind.
    pub(crate) safe_area_records: rustc_hash::FxHashMap<RenderId, SafeAreaLayout>,
}

impl HydroState {
    pub(crate) fn new(family_resolution: FontFamilyResolution) -> Self {
        Self {
            text: Arc::new(TextMeasureService::new(family_resolution)),
            measurement: MeasurementCaches::default(),
            counters: FrameWorkCounters::default(),
            safe_area_records: rustc_hash::FxHashMap::default(),
        }
    }
    /// Records the §7.1 context a node laid out against (`None` clears it, so
    /// a node moved into a scroll surface's context-free content keeps no
    /// stale record). A `Some` reuses the occupied slot's storage — no new
    /// allocation per re-layout.
    pub(crate) fn record_safe_area(
        &mut self,
        render_id: RenderId,
        safe_area: Option<SafeAreaLayout>,
    ) {
        match safe_area {
            Some(area) => {
                self.safe_area_records.insert(render_id, area);
            }
            None => {
                self.safe_area_records.remove(&render_id);
            }
        }
    }

    /// The §7.1 context this node was last laid out against, if any.
    pub(crate) fn recorded_safe_area(&self, render_id: RenderId) -> Option<&SafeAreaLayout> {
        self.safe_area_records.get(&render_id)
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
