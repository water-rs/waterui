//! View-subtree capture into a Metal texture.
//!
//! [`ViewCapture`] renders a view subtree — including any GPU surfaces
//! nested inside it — into a caller-owned texture for the filter and
//! view-effect pipelines: the layer tree is rasterized synchronously by
//! `CALayer.renderInContext` into a `CGContext` whose backing store is a
//! shared `MTLBuffer`, a Metal texture view over that buffer lets the
//! compositor sample the native pixels with no CPU→GPU upload, each
//! [`CapturableSurface`] gets its own private texture, and a final pass on
//! a shared serial queue composites them under the captured overlay. The
//! raster itself is still CPU work — sharing storage makes the upload
//! free, not the drawing.
//!
//! # Orientation and scale contract
//!
//! The destination texture is top-down and sized in device pixels: texel
//! row 0 is the visually topmost row. The context's CTM maps the layer's
//! point space onto the pixel destination — scaling plus the
//! platform's orientation normalization — so the live layer transform
//! is never touched.
//!
//! # Safety
//!
//! The `unsafe` here calls `CGContext`/`CATransaction`/`MTLCommandBuffer`
//! entry points on objects this module owns or the caller has lent it, on
//! the threads the module contract names: every `ViewCapture` method and
//! every [`CapturableSurface`] call is main-thread only; `Compositor`
//! internals run on its private serial queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use dispatch2::{DispatchQoS, DispatchQueue, GlobalQueueIdentifier, MainThreadBound};
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGBitmapContextCreate, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo, CGImageComponentInfo,
};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLLibrary,
    MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder,
    MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResource,
    MTLResourceOptions, MTLSamplerAddressMode, MTLSamplerDescriptor, MTLSamplerMinMagFilter,
    MTLSamplerState, MTLScissorRect, MTLSize, MTLStorageMode, MTLStoreAction, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};
use objc2_quartz_core::{CALayer, CATransaction};

use crate::PlatformView;
use crate::core_animation::flush_transaction;
use crate::geometry::Rect;
use crate::main_queue::enqueue;

/// The MSL the composition pipeline is compiled from — `CaptureComposite`
/// in-tree, so the kit carries no bundle resources.
const CAPTURE_COMPOSITE_MSL: &str = include_str!("capture_composite.metal");

/// A value handed to the compositor's serial queue and handed back —
/// never *shared* across threads. `objc2` protocol objects are not `Send`,
/// so the crossing is made explicit here.
struct QueueSend<T>(
    // SAFETY: `T` is moved onto the serial queue once and never shared —
    // confinement, not sharing.
    T,
);

// SAFETY: the value is moved to the serial queue once and accessed nowhere
// else — the confinement capture work needs.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Send for QueueSend<T> {
    // SAFETY: the value is moved to the serial queue once and accessed nowhere
    // else — the confinement capture work needs.
}

impl<T> QueueSend<T> {
    /// Reads the value.
    const fn get(&self) -> &T {
        &self.0
    }
}

/// How a captured view's point-space bounds map onto the pixel destination.
#[derive(Clone, Copy, Debug)]
pub struct CaptureGeometry {
    /// Horizontal point-to-pixel scale.
    pub scale_x: f64,
    /// Vertical point-to-pixel scale.
    pub scale_y: f64,
}

impl CaptureGeometry {
    /// The geometry mapping `bounds` (non-empty, in points) onto a
    /// `width` × `height` pixel destination.
    ///
    /// # Panics
    ///
    /// When `bounds` is empty.
    #[must_use]
    pub fn new(bounds: Rect, width: usize, height: usize) -> Self {
        assert!(
            bounds.size.width > 0.0 && bounds.size.height > 0.0,
            "capture content must have non-zero bounds"
        );
        #[expect(
            clippy::cast_precision_loss,
            reason = "a capture texture is at most a few thousand pixels on a side"
        )]
        Self {
            scale_x: width as f64 / bounds.size.width,
            scale_y: height as f64 / bounds.size.height,
        }
    }
}

/// Where a GPU surface's own texture lands inside a capture.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSpec {
    /// The capture's identity for the surface — the surface view's address.
    pub surface_id: usize,
    /// Pixel-space origin inside the destination texture.
    pub origin: MTLOrigin,
    /// Pixel-space size inside the destination texture.
    pub size: MTLSize,
    /// The format the surface renders at.
    pub pixel_format: MTLPixelFormat,
}

/// The rect `bounds` (already in the capture's content space) maps to in a
/// `width` × `height` pixel destination: floored origin, ceiled size,
/// clamped to the target. `None` when it lands entirely outside.
#[must_use]
pub fn surface_spec(
    surface_id: usize,
    bounds: Rect,
    geometry: CaptureGeometry,
    pixel_format: MTLPixelFormat,
    target_width: usize,
    target_height: usize,
) -> Option<SurfaceSpec> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "captured rects are small, finite, and clamped to the target"
    )]
    let origin_x = (bounds.origin.x * geometry.scale_x).floor().max(0.0) as usize;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "captured rects are small, finite, and clamped to the target"
    )]
    let origin_y = (bounds.origin.y * geometry.scale_y).floor().max(0.0) as usize;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "captured rects are small, finite, and clamped to the target"
    )]
    let width = (bounds.size.width * geometry.scale_x).ceil() as usize;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "captured rects are small, finite, and clamped to the target"
    )]
    let height = (bounds.size.height * geometry.scale_y).ceil() as usize;
    let width = width.min(target_width.saturating_sub(origin_x.min(target_width)));
    let height = height.min(target_height.saturating_sub(origin_y.min(target_height)));
    (width > 0 && height > 0).then_some(SurfaceSpec {
        surface_id,
        origin: MTLOrigin {
            x: origin_x,
            y: origin_y,
            z: 0,
        },
        size: MTLSize {
            width,
            height,
            depth: 1,
        },
        pixel_format,
    })
}

/// The signal that a prepared surface frame produced no usable pixels:
/// its texture must not be sampled.
///
/// Reported when the frame was never submitted — a lost context between
/// preparation and submission — and when an in-flight submission was
/// lost to device failure. Every fence in the batch still settles, and
/// nothing composes the missing frame; fatal programming errors still
/// fail fast rather than reporting through this outcome.
#[derive(Clone, Copy, Debug)]
pub struct CaptureDeferred;

/// The callback a surface render request answers with — run on the main
/// thread, exactly once per accepted request.
///
/// `Ok(())` means the frame's texture carries usable pixels;
/// `Err(CaptureDeferred)` means it produced none.
pub type SurfaceCaptureCompletion = Box<dyn FnOnce(Result<(), CaptureDeferred>) + Send>;

