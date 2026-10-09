//! Capability traits on the backend type.
//!
//! A capability is not a marker: it declares the render-side hook the
//! render loop calls, so a backend without the capability has no code path
//! to reach, and no default or stub exists. `Engine`, `Surface` and
//! `Transaction` methods bounded by these traits wrap the hook into an
//! owned `FnOnce(&mut B::Renderer) + Send` op that travels in the commit
//! with the layer ops, in order.

use std::borrow::Cow;

use crate::ShaderId;
use crate::backend::Backend;
use crate::error::ResourceError;
use crate::image::Format;
use cherenkov_record::{
    BackdropId, BackdropSampling, BackdropShaderId, GpuInstalls, LayerId, SurfaceId,
};

use crate::message::ProducerId;
use crate::style::FilterId;

/// The backend draws user WGSL shader paints.
pub trait ShaderPaint: Backend {
    /// Validates the source on the caller thread, before anything is
    /// queued: everything the backend can check without its device.
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the source fails validation.
    fn validate_shader(source: &ShaderSource) -> Result<(), ResourceError>;
    /// Registers a shader [`ShaderPaint::validate_shader`] accepted, on the
    /// render thread. A rejection fails every later render that draws the
    /// shader with [`RenderError::Rejected`](crate::RenderError::Rejected).
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when pipeline creation fails.
    #[cfg(not(target_arch = "wasm32"))]
    fn add_shader(
        r: &mut Self::Renderer,
        id: ShaderId,
        source: ShaderSource,
    ) -> Result<(), ResourceError>;
    /// Registers a shader [`ShaderPaint::validate_shader`] accepted, on the
    /// owning JS thread, awaiting the browser's pipeline creation without
    /// blocking its event loop.
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when pipeline creation fails.
    #[cfg(target_arch = "wasm32")]
    fn add_shader(
        r: &mut Self::Renderer,
        id: ShaderId,
        source: ShaderSource,
    ) -> impl core::future::Future<Output = Result<(), ResourceError>>;
    /// Unregisters a shader no installed content draws any more.
    fn remove_shader(r: &mut Self::Renderer, id: ShaderId);
}

/// The backend runs filtrate filters.
pub trait Filters: Backend {
    /// Unregisters a filter or effect.
    fn remove_filter(r: &mut Self::Renderer, id: FilterId);
}

/// The backend can run the filtrate filter `F`.
pub trait Runs<F: filtrate_core::Filter + crate::RenderTransfer>: Filters {
    /// Registers a filter on the render thread.
    fn add_filter(r: &mut Self::Renderer, id: FilterId, filter: F);
}

/// The backend runs custom effects (`Box<dyn filtrate::Effect + Send>` on
/// GPU backends).
pub trait Effects: Filters {
    /// The effect payload type.
    type Effect: crate::RenderTransfer + 'static;
    /// Registers an effect on the render thread.
    fn add_effect(r: &mut Self::Renderer, id: FilterId, effect: Self::Effect);
}

