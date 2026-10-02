use super::*;
use std::sync::Arc;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    /// Thread-safe text shaping/measurement, shared with worker-thread layout
    /// measurement via `Arc`. See [`TextMeasureService`].
    pub(crate) text: Arc<TextMeasureService>,
    pub(crate) measurement: MeasurementCaches,
    /// Per-frame Cherenkov-migration counters; see
    /// [`crate::renderer::MigrationCounters`].
    pub(crate) counters: MigrationCounters,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            text: Arc::new(TextMeasureService::new()),
            measurement: MeasurementCaches::default(),
            counters: MigrationCounters::default(),
        }
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
