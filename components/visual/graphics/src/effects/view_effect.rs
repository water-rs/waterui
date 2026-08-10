//! GPU effect rendering for captured view content.
//!
//! This module provides `ViewEffect`, a view that captures its child's rendered output
//! and applies custom GPU effects to it. The effect receives the captured texture and
//! can output to a texture of a different size without affecting layout.

extern crate alloc;

use alloc::{boxed::Box, sync::Arc};
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use num_traits::ToPrimitive;

use waterui_core::View;

use crate::gpu_surface::RedrawHandle;
type ErasedSetupFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

/// Thread-safe callback used by a view effect to wake an on-demand renderer.
pub type ViewEffectRedrawCallback = Arc<dyn Fn() + Send + Sync>;

/// GPU resources provided to the effect renderer during setup.
///
/// Contains references to the wgpu device, queue, and texture formats.
/// No dimensions are provided at setup time - effects handle dimension
/// changes lazily in `render()`.
pub struct ViewEffectContext<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The texture format of the captured input.
    pub input_format: wgpu::TextureFormat,
    /// The texture format of the effect output.
    pub output_format: wgpu::TextureFormat,
}

impl fmt::Debug for ViewEffectContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewEffectContext")
            .field("input_format", &self.input_format)
            .field("output_format", &self.output_format)
            .finish_non_exhaustive()
    }
}

impl ViewEffectContext<'_> {
    /// Returns `true` if the input format is HDR-capable (floating-point).
    #[must_use]
    pub const fn is_input_hdr(&self) -> bool {
        matches!(
            self.input_format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        )
    }

    /// Returns `true` if the output format is HDR-capable (floating-point).
    #[must_use]
    pub const fn is_output_hdr(&self) -> bool {
        matches!(
            self.output_format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        )
    }
}

/// Input data provided during each render call.
///
/// Contains the captured view's texture and dimensions.
pub struct ViewEffectInput<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The captured view's texture (read from this).
    pub texture: &'a wgpu::Texture,
    /// A view into the captured texture.
    pub view: wgpu::TextureView,
    /// The texture format of the captured input.
    pub format: wgpu::TextureFormat,
    /// Width of the captured view in pixels.
    pub width: u32,
    /// Height of the captured view in pixels.
    pub height: u32,
}

impl fmt::Debug for ViewEffectInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewEffectInput")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl ViewEffectInput<'_> {
    /// Returns `true` if the input format is HDR-capable (floating-point).
    #[must_use]
    pub const fn is_hdr(&self) -> bool {
        matches!(
            self.format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        )
    }
}

/// Output data provided during each render call.
///
/// Contains the texture to render the effect result into.
pub struct ViewEffectOutput<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The output texture (write to this).
    pub texture: &'a wgpu::Texture,
    /// A view into the output texture.
    pub view: wgpu::TextureView,
    /// The texture format of the output.
    pub format: wgpu::TextureFormat,
    /// Width of the output texture in pixels.
    pub width: u32,
    /// Height of the output texture in pixels.
    pub height: u32,
}

impl fmt::Debug for ViewEffectOutput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewEffectOutput")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl ViewEffectOutput<'_> {
    /// Returns `true` if the output format is HDR-capable (floating-point).
    #[must_use]
    pub const fn is_hdr(&self) -> bool {
        matches!(
            self.format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        )
    }
}