/// The backend composites user GPU content as layer content.
///
/// A producer lives at renderer scope, not on a surface: one
/// [`GpuProducer`](crate::GpuProducer) can be bound to layers of any number of
/// the engine's surfaces, and every binding samples the producer's one
/// current frame. Rendered and submitted pixels are that one frame path:
/// a rendered producer draws into a buffer from the renderer-owned frame
/// ring and a [`FrameSink`](crate::FrameSink)'s producer takes the frame
/// its owner submits.
///
/// `GpuInstalls` — the marker that lets the backend's target seal an
/// install payload — comes with `GpuContent`: a backend that runs GPU
/// producers is always a target that can install them.
pub trait GpuContent: Backend + GpuInstalls {
    /// The rendered producer's content payload type.
    type Content: crate::RenderTransfer + 'static;
    /// The frame payload a [`FrameSink`](crate::FrameSink) submits
    /// (`ExternalFrame` on the wgpu backend).
    type Frame: crate::RenderTransfer + 'static;
    /// Whether `frame`'s declared alpha contract is fully opaque — only
    /// then does a planes-capable backend know the layer's coverage
    /// without compositing it (#90).
    fn frame_opaque(frame: &Self::Frame) -> bool;
    /// Registers the producer `producer`'s content on the render thread,
    /// before any binding of it is drawn. Its frame ring is allocated by
    /// each surface's compositor contract, not set up here.
    fn add_gpu_producer(r: &mut Self::Renderer, producer: ProducerId, content: Self::Content);
    /// Registers `producer` as a submitted-frame producer — the kind whose
    /// frames come from [`FrameSink::submit`](crate::FrameSink::submit).
    /// It has no wake state: each submitted frame wakes the surfaces
    /// [`submit_frame`](Self::submit_frame) reports it bound on.
    fn add_frame_producer(r: &mut Self::Renderer, producer: ProducerId);
    /// Binds `producer` to `layer` of `surface` at `size` pixels — the
    /// binding samples `ImageSource::Content(producer)`, the producer's
    /// current frame, and becomes a `Source::Frame` candidate for the
    /// surface's `planes::plan`. A size change is a new binding.
    ///
    /// Returns the current frame's declared alpha for the layer's alpha
    /// contract: `Some(opaque)` once a frame has landed — a rendered
    /// producer's premultiplied ring frame reports `false` — and `None`
    /// before the first, so the layer is noted not known opaque.
    fn bind_gpu_producer(
        r: &mut Self::Renderer,
        surface: SurfaceId,
        layer: LayerId,
        producer: &crate::GpuProducer<Self>,
        size: (u32, u32),
    ) -> Option<bool>;
    /// Installs `frame` as `producer`'s current frame and returns the
    /// `(surface, layer)` pairs it is bound on, so the frame's declared
    /// alpha contract is noted on each of them. A frame producer has no
    /// setup: a frame submitted after a device replacement supplies the
    /// first frame on the new device.
    fn submit_frame(
        r: &mut Self::Renderer,
        producer: ProducerId,
        frame: Self::Frame,
    ) -> Vec<(SurfaceId, LayerId)>;
    /// Retires `producer`, the last `GpuProducer` handle having dropped:
    /// every binding releases it and its device resources — current frame
    /// and frame ring — are freed.
    fn retire_gpu_producer(r: &mut Self::Renderer, producer: ProducerId);
    /// The device-replacement contract: drains every live producer,
    /// dropping the device resources the current device made. A rendered
    /// producer returns its content for the new renderer to re-register;
    /// a frame producer's frame is device state and drops with the old
    /// device — the sink's next submit supplies a frame on the new device.
    /// Also releases every binding.
    fn drain_gpu_producers(r: &mut Self::Renderer) -> Vec<(ProducerId, DrainedProducer<Self>)>;
}

/// What a device replacement hands back for one drained producer
/// ([`GpuContent::drain_gpu_producers`]).
pub enum DrainedProducer<B: GpuContent> {
    /// A rendered producer's content, to re-register with
    /// [`Engine::gpu_producer`](crate::Engine::gpu_producer) on the new
    /// renderer.
    Rendered(B::Content),
    /// A submitted-frame producer: its frame dropped with the old device.
    /// Recreate the pair with [`Engine::frame_producer`](crate::Engine::frame_producer)
    /// and submit again.
    Frame,
}

impl<B: GpuContent> std::fmt::Debug for DrainedProducer<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // `B::Content` is not required to be `Debug`.
            Self::Rendered(_) => f.write_str("Rendered(..)"),
            Self::Frame => f.write_str("Frame"),
        }
    }
}

/// Which image storage formats `add_image` accepts: the backend uploads
/// [`ImageData<F>`](crate::ImageData).
pub trait Uploads<F: Format>: Backend {}

/// A filtrate chain a backdrop group can run, with its footprint bound.
///
/// Spatial chains report their own footprint; colour chains read no
/// neighbour texels and report [`Footprint::ZERO`](filtrate_core::Footprint::ZERO).
pub trait BackdropChain<K: filtrate_core::kind::Kind>: filtrate_core::Filter<Kind = K> {
    /// The chain's footprint for `params` (see
    /// [`SpatialFilter::footprint_of`](filtrate_core::SpatialFilter::footprint_of)).
    fn footprint_bound(params: &Self::Params) -> filtrate_core::Footprint;
}

impl<F: filtrate_core::SpatialFilter> BackdropChain<filtrate_core::kind::Spatial> for F {
    fn footprint_bound(params: &Self::Params) -> filtrate_core::Footprint {
        F::footprint_of(params)
    }
}

impl<F: filtrate_core::Filter<Kind = filtrate_core::kind::Color>>
    BackdropChain<filtrate_core::kind::Color> for F
{
    fn footprint_bound(_params: &Self::Params) -> filtrate_core::Footprint {
        filtrate_core::Footprint::ZERO
    }
}

/// The backend captures and samples backdrops
/// (`Surface::backdrop_group_unfiltered`, `LayerEdit::backdrop`).
pub trait Backdrop: Filters + BackdropSampling {
    /// Registers backdrop group `id` on `surface` with no filter chain,
    /// capturing per `spec` ([`BackdropSpec`](crate::BackdropSpec)).
    fn add_backdrop_group(
        r: &mut Self::Renderer,
        surface: SurfaceId,
        id: BackdropId,
        spec: crate::BackdropSpec,
    );

