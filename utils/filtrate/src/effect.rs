//! GPU-aware runtime trait for filter effects.
//!
//! `Effect` is the runtime counterpart to `Filter`. While `Filter` describes
//! a filter as pure data (shader fragments, parameters, stage layout),
//! `Effect` describes how a filter pipeline actually runs on a wgpu device:
//! it owns GPU resources between [`Effect::setup`] and the next teardown,
//! reads input textures, and writes output textures.
//!
//! Most callers do not implement `Effect` directly; they implement [`Filter`]
//! and use a `Pipeline`-like adapter (currently `waterui_graphics::FilterAdapter`).
//! `Effect` is the seam used by GPU host code to dispatch a runtime-typed
//! filter without knowing its concrete shape.

extern crate alloc;

use alloc::boxed::Box;
use core::any::TypeId;
use core::fmt;
use core::future::Future;
use core::pin::Pin;

/// Boxed future for filter setup.
pub type EffectSetupFuture<'a> = Pin<Box<dyn Future<Output = EffectSetupResult> + 'a>>;

/// Result returned by filter setup.
pub type EffectSetupResult = Result<(), &'static str>;

/// Result returned by one filter render pass. `Ok(true)` indicates an
/// animation is still active and the host should request another frame.
pub type EffectRenderResult = Result<bool, &'static str>;

/// GPU resources provided to the effect during setup.
pub struct EffectContext<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The texture format of the input (captured view).
    pub input_format: wgpu::TextureFormat,
    /// The texture format of the output.
    pub output_format: wgpu::TextureFormat,
    /// Optional pipeline cache for faster pipeline creation.
    pub pipeline_cache: Option<&'a wgpu::PipelineCache>,
}

impl fmt::Debug for EffectContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectContext")
            .field("input_format", &self.input_format)
            .field("output_format", &self.output_format)
            .finish_non_exhaustive()
    }
}

/// Input texture provided during effect rendering.
pub struct EffectInput<'a> {
    /// The wgpu device.
    pub device: &'a wgpu::Device,
    /// The wgpu queue.
    pub queue: &'a wgpu::Queue,
    /// The captured view's texture.
    pub texture: &'a wgpu::Texture,
    /// A view into the input texture.
    pub view: wgpu::TextureView,
    /// The texture format.
    pub format: wgpu::TextureFormat,
    /// Width of the input texture in pixels.
    pub width: u32,
    /// Height of the input texture in pixels.
    pub height: u32,
}

impl fmt::Debug for EffectInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectInput")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Output texture provided during effect rendering.
pub struct EffectOutput<'a> {
    /// The wgpu device.
    pub device: &'a wgpu::Device,
    /// The wgpu queue.
    pub queue: &'a wgpu::Queue,
    /// The output texture to write to.
    pub texture: &'a wgpu::Texture,
    /// A view into the output texture.
    pub view: wgpu::TextureView,
    /// The texture format.
    pub format: wgpu::TextureFormat,
    /// Width of the output texture in pixels.
    pub width: u32,
    /// Height of the output texture in pixels.
    pub height: u32,
}

impl fmt::Debug for EffectOutput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectOutput")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Trait for GPU effects.
///
/// Implement this trait to create custom GPU filters that process captured
/// view textures. The effect receives input and output textures with their
/// dimensions, allowing for effects that change output size.
///
/// # Async Setup
///
/// The `setup` method returns a future, allowing async initialization.
/// For sync effects, return `async {}` after doing sync work.
/// The future is awaited on the same render thread that created it.
///
/// # Animation Support
///
/// The `render` method returns an [`EffectRenderResult`]. Return `Ok(true)`
/// while animation is in progress, `Ok(false)` for a completed frame, and
/// `Err(...)` for an explicit render failure.
pub trait Effect: 'static {
    /// Called once when GPU resources are ready.
    ///
    /// Use this to create pipelines, bind groups, samplers, and other
    /// GPU resources that persist across frames.
    ///
    /// # Errors
    ///
    /// Returns an explicit setup error when the effect cannot build the
    /// required GPU pipeline for the current device or texture formats.
    fn setup(&mut self, ctx: &EffectContext) -> impl Future<Output = EffectSetupResult>;

    /// Called each frame to apply the effect.
    ///
    /// Read from `input.texture`/`input.view` and write to
    /// `output.texture`/`output.view`. Input and output may have different
    /// dimensions.
    ///
    /// Returns `Ok(true)` if another frame is needed (animation in progress).
    ///
    /// # Errors
    ///
    /// Returns an explicit render error when the compiled effect graph is
    /// incomplete or required GPU resources are missing.
    fn render(&mut self, input: &EffectInput, output: &EffectOutput) -> EffectRenderResult;

    /// Resolve the output dimensions from the current snapped effect state.
    ///
    /// Implementations that depend on reactive inputs must snapshot those
    /// values in [`Effect::sync_targets`] and only read the snapped state
    /// here.
    #[must_use]
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        (input_width, input_height)
    }

    /// Snapshot reactive target values before render dispatch.
    ///
    /// Native backends call this on the UI thread before scheduling render
    /// on a background queue. Effects without reactive sources can keep the
    /// default.
    fn sync_targets(&mut self) {}

    /// Whether the effect has pending state that requires another render
    /// pass. Used by native backends to keep on-demand rendering responsive
    /// when reactive parameters change without layout updates.
    fn redraw_hint(&self) -> bool {
        false
    }
}

/// Object-safe trait for type-erased GPU effects.
///
/// Used by host code that holds heterogeneous effects in a `Box<dyn ...>`
/// (for example, the `Metadata<AppliedEffect>` FFI shim in `WaterUI`).
pub trait ErasedEffect: 'static {
    /// Drive `Effect::setup` through a boxed future.
    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> EffectSetupFuture<'a>;
    /// Drive `Effect::render`.
    fn render(&mut self, input: &EffectInput, output: &EffectOutput) -> EffectRenderResult;
    /// Drive `Effect::output_size`.
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32);
    /// Drive `Effect::sync_targets`.
    fn sync_targets(&mut self);
    /// Drive `Effect::redraw_hint`.
    fn redraw_hint(&self) -> bool;
    /// Returns the [`TypeId`] of the underlying concrete effect type. Useful
    /// for backend-side type matching.
    fn concrete_type_id(&self) -> TypeId;
}

impl<T: Effect> ErasedEffect for T {
    fn setup<'a>(&'a mut self, ctx: &'a EffectContext<'a>) -> EffectSetupFuture<'a> {
        Box::pin(Effect::setup(self, ctx))
    }

    fn render(&mut self, input: &EffectInput, output: &EffectOutput) -> EffectRenderResult {
        Effect::render(self, input, output)
    }

    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        Effect::output_size(self, input_width, input_height)
    }

    fn sync_targets(&mut self) {
        Effect::sync_targets(self);
    }

    fn redraw_hint(&self) -> bool {
        Effect::redraw_hint(self)
    }

    fn concrete_type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }
}
