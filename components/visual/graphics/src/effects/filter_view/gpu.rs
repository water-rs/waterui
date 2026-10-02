//! The GPU lowering of [`AnyEffect`]: a filter runs through `filtrate`'s
//! [`Executor`], and an arbitrary [`Effect`] runs as itself.

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;

use cherenkov::RenderTransfer;
use filtrate::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderResult,
    EffectSetupResult, Executor, Filter,
};
use waterui_core::{AnyView, View};

use super::{AnyEffect, EffectSource, FilteredView, OutputSizeState, ParamGuards};

type BoxedSetup<'a> = Pin<Box<dyn Future<Output = EffectSetupResult> + 'a>>;

/// Object-safe [`Effect`], built on the render thread by [`AnyEffect::build`].
///
/// `Effect::setup` returns an `impl Future`, so the trait itself cannot be
/// boxed; this form boxes the future. It is not `Send`: `filtrate`'s
/// [`Executor`] lives on the thread that built it.
pub trait ErasedEffect {
    /// Installs the callback the effect fires when it needs another frame.
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback);
    /// Creates pipelines and resources; once, before the first render.
    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> BoxedSetup<'a>;
    /// Encodes one frame of effect work.
    ///
    /// # Errors
    /// The effect's own render error, surfaced by the host as a frame failure.
    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult;
    /// Resolves the output texture dimensions for an input texture.
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32);
    /// Whether the effect wants another frame (an animating parameter).
    fn redraw_hint(&self) -> bool;
}

impl<E: Effect> ErasedEffect for E {
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        Effect::set_redraw_callback(self, callback);
    }

    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> BoxedSetup<'a> {
        Box::pin(Effect::setup(self, ctx))
    }

    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        Effect::encode_render(self, input, output, encoder)
    }

    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        Effect::output_size(self, input_width, input_height)
    }

    fn redraw_hint(&self) -> bool {
        Effect::redraw_hint(self)
    }
}

/// An arbitrary GPU effect, erased for the render thread.
pub(super) trait GpuEffect: RenderTransfer {
    fn build(self: Box<Self>, output_size: OutputSizeState) -> Box<dyn ErasedEffect>;
}

impl<E: Effect + RenderTransfer> GpuEffect for E {
    fn build(self: Box<Self>, output_size: OutputSizeState) -> Box<dyn ErasedEffect> {
        Box::new(OutputSizedEffect {
            effect: *self,
            output_size,
        })
    }
}

/// Lowers a filter into `filtrate`'s executor, which resizes its output only
/// when the view declared an output size.
pub(super) fn lower_filter<F: Filter>(
    filter: F,
    output_size: OutputSizeState,
) -> Box<dyn ErasedEffect> {
    if output_size.declared().is_some() {
        let policy = output_size.clone();
        Box::new(OutputSizedEffect {
            effect: Executor::new(filter).with_output_size(move |width, height| {
                policy
                    .declared()
                    .map_or((width, height), |size| size.compute(width, height))
            }),
            output_size,
        })
    } else {
        Box::new(OutputSizedEffect {
            effect: Executor::new(filter),
            output_size,
        })
    }
}

/// Applies an optional output-size declaration while preserving the wrapped effect.
struct OutputSizedEffect<E> {
    effect: E,
    output_size: OutputSizeState,
}

impl<E: Effect> Effect for OutputSizedEffect<E> {
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        self.output_size.declared().map_or_else(
            || self.effect.output_size(input_width, input_height),
            |size| size.compute(input_width, input_height),
        )
    }

    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        self.output_size.set_change_callback(callback.clone());
        self.effect.set_redraw_callback(callback);
    }

    async fn setup(&mut self, ctx: &EffectContext<'_>) -> EffectSetupResult {
        self.effect.setup(ctx).await
    }

    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        self.effect.encode_render(input, output, encoder)
    }

    fn redraw_hint(&self) -> bool {
        self.effect.redraw_hint()
    }
}

impl AnyEffect {
    /// Erases an arbitrary GPU effect. It has no portable
    /// [`description`](Self::description), so only a backend that runs
    /// effects on a GPU can realize it.
    pub fn new(effect: impl Effect + RenderTransfer) -> Self {
        Self::from_source(EffectSource::Gpu(Box::new(effect)))
    }

    /// Builds the effect on the render thread: a filter lowered into
    /// `filtrate`'s [`Executor`], or the GPU effect itself.
    #[must_use]
    pub fn build(self) -> Box<dyn ErasedEffect> {
        match self.source {
            EffectSource::Filter(filter) => filter.build(self.output_size),
            EffectSource::Gpu(effect) => effect.build(self.output_size),
        }
    }
}

impl FilteredView {
    /// Applies an arbitrary GPU effect to `view`.
    pub fn new(view: impl View, effect: impl Effect + RenderTransfer) -> Self {
        Self {
            content: AnyView::new(view),
            effect: AnyEffect::new(effect),
            guards: ParamGuards::default(),
        }
    }
}
