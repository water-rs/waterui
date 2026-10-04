//! The reference executor: runs a [`Filter`] on a wgpu texture.
//!
//! It asks the shared composer, `cherenkov-shader`, to compose the filter's
//! stages and takes the reference alternative of the composition: every
//! piece of the composition is one full-screen fragment pass, every spatial
//! stage is materialized, and no colour prefix is folded into a spatial
//! stage's samples. Intermediates are `Rgba16Float`. Stages that operate in
//! sRGB are wrapped in conversions from and back to the working space.
//!
//! It is deliberately simple: it is the behaviour the Cherenkov engine's
//! fused execution is verified against, and it serves consumers that are not
//! user interfaces, such as video processing.

extern crate alloc;

pub mod animation;
mod entry;
mod gpu;
mod plan;
pub mod uniforms;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;
use core::fmt;

use filtrate_core::{Chain, Filter, ParamArray, SpatialFilter, WatchGuard};

use crate::effect::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderError,
    EffectRenderResult, EffectSetupError, EffectSetupResult,
};
use animation::ParamAnimator;
use gpu::Gpu;
pub use gpu::{filterable, sampler};

/// Runs a [`Filter`] on caller-owned wgpu textures.
///
/// Output matches input unless [`Self::with_output_size`] declares otherwise.
/// Hosts allocate their output using [`Effect::output_size`]; the executor
/// allocates its intermediate textures and validates the supplied output.
///
/// Parameters that are reactive ([`FilterParam`](crate::FilterParam)
/// signals) are watched; a change carrying an interpolator animates, and
/// [`Effect::encode_render`] reports whether another frame is needed.
pub struct Executor<F: Filter, S = fn(u32, u32) -> (u32, u32)> {
    filter: F,
    output_size: S,
    resizes_output: bool,
    /// Parameter watcher subscriptions, dropped before the animator whose
    /// channel they feed.
    watcher_guards: Vec<WatchGuard>,
    animator: ParamAnimator,
    gpu: Option<Gpu>,
    /// Sticky setup error: once set, rendering fails fast.
    setup_error: Option<EffectSetupError>,
}

impl<F: Filter, S> fmt::Debug for Executor<F, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Executor")
            .field("animator", &self.animator)
            .field("set_up", &self.gpu.is_some())
            .field("setup_error", &self.setup_error)
            .finish_non_exhaustive()
    }
}

impl<F: Filter> Executor<F> {
    /// An executor for `filter`. Pipelines are built by [`Effect::setup`].
    #[must_use]
    pub fn new(filter: F) -> Self {
        let mut targets = alloc::vec![0.0; <F::Params as ParamArray>::LEN];
        filter.params().write_to(&mut targets);
        let (animator, watcher_guards) =
            ParamAnimator::new(targets, |installer| filter.visit_signals(installer));
        Self {
            filter,
            output_size: |width, height| (width, height),
            resizes_output: false,
            watcher_guards,
            animator,
            gpu: None,
            setup_error: None,
        }
    }
}

