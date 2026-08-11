//! GPU-aware runtime trait for filter effects.
//!
//! `Effect` is the runtime counterpart to `Filter`. While `Filter` describes
//! a filter as pure data (shader fragments, parameters, stage layout),
//! `Effect` describes how a filter pipeline actually runs on a wgpu device:
//! it owns GPU resources between [`Effect::setup`] and the next teardown,
//! reads input textures, and writes output textures.
//!
//! Most callers do not implement `Effect` directly; they implement [`Filter`](crate::Filter)
//! and use [`FilterAdapter`](crate::FilterAdapter).
//! `Effect` is the seam used by GPU host code to dispatch a runtime-typed
//! filter without knowing its concrete shape.
//!
//! # Color and alpha contract
//!
//! - **Premultiplied alpha, end to end.** Input textures, intermediates,
//!   and outputs carry premultiplied alpha. Linear spatial operations
//!   (blurs, convolutions, resampling) run directly on premultiplied data;
//!   fused color passes unpremultiply once in the shared preamble, apply
//!   every fragment on straight-alpha color, and re-premultiply in the
//!   postamble. Opaque content (alpha = 1) is unaffected either way.
//! - **Encoding-agnostic values.** Filters operate on texel values exactly
//!   as sampled — no implicit sRGB decode/encode is inserted, matching the
//!   behavior of non-linear filter stacks like Core Image's default. Hosts
//!   that want linear-light filtering pass linear(-view) textures in and
//!   out; scratch intermediates preserve whichever convention the input
//!   uses (LDR scratch is non-sRGB `Rgba8Unorm`, so sampled values round-
//!   trip unchanged).

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use core::fmt;
use core::future::Future;
use std::time::{Duration, Instant};

/// Error produced while compiling an effect's GPU pipeline during setup.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EffectSetupError {
    /// The filter chain declares more parameters than the uniform budget.
    #[error("filter chain declares {declared} params, exceeding the {limit}-param uniform budget")]
    TooManyParams {
        /// Parameters declared by the chain.
        declared: usize,
        /// The uniform budget ([`filtrate_core::MAX_FILTER_PARAMS`]).
        limit: usize,
    },
    /// The filter graph produced no stages or passes.
    #[error("filter graph produced no executable passes")]
    EmptyGraph,
    /// Pipeline creation hit a wgpu validation error. The message carries
    /// the full naga/wgpu diagnostic (shader line/column included).
    #[error("{stage} pipeline validation failed: {message}")]
    PipelineValidation {
        /// Which pipeline failed ("color", "spatial", "blit", …).
        stage: &'static str,
        /// The full wgpu validation diagnostic.
        message: String,
    },
    /// The selected scratch texture format is unsupported on this device.
    #[error("scratch texture format {format:?} is unsupported on this device")]
    ScratchFormatUnsupported {
        /// The rejected format.
        format: wgpu::TextureFormat,
    },
    /// HDR intermediates are required by policy but unavailable.
    #[error("HDR intermediates required by policy but unavailable: {0}")]
    HdrRequiredUnavailable(#[source] alloc::boxed::Box<Self>),
    /// An internal planner invariant was violated.
    #[error("filter planner invariant violated: {0}")]
    PlannerInvariant(&'static str),
    /// Effect-specific setup failure outside the planner/pipeline paths.
    #[error("{0}")]
    Other(&'static str),
}

/// Error produced while encoding one effect frame.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EffectRenderError {
    /// Setup failed earlier; the error is sticky and rendering fails fast.
    #[error("effect setup failed: {0}")]
    SetupFailed(#[from] EffectSetupError),
    /// The input or output texture format differs from the formats the
    /// pipeline was compiled against during setup.
    #[error(
        "render texture formats (input {input:?}, output {output:?}) do not match setup formats \
         (input {setup_input:?}, output {setup_output:?})"
    )]
    FormatMismatch {
        /// Input format seen at render time.
        input: wgpu::TextureFormat,
        /// Output format seen at render time.
        output: wgpu::TextureFormat,
        /// Input format the pipeline was compiled for.
        setup_input: wgpu::TextureFormat,
        /// Output format the pipeline was compiled for.
        setup_output: wgpu::TextureFormat,
    },
    /// A GPU resource that setup should have produced is missing.
    #[error("{0}")]
    MissingResource(&'static str),
}

/// Result returned by filter setup.
pub type EffectSetupResult = Result<(), EffectSetupError>;

/// Result returned by one filter render pass. `Ok(true)` indicates an
/// animation is still active and the host should request another frame.
pub type EffectRenderResult = Result<bool, EffectRenderError>;

/// Thread-safe callback used by an effect to wake an on-demand renderer.
pub type EffectRedrawCallback = Arc<dyn Fn() + Send + Sync>;

/// Deterministic timeline information for one effect input frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectFrameTiming {
    presentation_time: Duration,
    delta: Duration,
    sequence: u64,
    discontinuity: bool,
}

impl EffectFrameTiming {
    /// Creates timing for one input frame.
    #[must_use]
    pub const fn new(presentation_time: Duration, delta: Duration, sequence: u64) -> Self {
        Self {
            presentation_time,
            delta,
            sequence,
            discontinuity: false,
        }
    }

    /// Marks whether this frame begins a discontinuous timeline segment.
    #[must_use]
    pub const fn with_discontinuity(mut self, discontinuity: bool) -> Self {
        self.discontinuity = discontinuity;
        self
    }

    /// Returns the timestamp on the host-selected timeline.
    #[must_use]
    pub const fn presentation_time(self) -> Duration {
        self.presentation_time
    }

    /// Returns elapsed timeline time since the preceding frame.
    #[must_use]
    pub const fn delta(self) -> Duration {
        self.delta
    }

    /// Returns the monotonically increasing frame sequence number.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Returns whether this frame begins a discontinuous segment.
    #[must_use]
    pub const fn is_discontinuity(self) -> bool {
        self.discontinuity
    }
}

/// Per-host wall-clock adapter for ordinary interactive view effects.
///
/// Media pipelines should supply their own presentation timestamps through
/// [`EffectFrameTiming`] instead. This clock exists so UI hosts own wall-clock
/// policy explicitly rather than effects reading `Instant::now()` internally.
#[derive(Debug)]
pub struct EffectFrameClock {
    origin: Instant,
    previous: Instant,
    sequence: u64,
}

impl EffectFrameClock {
    /// Starts a host-owned frame clock.
    #[must_use]
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            origin: now,
            previous: now,
            sequence: 0,
        }
    }

    /// Samples the next frame timing.
    #[must_use]
    pub fn tick(&mut self) -> EffectFrameTiming {
        let now = Instant::now();
        let timing = EffectFrameTiming::new(
            now.saturating_duration_since(self.origin),
            now.saturating_duration_since(self.previous),
            self.sequence,
        );
        self.previous = now;
        self.sequence = self.sequence.saturating_add(1);
        timing
    }
}

impl Default for EffectFrameClock {
    fn default() -> Self {
        Self::new()
    }
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
    /// Deterministic host-selected timing for this frame.
    pub timing: EffectFrameTiming,
}

impl fmt::Debug for EffectInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectInput")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("timing", &self.timing)
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