/// Trait for GPU effect renderers.
///
/// Implement this trait to create custom GPU effects that process captured view content.
/// The renderer receives the captured view's texture and can output to a different-sized
/// texture without affecting layout.
///
/// # Async Setup
///
/// The `setup` method returns a future, allowing async initialization.
/// For sync renderers, return `async {}` after doing sync work.
///
/// **Note:** The future does not require `Send` - it's created and awaited on the same thread.
///
/// # Example
///
/// ```ignore
/// struct BlurEffect {
///     pipeline: Option<wgpu::RenderPipeline>,
///     bind_group_layout: Option<wgpu::BindGroupLayout>,
///     sampler: Option<wgpu::Sampler>,
///     blur_radius: f32,
/// }
///
/// impl EffectRenderer for BlurEffect {
///     fn setup(&mut self, ctx: &ViewEffectContext) -> impl Future<Output = ()> {
///         // Create pipeline, bind group layout, sampler
///         self.bind_group_layout = Some(ctx.device.create_bind_group_layout(&...));
///         self.sampler = Some(ctx.device.create_sampler(&...));
///         self.pipeline = Some(ctx.device.create_render_pipeline(&...));
///         async {}
///     }
///
///     fn render(&mut self, input: &ViewEffectInput, output: &ViewEffectOutput) {
///         // Sample from input.view, render blurred result to output.view
///         let mut encoder = input.device.create_command_encoder(&Default::default());
///         // ... apply blur effect ...
///         input.queue.submit([encoder.finish()]);
///     }
/// }
/// ```
pub trait EffectRenderer: 'static {
    /// Installs the host callback used when external renderer state becomes dirty.
    ///
    /// Renderers without externally driven state can keep the default no-op
    /// implementation. Stateful renderers install the callback before [`Self::setup`].
    fn set_redraw_callback(&mut self, _callback: ViewEffectRedrawCallback) {}

    /// Called once when GPU resources are ready.
    ///
    /// Use this to create pipelines, bind groups, samplers, and other
    /// GPU resources that persist across frames. No dimensions are provided;
    /// handle dimension-dependent resources in `render()`.
    ///
    /// Returns a future that completes when setup is done.
    fn setup(&mut self, ctx: &ViewEffectContext) -> impl Future<Output = ()>;

    /// Called each frame to apply the effect.
    ///
    /// Read from `input.texture`/`input.view` and write to `output.texture`/`output.view`.
    /// Input and output dimensions are available via the respective structs.
    fn render(&mut self, input: &ViewEffectInput, output: &ViewEffectOutput);

    /// Returns whether the effect needs another frame immediately.
    ///
    /// Effects with internal animations should return `true` while animation is active.
    /// Static effects can use the default `false`.
    #[must_use]
    fn needs_redraw(&self) -> bool {
        false
    }
}

/// Object-safe trait for type-erased effect renderers.
pub(crate) trait EffectRendererImpl: 'static {
    fn set_redraw_callback(&mut self, callback: ViewEffectRedrawCallback);
    fn setup<'a>(&'a mut self, ctx: &'a ViewEffectContext<'a>) -> ErasedSetupFuture<'a>;
    fn render(&mut self, input: &ViewEffectInput, output: &ViewEffectOutput);
    fn needs_redraw(&self) -> bool;
}

impl<T: EffectRenderer> EffectRendererImpl for T {
    fn set_redraw_callback(&mut self, callback: ViewEffectRedrawCallback) {
        EffectRenderer::set_redraw_callback(self, callback);
    }

    fn setup<'a>(&'a mut self, ctx: &'a ViewEffectContext<'a>) -> ErasedSetupFuture<'a> {
        Box::pin(EffectRenderer::setup(self, ctx))
    }

    fn render(&mut self, input: &ViewEffectInput, output: &ViewEffectOutput) {
        EffectRenderer::render(self, input, output);
    }

    fn needs_redraw(&self) -> bool {
        EffectRenderer::needs_redraw(self)
    }
}

/// Output size specification for `ViewEffect`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OutputSize {
    /// Match the input view's size (default).
    #[default]
    MatchInput,
    /// Fixed pixel dimensions.
    Fixed {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// Scale factor relative to input dimensions.
    Scale(f32),
}

impl OutputSize {
    /// Compute the output dimensions from input dimensions.
    #[must_use]
    pub fn compute(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        match *self {
            Self::MatchInput => (input_width, input_height),
            Self::Fixed { width, height } => (width, height),
            Self::Scale(factor) => (
                scaled_dimension(input_width, factor),
                scaled_dimension(input_height, factor),
            ),
        }
    }
}

