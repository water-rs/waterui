//! GPU-aware runtime trait for filter effects.
//!
//! `Effect` is the runtime counterpart to `Filter`. While `Filter` describes
//! a filter as pure data (shader fragments, parameters, stage layout),
//! `Effect` describes how a filter pipeline actually runs on a wgpu device:
//! it owns GPU resources between [`Effect::setup`] and the next teardown,
//! reads input textures, and writes output textures.
//!
//! Most callers do not implement `Effect` directly; they implement [`Filter`](crate::Filter)
//! and use a `Pipeline`-like adapter (currently `waterui_graphics::FilterAdapter`).
//! `Effect` is the seam used by GPU host code to dispatch a runtime-typed
//! filter without knowing its concrete shape.

extern crate alloc;

use alloc::{boxed::Box, sync::Arc};
use core::fmt;
use core::future::Future;
use std::collections::HashMap;
use std::sync::OnceLock;

use parking_lot::Mutex;

/// Result returned by filter setup.
pub type EffectSetupResult = Result<(), &'static str>;

/// Result returned by one filter render pass. `Ok(true)` indicates an
/// animation is still active and the host should request another frame.
pub type EffectRenderResult = Result<bool, &'static str>;

/// Thread-safe callback used by an effect to wake an on-demand renderer.
pub type EffectRedrawCallback = Arc<dyn Fn() + Send + Sync>;

/// Device-bound WGSL module cache shared by GPU hosts and effects.
///
/// A cache must only be used with the device passed to [`Self::get_or_create`].
/// GPU hosts own and inject it explicitly through their setup context, keeping
/// cache lifetime aligned with the device instead of relying on process-global
/// renderer state.
#[derive(Clone, Default)]
pub struct ShaderCache {
    entries: Arc<Mutex<HashMap<u64, Vec<CachedShader>>>>,
}

impl fmt::Debug for ShaderCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShaderCache")
            .finish_non_exhaustive()
    }
}

struct CachedShader {
    source: Box<str>,
    module: Arc<OnceLock<Arc<wgpu::ShaderModule>>>,
}

impl ShaderCache {
    /// Creates an empty shader cache for one GPU device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a cached module for `source`, compiling it exactly once when absent.
    #[must_use]
    pub fn get_or_create(
        &self,
        device: &wgpu::Device,
        label: &str,
        source: &str,
    ) -> Arc<wgpu::ShaderModule> {
        self.get_or_create_prehashed(device, label, source, shader_source_hash(source))
    }

    /// Returns a cached module using a caller-provided stable source hash.
    ///
    /// Hash collisions are resolved by comparing the complete WGSL source.
    #[must_use]
    pub fn get_or_create_prehashed(
        &self,
        device: &wgpu::Device,
        label: &str,
        source: &str,
        source_hash: u64,
    ) -> Arc<wgpu::ShaderModule> {
        let module = {
            let mut entries = self.entries.lock();
            let bucket = entries.entry(source_hash).or_default();
            let existing = bucket
                .iter()
                .find(|entry| entry.source.as_ref() == source)
                .map(|entry| Arc::clone(&entry.module));
            let module = existing.unwrap_or_else(|| {
                let module = Arc::new(OnceLock::new());
                bucket.push(CachedShader {
                    source: source.into(),
                    module: Arc::clone(&module),
                });
                module
            });
            drop(entries);
            module
        };

        Arc::clone(module.get_or_init(|| {
            Arc::new(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: (!label.is_empty()).then_some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            }))
        }))
    }
}

/// Computes the stable FNV-1a hash used by [`ShaderCache`].
#[must_use]
pub const fn shader_source_hash(source: &str) -> u64 {
    let bytes = source.as_bytes();
    let mut hash = 0xcbf2_9ce4_8422_2325;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

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
    /// Device-bound shader module cache owned by the GPU host.
    pub shader_cache: &'a ShaderCache,
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
    /// Installs the host callback used when external effect state becomes dirty.
    ///
    /// Effects without externally driven state can keep the default no-op
    /// implementation. Stateful effects install the callback before [`Self::setup`].
    fn set_redraw_callback(&mut self, _callback: EffectRedrawCallback) {}

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

    /// Encodes one frame of effect work into the provided command encoder.
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
    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult;

    /// Called each frame to apply the effect and submit its encoded GPU work.
    ///
    /// Hosts that render many effects in one frame should call
    /// [`Effect::encode_render`] repeatedly with a shared encoder and submit
    /// once after all effects have been encoded.
    ///
    /// # Errors
    ///
    /// Returns an explicit render error when the compiled effect graph is
    /// incomplete or required GPU resources are missing.
    fn render(&mut self, input: &EffectInput, output: &EffectOutput) -> EffectRenderResult {
        let mut encoder = input
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filter effect encoder"),
            });
        let result = self.encode_render(input, output, &mut encoder);
        input.queue.submit([encoder.finish()]);
        result
    }

    /// Resolves the output dimensions from the current effect state.
    #[must_use]
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        (input_width, input_height)
    }

    /// Whether the effect has pending state that requires another render
    /// pass. Used by native backends to keep on-demand rendering responsive
    /// when reactive parameters change without layout updates.
    fn redraw_hint(&self) -> bool {
        false
    }
}