/// A GPU surface a [`ViewCapture`] can capture.
///
/// Implemented by the backend's surface leaf; every method — and the
/// completion
/// [`render_prepared_external_texture`](CapturableSurface::render_prepared_external_texture)
/// receives — runs on the main thread.
pub trait CapturableSurface {
    /// The Metal pixel format this surface presents at.
    fn capture_pixel_format(&self) -> MTLPixelFormat;
    /// The surface view's bounds in `relative_to`'s coordinate space.
    fn content_bounds(&self, relative_to: &PlatformView) -> Rect;
    /// Suspends the surface's own presentation while its content is captured
    /// elsewhere.
    fn begin_capture_suppression(&self);
    /// Resumes the surface's presentation.
    fn end_capture_suppression(&self);
    /// Redirects the surface's redraw requests to `on_redraw`; while
    /// external, the surface presents nowhere itself.
    fn begin_external_rendering(&self, on_redraw: Rc<dyn Fn()>);
    /// Ends external rendering; `resume` restarts normal presentation.
    fn end_external_rendering(&self, resume: bool);
    /// Makes `texture` this surface's render target and reports whether its
    /// renderer setup is complete.
    fn prepare_external_render(&self, texture: &ProtocolObject<dyn MTLTexture>) -> bool;
    /// Renders one frame into the prepared `texture` at `width`×`height`
    /// pixels; `completion` reports the frame's outcome on the main
    /// thread, exactly once.
    fn render_prepared_external_texture(
        &self,
        texture: &ProtocolObject<dyn MTLTexture>,
        width: u32,
        height: u32,
        completion: SurfaceCaptureCompletion,
    );
    /// Whether at least one frame of this output has reached the screen
    /// since mount — first-paint readiness. On-screen readiness comes only
    /// from a real presentation receipt; an offscreen capture completion
    /// never answers it.
    fn has_presented_frame(&self) -> bool;
    /// Whether this output participates in its window's first-paint
    /// readiness — `participatesInFirstPaintReady`. A view whose window
    /// cannot present (hidden, occluded, zero-alpha, degenerate bounds,
    /// detached) is not a participant: first-frame waiters skip it and it
    /// owes no frame. Non-participation is never reported as presented.
    fn participates_in_first_paint(&self) -> bool;
    /// Arms `waker` to wake after the next successfully presented frame;
    /// an output that has already presented may leave it unarmed.
    fn register_ready_waiter(&self, waker: std::task::Waker);
}

impl fmt::Debug for dyn CapturableSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapturableSurface").finish_non_exhaustive()
    }
}

/// Counts down surface submissions; `completion` runs when the last one
/// settles — `Err(CaptureDeferred)` when any fence reported no usable
/// pixels. Completed on the main thread.
pub struct FenceBatch {
    remaining: AtomicUsize,
    failed: AtomicBool,
    completion: Mutex<Option<SurfaceCaptureCompletion>>,
}

impl fmt::Debug for FenceBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FenceBatch")
            .field("remaining", &self.remaining.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl FenceBatch {
    /// A batch of `count` fences.
    ///
    /// # Panics
    ///
    /// `count` must be non-zero.
    #[must_use]
    pub fn new(
        count: usize,
        completion: impl FnOnce(Result<(), CaptureDeferred>) + Send + 'static,
    ) -> Self {
        assert!(
            count > 0,
            "a GPU fence batch must contain at least one submission"
        );
        Self {
            remaining: AtomicUsize::new(count),
            failed: AtomicBool::new(false),
            completion: Mutex::new(Some(Box::new(completion))),
        }
    }

    /// One fence settled: `Ok` when its frame's pixels are usable,
    /// `Err(CaptureDeferred)` when the surface never submitted it. Either
    /// way the batch waits on every outstanding fence — a deferred frame
    /// never releases the submissions still in flight.
    ///
    /// # Panics
    ///
    /// When more fences land than the batch was built with.
    pub fn complete_one(&self, outcome: Result<(), CaptureDeferred>) {
        if outcome.is_err() {
            self.failed.store(true, Ordering::Relaxed);
        }
        // The store above is ordered before this release decrement, so the
        // fence that observes `remaining == 1` sees every reported failure.
        let remaining = self.remaining.fetch_sub(1, Ordering::AcqRel);
        assert!(remaining > 0, "a GPU fence batch completed more than once");
        if remaining == 1
            && let Some(completion) = self.completion.lock().expect("fence batch lock").take()
        {
            completion(if self.failed.load(Ordering::Relaxed) {
                Err(CaptureDeferred)
            } else {
                Ok(())
            });
        }
    }
}

/// One surface's destination texture, as [`CompositorGuard`] hands it back.
#[derive(Clone, Debug)]
pub struct RenderedSurface {
    /// The spec it was prepared from.
    pub spec: SurfaceSpec,
    /// Its private composite texture.
    pub texture: Retained<ProtocolObject<dyn MTLTexture>>,
}

/// Every cache the compositor built on one `MTLDevice`.
///
/// The command queue, per-surface textures, composite pipeline and
/// sampler are all created from `device`, so a device identity change
/// must replace the whole bundle before any of them is read — the only
/// path that binds a bundle is
/// [`CompositorState::device_resources`].
struct DeviceResources {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    surface_textures: HashMap<usize, Retained<ProtocolObject<dyn MTLTexture>>>,
    pipeline: Option<Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
    pipeline_format: Option<MTLPixelFormat>,
    sampler: Option<Retained<ProtocolObject<dyn MTLSamplerState>>>,
}

impl DeviceResources {
    /// A fresh bundle bound to `device`.
    ///
    /// # Panics
    ///
    /// When `device` cannot create a command queue.
    fn new(device: &ProtocolObject<dyn MTLDevice>) -> Self {
        Self {
            device: device.retain(),
            command_queue: device
                .newCommandQueue()
                .expect("failed to create the Metal capture composition command queue"),
            surface_textures: HashMap::new(),
            pipeline: None,
            pipeline_format: None,
            sampler: None,
        }
    }

    /// The private texture for `spec` — reused when id, size and format
    /// all match.
    ///
    /// # Panics
    ///
    /// When the device cannot allocate a texture.
    fn surface_texture(&mut self, spec: SurfaceSpec) -> Retained<ProtocolObject<dyn MTLTexture>> {
        if let Some(texture) = self.surface_textures.get(&spec.surface_id) {
            let matches = texture.width() == spec.size.width
                && texture.height() == spec.size.height
                && texture.pixelFormat() == spec.pixel_format;
            if matches {
                return texture.clone();
            }
        }
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                spec.pixel_format,
                spec.size.width,
                spec.size.height,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        descriptor.setStorageMode(MTLStorageMode::Private);
        let texture = self
            .device
            .newTextureWithDescriptor(&descriptor)
            .expect("failed to create a GPU surface capture texture");
        self.surface_textures
            .insert(spec.surface_id, texture.clone());
        texture
    }

    /// The pipeline for `format`, compiled once per format.
    ///
    /// # Panics
    ///
    /// When the in-tree shader fails to compile or lacks its entry points.
    fn render_pipeline(
        &mut self,
        format: MTLPixelFormat,
    ) -> Retained<ProtocolObject<dyn MTLRenderPipelineState>> {
        if let Some(pipeline) = &self.pipeline
            && self.pipeline_format == Some(format)
        {
            return pipeline.clone();
        }
        let source = NSString::from_str(CAPTURE_COMPOSITE_MSL);
        let library = self
            .device
            .newLibraryWithSource_options_error(&source, None)
            .expect("failed to compile the capture composite Metal library");
        let vertex = library
            .newFunctionWithName(&NSString::from_str("capture_composite_vertex"))
            .expect("CaptureComposite is missing capture_composite_vertex");
        let fragment = library
            .newFunctionWithName(&NSString::from_str("capture_composite_fragment"))
            .expect("CaptureComposite is missing capture_composite_fragment");
        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(&vertex));
        descriptor.setFragmentFunction(Some(&fragment));
        // SAFETY: index 0 is the single color attachment.
        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setPixelFormat(format);
        attachment.setBlendingEnabled(true);
        attachment.setRgbBlendOperation(MTLBlendOperation::Add);
        attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
        attachment.setSourceRGBBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        let compiled = self
            .device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .expect("failed to compile the Metal capture composition pipeline");
        self.pipeline = Some(compiled.clone());
        self.pipeline_format = Some(format);
        compiled
    }

    /// The linear clamp-to-edge sampler.
    ///
    /// # Panics
    ///
    /// When the device cannot create one.
    fn composite_sampler(&mut self) -> Retained<ProtocolObject<dyn MTLSamplerState>> {
        if let Some(sampler) = &self.sampler {
            return sampler.clone();
        }
        let descriptor = MTLSamplerDescriptor::new();
        descriptor.setMinFilter(MTLSamplerMinMagFilter::Linear);
        descriptor.setMagFilter(MTLSamplerMinMagFilter::Linear);
        descriptor.setSAddressMode(MTLSamplerAddressMode::ClampToEdge);
        descriptor.setTAddressMode(MTLSamplerAddressMode::ClampToEdge);
        let sampler = self
            .device
            .newSamplerStateWithDescriptor(&descriptor)
            .expect("failed to create the Metal capture composition sampler");
        self.sampler = Some(sampler.clone());
        sampler
    }
}