impl<F: Filter, S: Fn(u32, u32) -> (u32, u32) + 'static> Executor<F, S> {
    /// Declares the final output size as a function of the input dimensions.
    ///
    /// The callback can read reactive state and is queried for each frame.
    /// Both returned dimensions must be nonzero. Intermediate passes keep the
    /// input resolution; the final pass maps output pixel centres into input
    /// coordinates. Colour-only passes use nearest texels, and spatial passes
    /// retain their declared sampling contract.
    ///
    /// Set this before setup: changing the policy invalidates GPU pipelines.
    #[must_use]
    pub fn with_output_size<T: Fn(u32, u32) -> (u32, u32) + 'static>(
        self,
        output_size: T,
    ) -> Executor<F, T> {
        Executor {
            filter: self.filter,
            output_size,
            resizes_output: true,
            watcher_guards: self.watcher_guards,
            animator: self.animator,
            gpu: None,
            setup_error: None,
        }
    }

    /// The filter this executor runs.
    #[must_use]
    pub const fn filter(&self) -> &F {
        &self.filter
    }

    /// An executor for this executor's filter followed by `filter`, keeping
    /// the installed redraw callback. The new executor needs its own setup.
    #[must_use]
    pub fn then<G: Filter>(self, filter: G) -> Executor<Chain<F, G>, S> {
        let redraw_callback = self.animator.redraw_callback();
        let next = Executor::new(Chain {
            first: self.filter,
            second: filter,
        });
        let next = Executor {
            output_size: self.output_size,
            resizes_output: self.resizes_output,
            filter: next.filter,
            watcher_guards: next.watcher_guards,
            animator: next.animator,
            gpu: next.gpu,
            setup_error: next.setup_error,
        };
        if let Some(callback) = redraw_callback {
            next.animator.install_redraw_callback(callback);
        }
        next
    }

    /// The parameters' largest magnitudes over their running animations,
    /// after the changes received so far — the values [`Self::footprint`]
    /// evaluates `footprint_of` at.
    #[must_use]
    pub fn param_bounds(&mut self) -> F::Params {
        F::Params::read_from(&self.animator.magnitude_bounds())
    }

    /// The largest distance, in pixels, between an output pixel and any
    /// input texel it reads, for every value the parameters take until
    /// their running animations complete.
    ///
    /// It evaluates [`SpatialFilter::footprint_of`] at every parameter's
    /// largest magnitude over its animation track (see
    /// [`AnimationTrack::magnitude_bound`](crate::AnimationTrack::magnitude_bound)),
    /// after applying the parameter changes received so far, and resolves
    /// the result against the input's `(width, height)` in pixels.
    pub fn footprint(&mut self, size: (f32, f32)) -> f32
    where
        F: SpatialFilter,
    {
        F::footprint_of(&F::Params::read_from(&self.animator.magnitude_bounds())).resolve(size)
    }

    /// The per-frame sampled parameter values, for test observation.
    #[cfg(test)]
    pub(crate) fn animated_values(&self) -> &[f32] {
        self.animator.current_values()
    }

    /// `setup` with every input format treated as unfilterable — tests
    /// exercise the manual-bilinear path on a device that could filter.
    #[cfg(test)]
    #[expect(
        clippy::future_not_send,
        reason = "the executor owns device-bound pipelines and is set up on the GPU host thread"
    )]
    pub(crate) async fn setup_unfilterable(
        &mut self,
        ctx: &EffectContext<'_>,
    ) -> EffectSetupResult {
        self.attach(
            Gpu::with_options(
                &self.filter,
                ctx,
                gpu::PlanOptions {
                    input_filterable: false,
                    intermediate_filterable: false,
                    fold: false,
                },
                self.resizes_output,
            )
            .await,
        )
    }

    /// `setup` picking the composer's folded alternative wherever offered —
    /// tests compare it against the plain program this executor takes.
    #[cfg(test)]
    #[expect(
        clippy::future_not_send,
        reason = "the executor owns device-bound pipelines and is set up on the GPU host thread"
    )]
    pub(crate) async fn setup_folded(&mut self, ctx: &EffectContext<'_>) -> EffectSetupResult {
        let features = ctx.device.features();
        self.attach(
            Gpu::with_options(
                &self.filter,
                ctx,
                gpu::PlanOptions {
                    input_filterable: gpu::filterable(ctx.input_format, features),
                    intermediate_filterable: gpu::filterable(gpu::INTERMEDIATE_FORMAT, features),
                    fold: true,
                },
                self.resizes_output,
            )
            .await,
        )
    }

    /// Runs the built pipelines, or sticks the setup error.
    fn attach(&mut self, result: Result<Gpu, EffectSetupError>) -> EffectSetupResult {
        match result {
            Ok(gpu) => {
                self.gpu = Some(gpu);
                self.setup_error = None;
                self.animator.ensure_redraw_callback();
                self.animator.apply_targets_to_current();
                Ok(())
            }
            Err(error) => {
                tracing::error!("[filtrate] executor setup failed: {error}");
                self.gpu = None;
                self.setup_error = Some(error.clone());
                Err(error)
            }
        }
    }
}

impl<F: Filter, S: Fn(u32, u32) -> (u32, u32) + 'static> Effect for Executor<F, S> {
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        let size = (self.output_size)(input_width, input_height);
        assert!(
            size.0 > 0 && size.1 > 0,
            "effect declared a zero output dimension: {size:?}"
        );
        size
    }

    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        self.animator.install_redraw_callback(callback);
    }

    #[expect(
        clippy::future_not_send,
        reason = "the executor owns device-bound pipelines and is set up on the GPU host thread"
    )]
    async fn setup(&mut self, ctx: &EffectContext<'_>) -> EffectSetupResult {
        self.attach(Gpu::new(&self.filter, ctx, self.resizes_output).await)
    }

    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        if let Some(error) = &self.setup_error {
            return Err(EffectRenderError::SetupFailed(error.clone()));
        }
        let expected = self.output_size(input.width, input.height);
        if expected != (output.width, output.height) {
            return Err(EffectRenderError::SizeMismatch {
                input: (input.width, input.height),
                expected,
                output: (output.width, output.height),
            });
        }
        let gpu = self.gpu.as_mut().ok_or(EffectRenderError::NotSetUp)?;
        let needs_redraw = self.animator.update(input.timing.delta());
        gpu.encode(input, output, encoder, self.animator.current_values())?;
        self.animator.mark_rendered();
        Ok(needs_redraw)
    }

    fn redraw_hint(&self) -> bool {
        self.animator.redraw_hint()
    }
}