/// A view that captures its child's rendered output and applies a GPU effect.
///
/// `ViewEffect` enables post-processing effects on any view. The child view is rendered
/// to an intermediate texture, which is then passed to the effect renderer. The effect
/// can output to a texture of a different size without affecting layout.
///
/// # Layout Behavior
///
/// Layout is based entirely on the child view's intrinsic size. The output texture
/// size (controlled by `output_size`) does not affect layout calculations. This allows
/// effects like blur (which may need padding) to work without affecting the view hierarchy.
///
/// # `GpuSurface` Optimization
///
/// When the child view is a `GpuSurface`, the effect can directly sample its texture
/// without an intermediate capture step. This optimization is handled automatically
/// by the native backends.
///
/// # Example
///
/// ```ignore
/// // Apply a blur effect to any view
/// ViewEffect::new(
///     text("Hello, World!"),
///     BlurEffect::new(10.0),
/// )
///
/// // Scale output for higher quality effects
/// ViewEffect::new(
///     my_complex_view(),
///     SharpenEffect::new(),
/// )
/// .output_size(OutputSize::Scale(2.0))
/// ```
pub struct ViewEffect<V: View, E: EffectRenderer> {
    /// The view to capture and apply effect to.
    pub(crate) content: V,
    /// The effect renderer (type-erased internally for FFI).
    pub(crate) effect: E,
    /// Output texture size configuration.
    pub(crate) output_size: OutputSize,
}

impl<V: View, E: EffectRenderer> fmt::Debug for ViewEffect<V, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewEffect")
            .field("output_size", &self.output_size)
            .finish_non_exhaustive()
    }
}

impl<V: View, E: EffectRenderer> ViewEffect<V, E> {
    /// Creates a new view effect with the provided content and effect renderer.
    ///
    /// # Arguments
    ///
    /// * `content` - The view to capture and apply the effect to.
    /// * `effect` - An implementation of `EffectRenderer` that handles the GPU effect.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let effected = ViewEffect::new(my_view(), BlurEffect::new(5.0));
    /// ```
    #[must_use]
    pub fn new(content: V, effect: E) -> Self {
        Self {
            content,
            effect,
            output_size: OutputSize::default(),
        }
    }

    /// Sets the output texture size.
    ///
    /// This controls the size of the texture that the effect renders to,
    /// without affecting layout (which is based on the child view's size).
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Double resolution for sharper effects
    /// ViewEffect::new(view, effect)
    ///     .output_size(OutputSize::Scale(2.0))
    ///
    /// // Fixed size output
    /// ViewEffect::new(view, effect)
    ///     .output_size(OutputSize::Fixed { width: 1920, height: 1080 })
    /// ```
    #[must_use]
    pub const fn output_size(mut self, size: OutputSize) -> Self {
        self.output_size = size;
        self
    }
}

/// Type-erased `ViewEffect` for FFI boundary.
///
/// This wraps the generic `ViewEffect<V, E>` with type-erased content and effect
/// for use across the FFI boundary.
pub struct ViewEffectErased {
    /// The child view (type-erased via `AnyView`).
    pub(crate) content: waterui_core::AnyView,
    /// The effect renderer (type-erased).
    pub(crate) effect: Box<dyn EffectRendererImpl>,
    /// Output size configuration.
    pub(crate) output_size: OutputSize,
    /// Handle shared with renderer-driven invalidation callbacks.
    redraw_handle: RedrawHandle,
}

impl fmt::Debug for ViewEffectErased {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewEffectErased")
            .field("output_size", &self.output_size)
            .finish_non_exhaustive()
    }
}

impl ViewEffectErased {
    fn new<E: EffectRenderer>(
        content: waterui_core::AnyView,
        effect: E,
        output_size: OutputSize,
    ) -> Self {
        let redraw_handle = RedrawHandle::new();
        let renderer_redraw_handle = redraw_handle.clone();
        let mut effect: Box<dyn EffectRendererImpl> = Box::new(effect);
        effect.set_redraw_callback(Arc::new(move || {
            renderer_redraw_handle.request_redraw();
        }));
        Self {
            content,
            effect,
            output_size,
            redraw_handle,
        }
    }

