//! Filtered-view plumbing: the `filtrate::Effect` adapter the engine runs on
//! its render thread, and the UI-side runtime a filtered mount owns while its
//! layer stays live.
//!
//! A `FilteredView` carries a declarative [`AnyEffect`] — a recipe, not a GPU
//! object. [`EngineEffect`] bridges the two worlds: the recipe is `Send`, the
//! built effect is not, so the adapter is a cell that holds the recipe while
//! it crosses to the render thread, builds it inside `setup` (which the engine
//! drives there), and delegates every later call to the built
//! [`waterui_graphics::filter_view::ErasedEffect`].
//!
//! [`FilteredRuntime`] is what a mount keeps on the UI side: the unbuilt
//! source until registration, the engine `Filter` handle once registered (its
//! drop unregisters the effect), and the [`ParamGuards`] keeping the filter's
//! reactive parameter subscriptions alive exactly as long as the filter can
//! write them.

#[cfg(hydrolysis_gpu_painter)]
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
#[cfg(hydrolysis_gpu_painter)]
use std::time::Duration;

use super::mount::target::FilterHandle;
use waterui_graphics::{AnyEffect, ParamGuards};

/// Per-frame applied-filter telemetry shared between the UI side and the
/// engine's render thread.
///
/// The UI side resets the counters before `Engine::render` and reads them
/// after it returns — the render is synchronous — while `EngineEffect` calls
/// accumulate into them from the render thread, so the cells are atomic.
///
/// The engine runs a filtered layer's subtree capture inside the same render
/// pass as the effect encode and reports no per-phase split for it, so there
/// is no separate capture number to measure: this cell counts the encodes it
/// can see and their CPU time only.
#[derive(Debug, Default)]
#[cfg(hydrolysis_gpu_painter)]
pub struct AppliedFilterMetrics {
    /// `encode_render` calls observed this frame.
    encoded: AtomicU32,
    /// CPU nanoseconds spent inside `encode_render` this frame.
    effect_nanos: AtomicU64,
}

#[cfg(hydrolysis_gpu_painter)]
impl AppliedFilterMetrics {
    /// Zeroes the frame counters before the engine renders.
    pub(crate) fn reset(&self) {
        self.encoded.store(0, Ordering::Relaxed);
        self.effect_nanos.store(0, Ordering::Relaxed);
    }

    /// Filters that encoded this frame and their combined encode time.
    pub(crate) fn snapshot(&self) -> (u32, Duration) {
        (
            self.encoded.load(Ordering::Relaxed),
            Duration::from_nanos(self.effect_nanos.load(Ordering::Relaxed)),
        )
    }

    /// Records one completed effect encode.
    fn record(&self, elapsed: Duration) {
        self.encoded.fetch_add(1, Ordering::Relaxed);
        self.effect_nanos.fetch_add(
            crate::num_cast::u128_as_u64(elapsed.as_nanos()),
            Ordering::Relaxed,
        );
    }
}

#[cfg(hydrolysis_gpu_painter)]
mod gpu;

#[cfg(hydrolysis_gpu_painter)]
pub use gpu::EngineEffect;

/// UI-side state of one mounted `FilteredView`: owns the effect source until
/// the engine registers it, then the `Filter` handle for the mount's life.
///
/// Holding `guards` here — rather than inside the registered effect — keeps
/// the nami subscriptions feeding the filter's parameter slots alive for as
/// long as the layer can draw, and tears them down when the node is dropped.
pub struct FilteredRuntime {
    source: Option<AnyEffect>,
    filter: Option<FilterHandle>,
    _guards: ParamGuards,
}

impl std::fmt::Debug for FilteredRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilteredRuntime").finish_non_exhaustive()
    }
}

impl FilteredRuntime {
    /// A runtime holding `view`'s effect source and parameter guards.
    pub(crate) const fn new(effect: AnyEffect, guards: ParamGuards) -> Self {
        Self {
            source: Some(effect),
            filter: None,
            _guards: guards,
        }
    }

    /// The filter handle this mount installs on its wrapper layer.
    ///
    /// Registers the effect on first call and returns its id — the runtime
    /// retains the registration until the mount drops it.
    pub(crate) fn filter(
        &mut self,
        register: impl FnOnce(AnyEffect) -> FilterHandle,
    ) -> cherenkov_record::FilterId {
        if self.filter.is_none() {
            let source = self
                .source
                .take()
                .expect("hydrolysis filtered mount without an effect source");
            self.filter = Some(register(source));
        }
        self.filter
            .as_ref()
            .expect("hydrolysis filtered mount lost its filter handle")
            .id()
    }
}

impl<E: crate::text::TextEngine> crate::renderer::HydrolysisRenderer<E> {
    /// Applied filters dispatched in the last rendered frame, as
    /// `(filters encoded, capture µs, effect encode µs)`.
    ///
    /// The capture leg is `0`: the engine captures a filtered subtree inside
    /// the same render pass as the effect encode and reports no per-phase
    /// split, so only the encode leg remains measurable.
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn applied_filter_stats(&self) -> (u32, u64, u64) {
        (
            self.frame_applied_filter_count,
            0,
            crate::renderer::frame::duration_micros_u64(self.frame_applied_filter_effect),
        )
    }
}