/// The compositor's queue-confined state.
///
/// SAFETY: its `Retained` Metal objects are only ever touched from the
/// serial queue the mutex hands the lock to; `objc2` does not mark its
/// protocol objects `Send`, so the marker is asserted here.
struct CompositorState {
    // SAFETY: every `Retained` field is reached only on the serial queue.
    resources: Option<DeviceResources>,
}

impl CompositorState {
    /// The caches bound to `device` — the single bind point every
    /// device-dependent entry point goes through.
    ///
    /// An identity change drops the previous bundle's own strong refs —
    /// textures already handed to in-flight captures stay alive through
    /// theirs — and binds a fresh bundle before any cache is read.
    ///
    /// # Panics
    ///
    /// When `device` cannot create a command queue.
    fn device_resources(&mut self, device: &ProtocolObject<dyn MTLDevice>) -> &mut DeviceResources {
        let stale = self.resources.as_ref().is_none_or(|resources| {
            Retained::as_ptr(&resources.device) != core::ptr::from_ref(device)
        });
        if stale {
            self.resources = Some(DeviceResources::new(device));
        }
        self.resources
            .as_mut()
            .expect("a device bundle is bound above")
    }
}

// SAFETY: the state is only ever reached through `Mutex`, and only on the
// compositor's serial queue.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for CompositorState {
    // SAFETY: the state is only ever reached through `Mutex`, and only on the
    // compositor's serial queue.
}

/// A scope for the compositor's queue-confined state, valid inside
/// [`Compositor::perform`].
pub struct CompositorGuard<'a> {
    state: std::sync::MutexGuard<'a, CompositorState>,
}

impl fmt::Debug for CompositorGuard<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompositorGuard").finish_non_exhaustive()
    }
}

/// The GPU half of a capture, confined to a private serial queue.
///
/// `perform` enqueues work onto the queue; the queue's seriality is the
/// mutual exclusion the guard hands to `work`.
#[derive(Clone)]
pub struct Compositor {
    queue: dispatch2::DispatchRetained<DispatchQueue>,
    state: Arc<Mutex<CompositorState>>,
}

impl fmt::Debug for Compositor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compositor").finish_non_exhaustive()
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    /// A compositor whose work runs on a serial queue targeting the
    /// user-interactive global queue.
    #[must_use]
    pub fn new() -> Self {
        let target = DispatchQueue::global_queue(GlobalQueueIdentifier::QualityOfService(
            DispatchQoS::UserInteractive,
        ));
        Self {
            queue: DispatchQueue::new_with_target(
                "dev.cocoaui.graphics.capture-composition",
                None,
                Some(&target),
            ),
            state: Arc::new(Mutex::new(CompositorState { resources: None })),
        }
    }

    /// Runs `work` on the serial queue with the compositor's state.
    ///
    /// # Panics
    ///
    /// When the compositor's state lock is poisoned.
    pub fn perform(&self, work: impl FnOnce(&mut CompositorGuard) + Send + 'static) {
        let state = self.state.clone();
        self.queue.exec_async(move || {
            let mut guard = CompositorGuard {
                state: state.lock().expect("capture compositor lock poisoned"),
            };
            work(&mut guard);
        });
    }

    /// Drops cached per-surface textures once in-flight work drains.
    pub fn discard_resources(&self) {
        self.perform(|guard| {
            if let Some(resources) = &mut guard.state.resources {
                resources.surface_textures.clear();
            }
        });
    }
}

impl CompositorGuard<'_> {
    /// A command buffer from the queue bound to `device`; the queue —
    /// like every cache — is recreated when the device changes.
    ///
    /// # Panics
    ///
    /// When `device` cannot create a command queue or buffer.
    pub fn make_command_buffer(
        &mut self,
        device: &ProtocolObject<dyn MTLDevice>,
    ) -> Retained<ProtocolObject<dyn MTLCommandBuffer>> {
        self.state
            .device_resources(device)
            .command_queue
            .commandBuffer()
            .expect("failed to create the Metal view composition command buffer")
    }

    /// The private texture each surface renders into — pruned to `specs`,
    /// reused across captures while the bound device is unchanged.
    ///
    /// # Panics
    ///
    /// When `device` cannot allocate a texture.
    pub fn prepare_surface_textures(
        &mut self,
        specs: &[SurfaceSpec],
        device: &ProtocolObject<dyn MTLDevice>,
    ) -> Vec<RenderedSurface> {
        let resources = self.state.device_resources(device);
        resources
            .surface_textures
            .retain(|id, _| specs.iter().any(|spec| spec.surface_id == *id));
        specs
            .iter()
            .map(|&spec| RenderedSurface {
                spec,
                texture: resources.surface_texture(spec),
            })
            .collect()
    }

    /// Encodes the composite pass: each surface's texture at its viewport and
    /// scissor, then the overlay full-screen, into `target`.
    ///
    /// Premultiplied-over blending (`.one` / `.oneMinusSourceAlpha`), a
    /// linear clamp-to-edge sampler, a fullscreen triangle per region — the
    /// `CaptureComposite` shader.
    ///
    /// # Panics
    ///
    /// When the pipeline or encoder cannot be created.
    pub fn encode_composition(
        &mut self,
        surfaces: &[RenderedSurface],
        overlay: Option<&ProtocolObject<dyn MTLTexture>>,
        target: &ProtocolObject<dyn MTLTexture>,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
        device: &ProtocolObject<dyn MTLDevice>,
    ) {
        let resources = self.state.device_resources(device);
        let descriptor = MTLRenderPassDescriptor::new();
        // SAFETY: index 0 is the single color attachment.
        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(target));
        attachment.setLoadAction(MTLLoadAction::Clear);
        attachment.setStoreAction(MTLStoreAction::Store);
        attachment.setClearColor(MTLClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 0.0,
        });
        let encoder = command_buffer
            .renderCommandEncoderWithDescriptor(&descriptor)
            .expect("failed to create the Metal capture composition encoder");
        encoder.setRenderPipelineState(&resources.render_pipeline(target.pixelFormat()));
        let sampler = resources.composite_sampler();
        // SAFETY: `encoder` is a live render encoder and index 0 is the
        // sampler slot the shader binds.
        unsafe {
            encoder.setFragmentSamplerState_atIndex(Some(&sampler), 0);
        }
        for surface in surfaces {
            let spec = surface.spec;
            encoder.setViewport(MTLViewport {
                originX: f64::from(u32::try_from(spec.origin.x).unwrap_or(u32::MAX)),
                originY: f64::from(u32::try_from(spec.origin.y).unwrap_or(u32::MAX)),
                width: f64::from(u32::try_from(spec.size.width).unwrap_or(u32::MAX)),
                height: f64::from(u32::try_from(spec.size.height).unwrap_or(u32::MAX)),
                znear: 0.0,
                zfar: 1.0,
            });
            encoder.setScissorRect(MTLScissorRect {
                x: spec.origin.x,
                y: spec.origin.y,
                width: spec.size.width,
                height: spec.size.height,
            });
            // SAFETY: `encoder` is a live render encoder and index 0 is the
            // texture slot the shader binds.
            unsafe {
                encoder.setFragmentTexture_atIndex(Some(&surface.texture), 0);
                encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
            }
        }
        if let Some(overlay) = overlay {
            encoder.setViewport(MTLViewport {
                originX: 0.0,
                originY: 0.0,
                width: f64::from(u32::try_from(target.width()).unwrap_or(u32::MAX)),
                height: f64::from(u32::try_from(target.height()).unwrap_or(u32::MAX)),
                znear: 0.0,
                zfar: 1.0,
            });
            encoder.setScissorRect(MTLScissorRect {
                x: 0,
                y: 0,
                width: target.width(),
                height: target.height(),
            });
            // SAFETY: same encoder/slot contract as above.
            unsafe {
                encoder.setFragmentTexture_atIndex(Some(overlay), 0);
                encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
            }
        }
        encoder.endEncoding();
    }
}