    /// Calls `setup` on the effect, returning a future that completes when ready.
    #[expect(
        clippy::future_not_send,
        reason = "view-effect setup is driven by the UI-local GPU host executor and intentionally accepts non-Send renderers"
    )]
    pub fn setup<'a>(
        &'a mut self,
        ctx: &'a ViewEffectContext<'a>,
    ) -> impl Future<Output = ()> + 'a {
        self.effect.setup(ctx)
    }

    /// Calls `render` on the effect.
    pub fn render(&mut self, input: &ViewEffectInput, output: &ViewEffectOutput) {
        let _ = self.redraw_handle.take_dirty();
        self.effect.render(input, output);
    }

    /// Returns whether the effect requests another immediate frame.
    #[must_use]
    pub fn needs_redraw(&self) -> bool {
        let callback_requested_redraw = self.redraw_handle.take_dirty();
        self.effect.needs_redraw() || callback_requested_redraw
    }

    /// Returns a clone of the handle used to wake the native renderer.
    #[must_use]
    pub fn redraw_handle(&self) -> RedrawHandle {
        self.redraw_handle.clone()
    }

    /// Returns the output size configuration.
    #[must_use]
    pub const fn output_size(&self) -> OutputSize {
        self.output_size
    }

    /// Returns a reference to the child view.
    #[must_use = "this borrows the child view without consuming the effect"]
    pub const fn content(&self) -> &waterui_core::AnyView {
        &self.content
    }

    /// Takes ownership of the child view.
    pub fn take_content(&mut self) -> waterui_core::AnyView {
        core::mem::take(&mut self.content)
    }

    /// Replaces the child view content while preserving the effect renderer.
    pub fn replace_content(&mut self, content: waterui_core::AnyView) {
        self.content = content;
    }

    /// Updates the output size configuration while preserving the effect renderer.
    pub const fn set_output_size(&mut self, output_size: OutputSize) {
        self.output_size = output_size;
    }
}

// ViewEffect is a raw view - the native backend handles rendering
waterui_core::raw_view!(ViewEffectErased, waterui_core::layout::StretchAxis::None);

impl<V: View, E: EffectRenderer> View for ViewEffect<V, E> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        ViewEffectErased::new(
            waterui_core::AnyView::new(self.content),
            self.effect,
            self.output_size,
        )
    }
}

fn scaled_dimension(value: u32, factor: f32) -> u32 {
    (value
        .to_f32()
        .expect("view_effect: dimension must be representable as f32")
        * factor)
        .round()
        .clamp(0.0, u32::MAX.to_f32().expect("u32::MAX must fit in f32"))
        .to_u32()
        .expect("view_effect: scaled dimension must convert to u32")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use nami::{Binding, Signal, binding};
    use waterui_core::AnyView;

    use super::*;

    struct SignalRenderer {
        callback: Arc<OnceLock<ViewEffectRedrawCallback>>,
        _watcher_guard: Box<dyn core::any::Any>,
    }

    impl SignalRenderer {
        fn new(value: &Binding<f32>) -> Self {
            let callback = Arc::<OnceLock<ViewEffectRedrawCallback>>::default();
            let watcher_callback = callback.clone();
            let guard = value.watch(move |_| {
                let callback = watcher_callback
                    .get()
                    .expect("ViewEffect redraw callback must be installed before signal changes");
                callback();
            });
            Self {
                callback,
                _watcher_guard: Box::new(guard),
            }
        }
    }

    impl EffectRenderer for SignalRenderer {
        fn set_redraw_callback(&mut self, callback: ViewEffectRedrawCallback) {
            assert!(
                self.callback.set(callback).is_ok(),
                "SignalRenderer redraw callback was installed more than once"
            );
        }

        fn setup(&mut self, _ctx: &ViewEffectContext) -> impl Future<Output = ()> {
            core::future::ready(())
        }

        fn render(&mut self, _input: &ViewEffectInput, _output: &ViewEffectOutput) {}
    }

    #[test]
    fn signal_change_pushes_redraw_to_erased_view_effect_handle() {
        let value = binding(0.0_f32);
        let erased = ViewEffectErased::new(
            AnyView::new(()),
            SignalRenderer::new(&value),
            OutputSize::MatchInput,
        );
        let redraw_handle = erased.redraw_handle();

        assert!(!redraw_handle.is_dirty());
        value.set(1.0);
        assert!(redraw_handle.is_dirty());
    }
}
