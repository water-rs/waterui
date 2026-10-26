//! The GPU adapter for [`FilteredRuntime`](super::FilteredRuntime): an
//! `EngineEffect` registers the declarative source on Cherenkov's engine.

use std::future::Future;
use std::sync::Arc;

use waterui_graphics::AnyEffect;
use waterui_graphics::filter_view::ErasedEffect;
use waterui_graphics::filtrate::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderResult,
    EffectSetupResult, RetainedMemory,
};

use crate::time::Instant;

use super::AppliedFilterMetrics;

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

    fn retained_memory(&self) -> RetainedMemory {
        self.built
            .as_ref()
            .map_or_else(RetainedMemory::default, |built| built.retained_memory())
    }

    fn trim(&mut self) {
        if let Some(built) = &mut self.built {
            built.trim();
        }
    }
}