/// One native raster destination: a shared `MTLBuffer` the CPU and GPU
/// both address, the `CGContext` the layer tree draws into, and the
/// Metal texture view over the same storage the compositor samples — so
/// native pixels reach the composite pass with no upload.
///
/// A frame is bound to its exact generation and geometry: the caller's
/// context-generation token, the capture device, the destination pixel
/// format, and the pixel size. Any change to those rebuilds it, because
/// the buffer stride, the context's bitmap layout, and the texture view
/// are baked at creation — and because storage issued under one context
/// generation is invalid for another even when the device object is
/// identical.
#[derive(Debug)]
struct NativeRasterFrame {
    /// The capture device — retained so the buffer and texture outlive a
    /// teardown; device identity alone is NOT the generation key.
    _device: Retained<ProtocolObject<dyn MTLDevice>>,
    /// The shared pixel storage the context draws into — owned here so
    /// the context's target memory stays valid for the frame's lifetime.
    _buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    /// The texture view over the buffer the compositor samples.
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    /// The raster context drawing into the buffer.
    context: CFRetained<CGContext>,
    /// The caller's context-generation token this frame was issued for.
    generation: u64,
    /// Destination width in pixels.
    pixel_width: usize,
    /// Destination height in pixels.
    pixel_height: usize,
    /// The destination's pixel format.
    pixel_format: MTLPixelFormat,
}

/// The (bits per component, `CGBitmapInfo`, bytes per pixel) a capture
/// pixel format maps to — the context and the texture view must agree on
/// one layout or the sampled pixels are garbage.
fn raster_layout(pixel_format: MTLPixelFormat) -> (usize, u32, usize) {
    match pixel_format {
        MTLPixelFormat::BGRA8Unorm | MTLPixelFormat::BGRA8Unorm_sRGB => (
            8,
            CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0,
            4,
        ),
        MTLPixelFormat::RGBA8Unorm | MTLPixelFormat::RGBA8Unorm_sRGB => (
            8,
            CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0,
            4,
        ),
        // 64-bit half-float RGBA: `kCGBitmapFloatComponents` + little-endian
        // 16-bit words is the layout `RGBA16Float` shares.
        MTLPixelFormat::RGBA16Float => (
            16,
            CGImageAlphaInfo::PremultipliedLast.0
                | CGImageComponentInfo::Float.0
                | CGImageByteOrderInfo::Order16Little.0,
            8,
        ),
        other => panic!("the native raster destination has no bitmap layout for {other:?}"),
    }
}

impl NativeRasterFrame {
    /// Builds a raster destination on `device` for `width` × `height`
    /// pixels of `pixel_format`.
    ///
    /// # Panics
    ///
    /// When the device cannot allocate the shared buffer or its texture
    /// view, or CoreGraphics refuses the destination's bitmap layout — a
    /// capture target of a format `raster_layout` rejects never reaches
    /// here, and every other failure means the pixel contract could not
    /// be met, so there is nothing to degrade to.
    fn new(
        device: &ProtocolObject<dyn MTLDevice>,
        pixel_format: MTLPixelFormat,
        pixel_width: usize,
        pixel_height: usize,
        generation: u64,
    ) -> Self {
        let (bits_per_component, bitmap_info, bytes_per_pixel) = raster_layout(pixel_format);
        // The texture view and the context must stride-identically agree
        // with the device: rows are padded to the minimum linear-texture
        // alignment for the format.
        let alignment = device.minimumLinearTextureAlignmentForPixelFormat(pixel_format);
        assert!(
            alignment > 0,
            "the device reported no linear texture alignment for {pixel_format:?}"
        );
        let row_bytes = (pixel_width * bytes_per_pixel).div_ceil(alignment) * alignment;
        let buffer = device
            .newBufferWithLength_options(
                row_bytes * pixel_height,
                MTLResourceOptions::StorageModeShared,
            )
            .expect("failed to allocate the native capture raster buffer");
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                pixel_format,
                pixel_width,
                pixel_height,
                false,
            )
        };
        // The composite pass only samples the overlay — but keeping the
        // render-target bit mirrors every other capture texture's usage.
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        let texture = buffer
            .newTextureWithDescriptor_offset_bytesPerRow(&descriptor, 0, row_bytes)
            .expect("failed to create the buffer-backed native capture texture");
        let color_space = crate::metal::color_space(pixel_format);
        // SAFETY: `buffer.contents()` is valid for the buffer's whole
        // length — `row_bytes * pixel_height` — for the context's entire
        // lifetime, which `self` bounds by owning the buffer; the layout
        // arguments are the same `row_bytes`/format pair the texture view
        // was created with.
        let context = unsafe {
            CGBitmapContextCreate(
                buffer.contents().as_ptr(),
                pixel_width,
                pixel_height,
                bits_per_component,
                row_bytes,
                Some(&color_space),
                bitmap_info,
            )
        }
        .expect("failed to create the native raster CGContext");
        Self {
            _device: device.retain(),
            _buffer: buffer,
            texture,
            context,
            generation,
            pixel_width,
            pixel_height,
            pixel_format,
        }
    }

    /// Rasterizes `layer`'s tree into the shared buffer at `geometry`'s
    /// scale — a synchronous CPU draw, completed when it returns.
    ///
    /// Geometry is expressed through the context's CTM alone: the live
    /// layer tree is never transformed or reparented. The CTM is applied
    /// after clearing and restored before returning, so a reused context
    /// carries no state across frames. The output obeys the composite
    /// pass's contract — texel row 0 is the view's top edge: a bitmap
    /// context is bottom-left-origin like `AppKit`'s layer space, so on
    /// macOS a plain scale lands each layer row on its matching texel
    /// row (the kit's flipped views already compensate the y-up draw,
    /// and mirroring again would count the flip twice); `UIKit`'s layer
    /// space is top-left-origin, so the iOS CTM translates then flips to
    /// put the view's top on row 0.
    fn draw(&self, layer: &CALayer, geometry: CaptureGeometry) {
        let context: &CGContext = &self.context;
        // SAFETY: the casts stay representable — a capture destination is
        // at most a few thousand pixels on a side.
        #[expect(
            clippy::cast_precision_loss,
            reason = "a capture texture is at most a few thousand pixels on a side"
        )]
        let (width, height) = (self.pixel_width as f64, self.pixel_height as f64);
        CGContext::clear_rect(Some(context), Rect::new(0.0, 0.0, width, height).into());
        CGContext::save_g_state(Some(context));
        #[cfg(target_os = "ios")]
        {
            CGContext::translate_ctm(Some(context), 0.0, height);
            CGContext::scale_ctm(Some(context), geometry.scale_x, -geometry.scale_y);
        }
        #[cfg(target_os = "macos")]
        CGContext::scale_ctm(Some(context), geometry.scale_x, geometry.scale_y);
        layer.renderInContext(context);
        CGContext::restore_g_state(Some(context));
        CGContext::flush(Some(context));
    }
}