    /// Unregisters a backdrop group; frames that still sample it fail.
    fn remove_backdrop_group(r: &mut Self::Renderer, surface: SurfaceId, id: BackdropId);
}

/// The backend can run the backdrop chain `F` of kind `K`
/// (`Surface::backdrop_group`).
pub trait BackdropRuns<K: filtrate_core::kind::Kind, F: BackdropChain<K> + crate::RenderTransfer>:
    Backdrop
{
    /// Registers backdrop group `id` on `surface` capturing per `spec`
    /// ([`BackdropSpec`](crate::BackdropSpec)), whose capture runs through
    /// `filter` with its footprint counted in capture texels.
    fn add_filtered_backdrop_group(
        r: &mut Self::Renderer,
        surface: SurfaceId,
        id: BackdropId,
        filter: F,
        spec: crate::BackdropSpec,
    );
}

/// The backend compiles per-member backdrop effect shaders
/// (`Engine::backdrop_shader`).
pub trait BackdropShaders: Backdrop {
    /// Validates the source on the caller thread, before anything is
    /// queued: everything the backend can check without its device.
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the source fails validation.
    fn validate_backdrop_shader(source: &crate::BackdropShaderSource) -> Result<(), ResourceError>;

    /// Registers a backdrop effect shader
    /// [`BackdropShaders::validate_backdrop_shader`] accepted, on the render
    /// thread, compiled for the composite contract; the pipeline is built
    /// here, never at draw time. A rejection fails every later render that
    /// samples the shader with
    /// [`RenderError::Rejected`](crate::RenderError::Rejected).
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when pipeline creation fails.
    #[cfg(not(target_arch = "wasm32"))]
    fn add_backdrop_shader(
        r: &mut Self::Renderer,
        id: BackdropShaderId,
        source: crate::BackdropShaderSource,
    ) -> Result<(), ResourceError>;

    /// Registers a backdrop effect shader
    /// [`BackdropShaders::validate_backdrop_shader`] accepted, on the owning
    /// JS thread, awaiting the browser's pipeline creation without blocking
    /// its event loop.
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when pipeline creation fails.
    #[cfg(target_arch = "wasm32")]
    fn add_backdrop_shader(
        r: &mut Self::Renderer,
        id: BackdropShaderId,
        source: crate::BackdropShaderSource,
    ) -> impl core::future::Future<Output = Result<(), ResourceError>>;

    /// Unregisters a backdrop effect shader no layer samples any more.
    fn remove_backdrop_shader(r: &mut Self::Renderer, id: BackdropShaderId);
}

/// The backend produces HDR output.
pub trait HdrOutput: Backend {}

/// The backend presents on multiple hardware planes.
pub trait Planes: Backend {}

/// The backend shows system layers the host supplies — a system web view,
/// an embedded platform view — in the layer tree, through
/// [`Hosted::at`](crate::Hosted::at).
///
/// A hosted layer is always realized on a system-compositor plane of its
/// own and never composited by the engine. [`Object`](Self::Object) is the
/// platform object itself, not pixels: no engine path samples, filters,
/// caches or reads it back, because none can reach it. Its eligibility is
/// the mandatory-plane rule — a hosted layer that cannot be placed on a
/// plane fails the render with
/// [`RenderError::Unplaceable`](crate::RenderError::Unplaceable), never
/// falls back to composition.
pub trait HostedLayers: Planes + GpuInstalls {
    /// The platform object the host hands over: an `NSView` on macOS, a
    /// `CALayer` on iOS, a `SurfaceControl` on Android, an `HtmlElement` on
    /// the web.
    type Object: Clone + crate::RenderTransfer + 'static;
    /// Binds `object` as the content of `layer` on `surface`, its own
    /// coordinate space mapped onto the layer's content space with
    /// `(0, 0)..size` as its extent. Replaces the layer's other content; a
    /// rebinding of the same object on the same layer keeps its plane and
    /// moves only its geometry.
    fn bind_hosted(
        r: &mut Self::Renderer,
        surface: SurfaceId,
        layer: LayerId,
        object: Self::Object,
        size: kurbo::Size,
    );
}

/// A user shader's WGSL fragment source.
#[derive(Clone, Debug)]
pub struct ShaderSource {
    /// The fragment source, without the backend's prelude.
    pub source: Cow<'static, str>,
    /// Whether the shader animates: when true, it is re-rendered every
    /// frame so its time uniform advances and the engine keeps refreshing.
    pub animated: bool,
}

impl ShaderSource {
    /// A static shader from a WGSL fragment body.
    pub fn wgsl(fragment: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: fragment.into(),
            animated: false,
        }
    }

    /// Marks the shader as animated (re-rendered each frame).
    #[must_use]
    pub fn animated(self) -> Self {
        Self {
            source: self.source,
            animated: true,
        }
    }
}
