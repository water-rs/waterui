//! Filtered-view plumbing: the `filtrate::Effect` adapter the engine runs on
//! its render thread, and the UI-side runtime a filtered mount owns while its
//! layer stays live.
//!
//! A `FilteredView` carries a declarative [`AnyEffect`] — a recipe, not a GPU
//! object. [`EngineEffect`] bridges the two worlds: the recipe is `Send`, the
//! built effect is not, so the adapter is a cell that holds the recipe while
//! it crosses to the render thread, builds it inside `setup` (which the engine
//! drives there), and delegates every later call to the built
//! [`ErasedEffect`].
//!
//! [`FilteredRuntime`] is what a mount keeps on the UI side: the unbuilt
//! source until registration, the engine `Filter` handle once registered (its
//! drop unregisters the effect), and the [`ParamGuards`] keeping the filter's
//! reactive parameter subscriptions alive exactly as long as the filter can
//! write them.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cherenkov::Filter;
use waterui_graphics::filter_view::ErasedEffect;
use waterui_graphics::filtrate::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderResult,
    EffectSetupResult,
};
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
pub struct AppliedFilterMetrics {
    /// `encode_render` calls observed this frame.
    encoded: AtomicU32,
    /// CPU nanoseconds spent inside `encode_render` this frame.
    effect_nanos: AtomicU64,
}

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

/// A `filtrate::Effect` that builds itself from an [`AnyEffect`] inside
/// `setup`, on the engine's render thread.
///
/// # `Send` invariant
///
/// The value crosses threads exactly once, when [`crate::engine::GpuEngine`]'s
/// effect registration ships it to the render thread. At that moment `built`
/// is still `None` — the only fields set are `source` (an `AnyEffect`, which
/// is `Send`), `redraw` (an `Arc` callback, also `Send`) and `metrics` (atomics).
/// `built` is populated inside `setup`, which the engine only calls on its
/// render thread, and the value never crosses back.
pub struct EngineEffect {
    /// The unbuilt declarative effect, taken when `setup` runs.
    source: Option<AnyEffect>,
    /// The built render-side effect — populated by `setup`, only ever touched
    /// on the render thread.
    built: Option<Box<dyn ErasedEffect>>,
    /// A redraw callback that arrived before the effect was built, forwarded
    /// at setup.
    redraw: Option<EffectRedrawCallback>,
    /// The frame telemetry every `encode_render` contributes to.
    metrics: Arc<AppliedFilterMetrics>,
}

// SAFETY: `built` is `None` for the whole window in which the value may move
// across threads; every field set before that is `Send`. The `non_send_fields`
// allow acknowledges what this block states: `built` is the `!Send` field the
// safety argument covers.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for EngineEffect {}

impl EngineEffect {
    /// Wraps a declarative effect source for engine registration, feeding
    /// `metrics` on every encode.
    pub(crate) fn new(source: AnyEffect, metrics: Arc<AppliedFilterMetrics>) -> Self {
        Self {
            source: Some(source),
            built: None,
            redraw: None,
            metrics,
        }
    }
}

impl Effect for EngineEffect {
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        match &mut self.built {
            Some(built) => built.set_redraw_callback(callback),
            None => self.redraw = Some(callback),
        }
    }

    // The returned future is `!Send` because the built effect itself is
    // `!Send` — effect setup runs on the render executor, which is
    // single-threaded by design.
    #[allow(clippy::future_not_send)]
    fn setup(&mut self, ctx: &EffectContext) -> impl Future<Output = EffectSetupResult> {
        let mut built = self
            .source
            .take()
            .expect("hydrolysis filter effect set up without a source")
            .build();
        if let Some(callback) = self.redraw.take() {
            built.set_redraw_callback(callback);
        }
        self.built = Some(built);
        async move {
            self.built
                .as_mut()
                .expect("hydrolysis filter effect lost during setup")
                .setup(ctx)
                .await
        }
    }

    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        let started_at = Instant::now();
        let result = self
            .built
            .as_mut()
            .expect("hydrolysis filter effect rendered before setup")
            .encode_render(input, output, encoder);
        self.metrics.record(started_at.elapsed());
        result
    }

    fn redraw_hint(&self) -> bool {
        self.built.as_ref().is_some_and(|built| built.redraw_hint())
    }
}

/// UI-side state of one mounted `FilteredView`: owns the effect source until
/// the engine registers it, then the `Filter` handle for the mount's life.
///
/// Holding `guards` here — rather than inside the registered effect — keeps
/// the nami subscriptions feeding the filter's parameter slots alive for as
/// long as the layer can draw, and tears them down when the node is dropped.
pub struct FilteredRuntime {
    source: Option<AnyEffect>,
    filter: Option<Filter>,
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

    /// The engine filter this mount installs on its wrapper layer.
    ///
    /// Registers the effect on first call; the returned reference borrows the
    /// stored handle, so the caller installs `tx[layer].filter(&filter)`
    /// inside the surface edit this call participates in.
    pub(crate) fn filter(
        &mut self,
        engine: &crate::engine::GpuEngine,
        metrics: &Arc<AppliedFilterMetrics>,
    ) -> &Filter {
        if self.filter.is_none() {
            let source = self
                .source
                .take()
                .expect("hydrolysis filtered mount without an effect source");
            self.filter = Some(engine.effect(EngineEffect::new(source, Arc::clone(metrics))));
        }
        self.filter
            .as_ref()
            .expect("hydrolysis filtered mount lost its filter handle")
    }
}

impl crate::renderer::HydrolysisRenderer {
    /// Applied filters dispatched in the last rendered frame, as
    /// `(filters encoded, capture µs, effect encode µs)`.
    ///
    /// The capture leg is `0`: the engine captures a filtered subtree inside
    /// the same render pass as the effect encode and reports no per-phase
    /// split, so only the encode leg remains measurable.
    pub(crate) fn applied_filter_stats(&self) -> (u32, u64, u64) {
        (
            self.frame_applied_filter_count,
            0,
            crate::renderer::frame::duration_micros_u64(self.frame_applied_filter_effect),
        )
    }
}