/// A leased raster frame: the destination is owned outright from the
/// moment it is drawn until the GPU work sampling it has settled, so a
/// later raster can never overwrite memory a consumer still reads. The
/// pool takes the frame back only through [`NativeRenderer::return_frame`],
/// driven by the compositor's completed handler on the main queue; any
/// other drop — cancellation, a superseded generation — releases the
/// storage without returning it.
#[derive(Debug)]
struct RasterLease {
    frame: Option<NativeRasterFrame>,
}

impl RasterLease {
    /// The texture the compositor samples.
    fn texture(&self) -> &Retained<ProtocolObject<dyn MTLTexture>> {
        &self.frame.as_ref().expect("a lease owns its frame").texture
    }

    /// Rasterizes `layer`'s tree into the leased destination — the draw
    /// half of a capture, infallible once the frame exists.
    fn draw(&self, layer: &CALayer, geometry: CaptureGeometry) {
        self.frame
            .as_ref()
            .expect("a lease owns its frame")
            .draw(layer, geometry);
    }

    /// Hands the frame back — only the settle path may call this.
    fn into_frame(mut self) -> NativeRasterFrame {
        self.frame.take().expect("a lease owns its frame")
    }
}

/// The full key a raster frame is pooled under: the caller's generation
/// token plus the destination's device, pixel format and pixel size. Any
/// key change retires the whole pool — a `NativeRenderer` represents one
/// current capture destination, not a cache of every geometry it has
/// ever seen.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RasterKey {
    generation: u64,
    device: *const std::ffi::c_void,
    pixel_format: MTLPixelFormat,
    pixel_width: usize,
    pixel_height: usize,
}

impl fmt::Debug for RasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RasterKey")
            .field("generation", &self.generation)
            .field("device", &self.device)
            .field("pixel_format", &self.pixel_format)
            .field("pixel_width", &self.pixel_width)
            .field("pixel_height", &self.pixel_height)
            .finish()
    }
}

/// The native raster half of a capture, confined to the main thread: a
/// small pool of `NativeRasterFrame`s leased per capture and returned
/// when the GPU settles, all under one current [`RasterKey`].
#[derive(Debug, Default)]
struct NativeRenderer {
    /// The key the pool currently issues for. Any mismatch drains
    /// `available`: storage from another key — an old generation, a
    /// replaced device, a different size or format — can never answer
    /// this key's capture.
    key: Option<RasterKey>,
    /// Settled frames under `key`, ready to reissue.
    available: Vec<NativeRasterFrame>,
}

impl NativeRenderer {
    /// Issues a frame for the full (`generation`, `device`, `format`,
    /// `width` × `height`) key and returns the lease owning it through
    /// consumption.
    ///
    /// # Panics
    ///
    /// When the frame cannot be created (see [`NativeRasterFrame::new`]).
    fn issue(
        &mut self,
        device: &ProtocolObject<dyn MTLDevice>,
        format: MTLPixelFormat,
        width: usize,
        height: usize,
        generation: u64,
    ) -> RasterLease {
        let key = RasterKey {
            generation,
            device: core::ptr::from_ref(device).cast(),
            pixel_format: format,
            pixel_width: width,
            pixel_height: height,
        };
        if self.key != Some(key) {
            self.key = Some(key);
            self.available.clear();
        }
        let frame = self
            .available
            .pop()
            .unwrap_or_else(|| NativeRasterFrame::new(device, format, width, height, generation));
        RasterLease { frame: Some(frame) }
    }

    /// Takes a settled frame back — main thread only, called from the
    /// compositor's completion path once the GPU stopped sampling it. A
    /// frame whose full key no longer matches the pool's current key is
    /// dropped instead: outstanding old-key storage never joins the new
    /// pool.
    fn return_frame(&mut self, frame: NativeRasterFrame) {
        let key = self
            .key
            .as_ref()
            .expect("a returned frame was issued under a key");
        if frame.generation == key.generation
            && frame.pixel_width == key.pixel_width
            && frame.pixel_height == key.pixel_height
            && frame.pixel_format == key.pixel_format
            && Retained::as_ptr(&frame.texture.device()) == key.device.cast()
        {
            self.available.push(frame);
        }
    }
}

/// A snapshot's two halves: the platform spec and the surface it came from.
struct CapturedSnapshot {
    spec: SurfaceSpec,
    surface: Rc<dyn CapturableSurface>,
}

/// One live external-render registration: a surface whose
/// `begin_external_rendering` has been paired once. The `Rc` is held by
/// the capture's `active` map AND by every outstanding `Preparation`
/// that snapshotted it, so the external render ends only when the map
/// parted the surface AND its last outstanding frame has settled —
/// always on the main thread, where every drop path lands.
struct SurfaceRegistration {
    surface: Rc<dyn CapturableSurface>,
}

impl Drop for SurfaceRegistration {
    fn drop(&mut self) {
        self.surface.end_external_rendering(true);
    }
}

/// Everything [`ViewCapture::capture`] decided on the main thread that the
/// compositor's queue needs. `raster` is the leased native output and
/// `surfaces` the registrations this capture's immutable snapshot owns:
/// both must outlive the last GPU read, so the preparation only releases
/// them through the composite buffer's completion.
struct Preparation {
    target: Retained<ProtocolObject<dyn MTLTexture>>,
    raster: RasterLease,
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    /// The registrations this capture's snapshot owns — kept alive past
    /// any membership change in `active` until the frame settles.
    surfaces: HashMap<usize, Rc<SurfaceRegistration>>,
}

/// Restores capture suppression on exactly the subset of `snapshots`
/// whose `begin_capture_suppression` completed — `begun` increments only
/// after each begin returns, so unwind-safe restoration on the main
/// thread can never end more than was opened. Suppression is restored
/// explicitly before the sole transaction commit, and again by Drop if
/// the pass exits early.
struct SuppressionGuard<'a> {
    snapshots: &'a [CapturedSnapshot],
    begun: usize,
}

impl<'a> SuppressionGuard<'a> {
    const fn new(snapshots: &'a [CapturedSnapshot]) -> Self {
        Self {
            snapshots,
            begun: 0,
        }
    }

    /// Opens suppression for every snapshot.
    ///
    /// # Panics
    ///
    /// When a surface's `begin_capture_suppression` panics — `Drop` then
    /// restores only the entries already begun.
    fn begin(&mut self) {
        while self.begun < self.snapshots.len() {
            self.snapshots[self.begun]
                .surface
                .begin_capture_suppression();
            self.begun += 1;
        }
    }

    /// Restores every opened snapshot, last-opened first.
    fn end(&mut self) {
        while self.begun > 0 {
            self.begun -= 1;
            self.snapshots[self.begun].surface.end_capture_suppression();
        }
    }
}

impl Drop for SuppressionGuard<'_> {
    fn drop(&mut self) {
        self.end();
    }
}

/// The one-shot capsule the composite completion owns through the
/// command buffer's life: the whole `Preparation` — its raster lease
/// and every CoreGraphics/Metal object in it stays untouched until it
/// settles on the main queue — the pool return target, and the caller's
/// completion. One pre-existing `Mutex` slot, nothing else.
struct Settle {
    preparation: QueueSend<Preparation>,
    return_to: MainThreadBound<Weak<ViewCapture>>,
    completion: Box<dyn Fn(bool) + Send>,
}

/// Answers the capturable GPU surface `view` presents, if any — the leaf-side
/// registry.
type SurfaceResolver = Rc<dyn Fn(&PlatformView) -> Option<Rc<dyn CapturableSurface>>>;

/// Captures `content`'s subtree into Metal textures. Main-thread only;
/// created once per effect view and shut down before the view drops.
pub struct ViewCapture {
    content: Retained<PlatformView>,
    resolve: SurfaceResolver,
    compositor: Compositor,
    on_redraw: RefCell<Option<Rc<dyn Fn()>>>,
    renderer: RefCell<NativeRenderer>,
    active: RefCell<HashMap<usize, Rc<SurfaceRegistration>>>,
}

impl fmt::Debug for ViewCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewCapture")
            .field("active", &self.active.borrow().len())
            .finish_non_exhaustive()
    }
}

impl ViewCapture {
    /// A capture for `content`. `resolve` answers the capturable GPU surface
    /// `view` presents, if any — the leaf-side registry.
    pub fn new(
        _mtm: objc2::MainThreadMarker,
        content: Retained<PlatformView>,
        resolve: impl Fn(&PlatformView) -> Option<Rc<dyn CapturableSurface>> + 'static,
    ) -> Self {
        Self {
            content,
            resolve: Rc::new(resolve),
            compositor: Compositor::new(),
            on_redraw: RefCell::new(None),
            renderer: RefCell::new(NativeRenderer::default()),
            active: RefCell::new(HashMap::new()),
        }
    }

    /// The redraw hook external surfaces call — installed by the owning
    /// effect view before any capture runs.
    pub fn set_on_redraw(&self, redraw: impl Fn() + 'static) {
        *self.on_redraw.borrow_mut() = Some(Rc::new(redraw));
    }

    /// Captures `content` into `target`; `completion` runs on the main
    /// thread with whether the frame landed. `generation` is the caller's
    /// context-generation token: it must come from the exact retained
    /// context `target` was allocated under — a later replacement must
    /// not relabel old storage, and a replacement that wraps the same
    /// physical device still invalidates every pooled frame.
    ///
    /// # Panics
    ///
    /// When called off the main thread.
    pub fn capture(
        self: &Rc<Self>,
        target: &ProtocolObject<dyn MTLTexture>,
        generation: u64,
        completion: impl Fn(bool) + Send + 'static,
    ) {
        let mtm = objc2::MainThreadMarker::new().expect("capture runs on the main thread");
        let (preparation, specs) = self.prepare(target, generation);

        // The raster pass lands its pixels in the leased frame's shared
        // buffer, not `target` — and it is synchronous: no GPU fence
        // stands in for it. Every capture still runs the composition
        // pass to draw the overlay over `target`, external surfaces or
        // not.
        //
        // Everything leaving the main thread is `Send`: the specs are
        // Copy, the compositor is shareable, `completion` is Send — and
        // `this` rides a `MainThreadBound`, only ever upgraded on the
        // main queue.
        let compositor = self.compositor.clone();
        let this = MainThreadBound::new(Rc::downgrade(self), mtm);
        let preparation = QueueSend(preparation);
        let mut completion = Some(Box::new(completion) as Box<dyn Fn(bool) + Send>);

        compositor.perform(move |guard| {
            let rendered =
                QueueSend(guard.prepare_surface_textures(&specs, &preparation.get().device));
            let mut preparation = Some(preparation);
            enqueue(move |mtm| {
                // Whole-binding move keeps the `QueueSend` wrapper —
                // capturing `rendered.0` would capture the bare Vec.
                let rendered = rendered;
                // The caller settles exactly once even when the owner is
                // gone: teardown mid-capture is a deferral, never a
                // silent drop that could hang a parent fence batch.
                let (Some(completion), Some(preparation)) = (completion.take(), preparation.take())
                else {
                    return;
                };
                match this.get(mtm).upgrade() {
                    Some(capture) => {
                        capture.submit_surfaces(&rendered.0, preparation.0, completion);
                    }
                    None => completion(false),
                }
            });
        });
    }

    /// Ends every external surface's presentation and releases GPU state.
    /// Call before the owning view drops.
    ///
    /// Dropping a registration ends its external rendering — a capture
    /// that still owns one keeps it until that capture settles.
    pub fn shutdown(&self) {
        self.active.borrow_mut().clear();
        *self.renderer.borrow_mut() = NativeRenderer::default();
        self.compositor.discard_resources();
    }

    /// Everything `capture` needs decided on the main thread: the
    /// preparation — its lease owns the rastered native output — and
    /// the surface spec list.
    fn prepare(
        &self,
        target: &ProtocolObject<dyn MTLTexture>,
        generation: u64,
    ) -> (Preparation, Vec<SurfaceSpec>) {
        let content = &*self.content;
        let was_hidden = crate::view::is_hidden(content);
        self.set_content_hidden(false);
        let restore = HiddenRestore {
            owner: self,
            was: was_hidden,
        };

        crate::view::prepare_for_capture(content);
        let layer = crate::view::layer(content).expect("a capture view must be layer-backed");
        let geometry = CaptureGeometry::new(
            crate::view::bounds(content),
            target.width(),
            target.height(),
        );
        let snapshots = self.collect_snapshots(target, geometry);
        self.update_external_surfaces(&snapshots);
        // The registrations this capture owns — the immutable snapshot
        // of who must stay external until its frame settles.
        let surfaces: HashMap<usize, Rc<SurfaceRegistration>> = {
            let active = self.active.borrow();
            snapshots
                .iter()
                .map(|s| {
                    (
                        s.spec.surface_id,
                        active
                            .get(&s.spec.surface_id)
                            .expect("a just-joined surface is registered")
                            .clone(),
                    )
                })
                .collect()
        };

        // Fallible work happens BEFORE any suppression mutation: a frame
        // allocation panic leaves the live tree untouched.
        let raster = self.renderer.borrow_mut().issue(
            &target.device(),
            target.pixelFormat(),
            target.width(),
            target.height(),
            generation,
        );

        // INVARIANT: the synchronous native raster pass runs inside one
        // outer disabled-actions `CATransaction` — suppression open, the
        // `renderInContext` draw, and suppression close all happen inside
        // it, then a single commit pushes every model change at once.
        // Nothing inside may open, commit or flush a transaction of its
        // own: a mid-pass commit would push the suppressed state to the
        // render server and flicker the on-screen tree. The suppression
        // setters (`CapturableSurface::begin/end_capture_suppression`)
        // are plain model mutations for exactly this reason, and the
        // guard restores exactly the subset it opened — synchronously
        // before the sole commit on success, from Drop on an early exit.
        // The layer tree itself is never transformed or reparented — the
        // destination geometry lives in the context's CTM.
        {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            let mut suppression = SuppressionGuard::new(&snapshots);
            suppression.begin();
            raster.draw(&layer, geometry);
            suppression.end();
            CATransaction::commit();
            flush_transaction();
        }

        let specs: Vec<SurfaceSpec> = snapshots.iter().map(|s| s.spec).collect();
        let preparation = Preparation {
            raster,
            target: target.retain(),
            device: target.device(),
            surfaces,
        };
        drop(restore);
        (preparation, specs)
    }

    /// Shows or re-hides the captured content without anything else seeing
    /// it — actions disabled, state restored before the frame ends.
    fn set_content_hidden(&self, hidden: bool) {
        if crate::view::is_hidden(&self.content) == hidden {
            return;
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        crate::view::set_hidden(&self.content, hidden);
        CATransaction::commit();
        flush_transaction();
    }

    /// The surface snapshot list for one capture.
    fn collect_snapshots(
        &self,
        target: &ProtocolObject<dyn MTLTexture>,
        geometry: CaptureGeometry,
    ) -> Vec<CapturedSnapshot> {
        let mut snapshots = Vec::new();
        self.collect_into(
            &self.content,
            &mut snapshots,
            geometry,
            target.width(),
            target.height(),
        );
        snapshots
    }

    /// Recursion over `view`'s subviews: a resolved surface snapshots and
    /// stops the descent.
    fn collect_into(
        &self,
        view: &PlatformView,
        snapshots: &mut Vec<CapturedSnapshot>,
        geometry: CaptureGeometry,
        target_width: usize,
        target_height: usize,
    ) {
        if let Some(surface) = (self.resolve)(view) {
            let bounds = surface.content_bounds(&self.content);
            if let Some(spec) = surface_spec(
                view_key(view),
                bounds,
                geometry,
                surface.capture_pixel_format(),
                target_width,
                target_height,
            ) {
                snapshots.push(CapturedSnapshot { spec, surface });
            }
            return;
        }
        for subview in crate::view::subviews(view) {
            self.collect_into(&subview, snapshots, geometry, target_width, target_height);
        }
    }

    /// Joins and parts external surfaces against this capture's snapshot
    /// list.
    ///
    /// # Panics
    ///
    /// When [`set_on_redraw`](Self::set_on_redraw) was never called.
    fn update_external_surfaces(&self, snapshots: &[CapturedSnapshot]) {
        let on_redraw = self
            .on_redraw
            .borrow()
            .clone()
            .expect("a view capture's redraw hook must be installed before capture");
        let mut active = self.active.borrow_mut();
        // Part first: a surface absent from this snapshot leaves the map
        // — its registration ends when the last owner (the map or an
        // outstanding capture's snapshot) drops on this thread.
        active.retain(|id, _| snapshots.iter().any(|s| s.spec.surface_id == *id));
        // Join: one registration per surface, begun once, `Rc`-shared by
        // the map and every capture that snapshots it.
        for snapshot in snapshots {
            active.entry(snapshot.spec.surface_id).or_insert_with(|| {
                snapshot.surface.begin_external_rendering(on_redraw.clone());
                Rc::new(SurfaceRegistration {
                    surface: snapshot.surface.clone(),
                })
            });
        }
    }

    /// Final GPU half: each surface renders into its private texture, then a
    /// fence batch composites them. Main thread.
    fn submit_surfaces(
        self: &Rc<Self>,
        rendered: &[RenderedSurface],
        preparation: Preparation,
        completion: Box<dyn Fn(bool) + Send>,
    ) {
        let mtm = objc2::MainThreadMarker::new().expect("capture flow runs on the main thread");
        let return_to = MainThreadBound::new(Rc::downgrade(self), mtm);
        if rendered.is_empty() {
            // No external surfaces to fence on: the overlay is already
            // rastered — straight to composition.
            Self::compose(
                &self.compositor,
                QueueSend(preparation),
                QueueSend(Vec::new()),
                completion,
                return_to,
            );
            return;
        }
        // The surfaces come from THIS capture's owned snapshot, never
        // the mutable `active` map: a second capture or a changed
        // subtree cannot un-register a surface an outstanding frame
        // still depends on.
        let surfaces: Vec<Rc<SurfaceRegistration>> = rendered
            .iter()
            .map(|item| {
                preparation
                    .surfaces
                    .get(&item.spec.surface_id)
                    .cloned()
                    .expect("the capture's snapshot owns every rendered surface")
            })
            .collect();
        for (item, registration) in rendered.iter().zip(&surfaces) {
            if !registration.surface.prepare_external_render(&item.texture) {
                // Setup still pending: the frame defers.
                completion(false);
                return;
            }
        }

        let batch = Arc::new(FenceBatch::new(rendered.len(), {
            let compositor = self.compositor.clone();
            let rendered = QueueSend(rendered.to_owned());
            let preparation = QueueSend(preparation);
            move |outcome| {
                if outcome.is_ok() {
                    Self::compose(&compositor, preparation, rendered, completion, return_to);
                } else {
                    // No usable pixels: the frame never submitted —
                    // release its lease unreturned. Destruction still
                    // happens on the main queue where the preparation's
                    // CG/Metal objects belong, then the failure reports.
                    enqueue(move |_| {
                        drop(preparation);
                    });
                    completion(false);
                }
            }
        }));
        for (item, registration) in rendered.iter().zip(&surfaces) {
            let batch = Arc::clone(&batch);
            registration.surface.render_prepared_external_texture(
                &item.texture,
                u32::try_from(item.spec.size.width).expect("a surface is smaller than u32"),
                u32::try_from(item.spec.size.height).expect("a surface is smaller than u32"),
                Box::new(move |outcome| batch.complete_one(outcome)),
            );
        }
    }

    /// The composition pass — deliberately free of `self` so it still
    /// lands if the owning view is torn down meanwhile. `return_to`
    /// returns the raster lease to its pool once the GPU is actually
    /// done sampling it.
    fn compose(
        compositor: &Compositor,
        preparation: QueueSend<Preparation>,
        rendered: QueueSend<Vec<RenderedSurface>>,
        completion: Box<dyn Fn(bool) + Send>,
        return_to: MainThreadBound<Weak<Self>>,
    ) {
        compositor.perform(move |guard| {
            let command_buffer = guard.make_command_buffer(&preparation.get().device);
            guard.encode_composition(
                rendered.get(),
                Some(preparation.get().raster.texture()),
                &preparation.get().target,
                &command_buffer,
                &preparation.get().device,
            );
            // The lease is held until this command buffer completes:
            // only then has the GPU stopped sampling the shared buffer.
            // The completed handler's contract is once, on Metal's
            // completion queue — a single pre-existing `Mutex` slot
            // holds the whole capsule through it, and one `enqueue`
            // settles everything on the main thread in order: the frame
            // returns to the pool, then the completion fires. Nothing
            // CG/Metal-owned drops on the completion queue.
            let settle = Mutex::new(Some(Settle {
                preparation,
                return_to,
                completion,
            }));
            let handler = RcBlock::new(
                move |buffer: std::ptr::NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    // SAFETY: the buffer is alive for the handler call.
                    let buffer = unsafe { buffer.as_ref() };
                    // Read the buffer's real outcome — a device loss or
                    // cancellation completes it with `Error`, and a
                    // callback must never unwind into Objective-C. The
                    // same one-shot capsule settles on the main queue on
                    // either result; only a completed frame may return
                    // to the pool, and an error frame is reported and
                    // dropped, never pretended to have submitted.
                    let completed = buffer.status() == MTLCommandBufferStatus::Completed;
                    let error = (!completed)
                        .then(|| {
                            buffer
                                .error()
                                .map(|error| error.localizedDescription().to_string())
                        })
                        .flatten();
                    let settle = settle.lock().expect("capture lock").take();
                    if let Some(settle) = settle {
                        enqueue(move |mtm| {
                            let Settle {
                                preparation,
                                return_to,
                                completion,
                            } = settle;
                            let preparation = preparation.0;
                            if completed && let Some(capture) = return_to.get(mtm).upgrade() {
                                capture
                                    .renderer
                                    .borrow_mut()
                                    .return_frame(preparation.raster.into_frame());
                            } else if let Some(error) = error {
                                tracing::error!(
                                    error = %error,
                                    "native view composition command buffer failed"
                                );
                            }
                            // Whatever `preparation` still holds drops
                            // here on the main thread on every outcome —
                            // a failed frame's lease releases unreturned.
                            completion(completed);
                        });
                    }
                },
            );
            // SAFETY: Metal copies the block.
            unsafe {
                command_buffer.addCompletedHandler(RcBlock::as_ptr(&handler));
            }
            command_buffer.commit();
        });
    }
}

/// A view's stable identity across the capture — its address.
fn view_key(view: &PlatformView) -> usize {
    core::ptr::from_ref::<PlatformView>(view) as usize
}

/// Restores `view`'s hidden flag on drop.
struct HiddenRestore<'a> {
    owner: &'a ViewCapture,
    was: bool,
}

impl Drop for HiddenRestore<'_> {
    fn drop(&mut self) {
        self.owner.set_content_hidden(self.was);
    }
}

#[cfg(test)]
mod tests {
    use objc2::rc::Retained;
    use objc2_metal::{
        MTLCopyAllDevices, MTLCreateSystemDefaultDevice, MTLOrigin, MTLPixelFormat, MTLResource,
        MTLSize, MTLTexture,
    };

    use super::{CaptureDeferred, CompositorState, FenceBatch, NativeRenderer, SurfaceSpec};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// The lease contract, exercised behaviorally: while a frame's lease
    /// is outstanding the pool issues *different* storage — so the CPU
    /// raster can never overwrite pixels a delayed consumer still reads —
    /// a settled same-generation frame returns and is reissued, and a
    /// caller-generation bump retires the pool even on the same device.
    #[test]
    fn an_outstanding_raster_lease_is_never_reissued_across_captures_or_generations() {
        let Some(device) = MTLCreateSystemDefaultDevice() else {
            return; // No Metal on this runner — nothing to check.
        };
        let mut renderer = NativeRenderer::default();
        let first = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 0);
        // A second capture issued before `first` settles must not share
        // its storage — two overlapping consumers can never alias.
        let second = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 0);
        assert_ne!(
            Retained::as_ptr(first.texture()),
            Retained::as_ptr(second.texture()),
            "two outstanding captures must not share one raster destination"
        );
        // Once `first`'s consumer settles it goes back and is reused.
        let first_texture = Retained::as_ptr(first.texture());
        renderer.return_frame(first.into_frame());
        let reissued = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 0);
        assert_eq!(
            Retained::as_ptr(reissued.texture()),
            first_texture,
            "a settled same-generation frame returns to the pool"
        );
        // A context replacement — possibly wrapping the same physical
        // device — retires the pool: the still-outstanding `second` can
        // never answer a new generation, and a stale return is dropped
        // rather than repopulating it.
        let _next = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 1);
        renderer.return_frame(second.into_frame());
        assert!(
            renderer.available.is_empty(),
            "a superseded generation's frame must never rejoin the pool"
        );
    }

    /// Regression test for the deferred-surface defect: a batch holding
    /// both submitted and deferred surfaces reports failure — exactly
    /// once, and only after every fence has settled, so the unrendered
    /// frame never reaches composition and no in-flight submission's
    /// resources release early.
    #[test]
    fn a_batch_with_a_deferred_surface_fails_once_all_fences_settle() {
        let calls = Arc::new(AtomicUsize::new(0));
        let outcome = Arc::new(Mutex::new(None));
        let batch = FenceBatch::new(3, {
            let calls = Arc::clone(&calls);
            let outcome = Arc::clone(&outcome);
            move |result: Result<(), CaptureDeferred>| {
                calls.fetch_add(1, Ordering::Relaxed);
                *outcome.lock().expect("outcome lock") = Some(result);
            }
        });
        batch.complete_one(Ok(()));
        batch.complete_one(Err(CaptureDeferred));
        // The deferred fence does not end the batch — the third
        // submission is still in flight.
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        batch.complete_one(Ok(()));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let result = outcome.lock().expect("outcome lock").take();
        assert!(
            matches!(result, Some(Err(CaptureDeferred))),
            "a batch with a deferred surface must not report success"
        );

        // An all-submitted batch still reports success, once.
        let calls = Arc::new(AtomicUsize::new(0));
        let batch = FenceBatch::new(2, {
            let calls = Arc::clone(&calls);
            move |result: Result<(), CaptureDeferred>| {
                assert!(result.is_ok());
                calls.fetch_add(1, Ordering::Relaxed);
            }
        });
        batch.complete_one(Ok(()));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        batch.complete_one(Ok(()));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    /// The device-bound cache invariant: while the bound `MTLDevice` is
    /// the same object, every cache survives a rebind; a different device
    /// swaps the whole bundle — queue, textures, pipeline, sampler —
    /// before any cache is read. The swap half only runs where the
    /// machine exposes a second Metal device; a single-GPU runner still
    /// proves the same-device path never resets spuriously.
    #[test]
    fn device_bound_caches_reset_only_on_a_device_change() {
        let devices = MTLCopyAllDevices();
        let Some(device) = devices.iter().next() else {
            return; // No Metal on this runner — nothing to check.
        };
        let spec = SurfaceSpec {
            surface_id: 1,
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: 8,
                height: 8,
                depth: 1,
            },
            pixel_format: MTLPixelFormat::BGRA8Unorm,
        };
        let mut state = CompositorState { resources: None };

        // Populate every cache on the first device, as a live capture
        // would: the handed-out texture stands in for a submitted
        // capture's own strong ref.
        let texture;
        let pipeline;
        let sampler;
        {
            let resources = state.device_resources(&device);
            texture = resources.surface_texture(spec);
            pipeline = resources.render_pipeline(spec.pixel_format);
            sampler = resources.composite_sampler();
        }
        let queue = Retained::as_ptr(&state.device_resources(&device).command_queue);

        // The same device keeps the whole bundle.
        {
            let resources = state.device_resources(&device);
            assert_eq!(Retained::as_ptr(&resources.command_queue), queue);
            let cached = resources
                .surface_textures
                .get(&spec.surface_id)
                .expect("the bound texture must survive a same-device rebind");
            assert_eq!(Retained::as_ptr(cached), Retained::as_ptr(&texture));
            assert_eq!(
                Retained::as_ptr(
                    resources
                        .pipeline
                        .as_ref()
                        .expect("the bound pipeline must survive")
                ),
                Retained::as_ptr(&pipeline),
            );
            assert_eq!(
                Retained::as_ptr(
                    resources
                        .sampler
                        .as_ref()
                        .expect("the bound sampler must survive")
                ),
                Retained::as_ptr(&sampler),
            );
        }

        // A different device swaps the bundle before any cache is read —
        // where this runner has a second Metal device.
        for other in &devices {
            if Retained::as_ptr(&other) == Retained::as_ptr(&device) {
                continue;
            }
            let resources = state.device_resources(&other);
            assert_eq!(
                Retained::as_ptr(&resources.device),
                Retained::as_ptr(&other)
            );
            assert!(resources.surface_textures.is_empty());
            assert!(resources.pipeline.is_none());
            assert!(resources.sampler.is_none());
            let rebound = resources.surface_texture(spec);
            assert_eq!(
                Retained::as_ptr(&rebound.device()),
                Retained::as_ptr(&other),
                "rebound textures must be created on the new device",
            );

            // The texture handed out before the swap still owns its
            // original device — a submitted capture's strong refs
            // outlive the bundle replacement.
            assert_eq!(
                Retained::as_ptr(&texture.device()),
                Retained::as_ptr(&device),
                "a handed-out texture keeps its original device across a rebind",
            );
            assert_eq!(texture.width(), spec.size.width);
        }
    }
}
