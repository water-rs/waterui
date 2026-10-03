//! View-subtree capture into a Metal texture.
//!
//! [`ViewCapture`] renders a view subtree — including any GPU surfaces
//! nested inside it — into a caller-owned texture for the filter and
//! view-effect pipelines: the layer tree goes through `CARenderer`, each
//! [`CapturableSurface`] gets its own private texture, and a final pass on a
//! shared serial queue composites them under the captured overlay.
//!
//! # Orientation and scale contract
//!
//! The destination texture is top-down and sized in device pixels.
//! `CARenderer` draws bottom-up and takes its destination rect in pixels, so
//! [`with_capture_transform`] scales the layer tree for the duration of the
//! frame — without mirroring: the kit's views are already flipped, and a
//! second inversion would count the flip twice.
//!
//! # Safety
//!
//! The `unsafe` here calls `CARenderer`/`CATransaction`/`MTLCommandBuffer`
//! entry points on objects this module owns or the caller has lent it, on
//! the threads the module contract names: every `ViewCapture` method and
//! every [`CapturableSurface`] call is main-thread only; `Compositor`
//! internals run on its private serial queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use dispatch2::{DispatchQoS, DispatchQueue, GlobalQueueIdentifier, MainThreadBound};
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLClearColor, MTLCommandBuffer, MTLCommandBufferStatus,
    MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin,
    MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResource, MTLSamplerAddressMode,
    MTLSamplerDescriptor, MTLSamplerMinMagFilter, MTLSamplerState, MTLScissorRect, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};
use objc2_quartz_core::{CALayer, CARenderer, CATransaction, CATransform3D};

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
// else — exactly the confinement `CARenderer` work needs.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Send for QueueSend<T> {
    // SAFETY: the value is moved to the serial queue once and accessed nowhere
    // else — exactly the confinement `CARenderer` work needs.
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

/// Runs `body` with `layer`'s transform scaled so its point space lands on
/// the pixel destination, restored before returning.
///
/// The scaled position folds into the transform — not `layer.position` — so
/// the only property touched is one the host's layout never writes. Pending
/// transactions are flushed on both edges because `CARenderer` renders the
/// committed tree.
pub fn with_capture_transform<T>(
    layer: &CALayer,
    geometry: CaptureGeometry,
    body: impl FnOnce() -> T,
) -> T {
    flush_transaction();
    let saved_transform = layer.transform();
    let saved_position = layer.position();

    CATransaction::begin();
    CATransaction::setDisableActions(true);
    layer.setTransform(saved_transform.concat(
        CATransform3D::new_scale(geometry.scale_x, geometry.scale_y, 1.0).concat(
            CATransform3D::new_translation(
                saved_position.x * (geometry.scale_x - 1.0),
                saved_position.y * (geometry.scale_y - 1.0),
                0.0,
            ),
        ),
    ));
    CATransaction::commit();
    flush_transaction();

    let restore = TransformRestore {
        layer,
        transform: saved_transform,
    };
    let result = body();
    drop(restore);
    result
}

/// Restores a layer's transform when the capture frame ends.
struct TransformRestore<'a> {
    layer: &'a CALayer,
    transform: CATransform3D,
}

impl Drop for TransformRestore<'_> {
    fn drop(&mut self) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.layer.setTransform(self.transform);
        CATransaction::commit();
        flush_transaction();
    }
}

/// Builds the `CARenderer` creation options: the destination's colour space
/// and the Metal command queue the renderer shares.
///
/// `kCARendererColorSpace` must carry the live `CGColorSpace` object —
/// `CARenderer` type-checks its option values, and a plist-serialized
/// `CFData` makes the render crash at first use.
fn car_renderer_options(
    color_space: &objc2_core_foundation::CFRetained<objc2_core_graphics::CGColorSpace>,
    queue: &ProtocolObject<dyn MTLCommandQueue>,
) -> Retained<objc2_foundation::NSDictionary<objc2::runtime::AnyObject, objc2::runtime::AnyObject>>
{
    // SAFETY: `CARenderer`'s option keys are system statics.
    let keys: [&NSString; 2] = unsafe {
        [
            objc2_quartz_core::kCARendererColorSpace,
            objc2_quartz_core::kCARendererMetalCommandQueue,
        ]
    };
    // SAFETY: `CGColorSpace` is toll-free bridged to NSObject and every
    // Metal object descends NSObject; CARenderer expects exactly these
    // key/value pairs.
    let color_space_obj: &objc2_foundation::NSObject = unsafe {
        objc2_core_foundation::CFRetained::as_ptr(color_space)
            .cast::<objc2_foundation::NSObject>()
            .as_ref()
    };
    // SAFETY: see above.
    let queue_obj: &objc2_foundation::NSObject =
        unsafe { &*std::ptr::from_ref(queue).cast::<objc2_foundation::NSObject>() };
    let objects: [&objc2_foundation::NSObject; 2] = [color_space_obj, queue_obj];
    let options = objc2_foundation::NSDictionary::from_slices(&keys, &objects);
    // SAFETY: `NSDictionary`'s generic parameters are markers — the
    // retained elements are the same objects either way.
    unsafe {
        Retained::from_raw(Retained::into_raw(options).cast())
            .expect("a fresh dictionary is non-null")
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

/// The `CARenderer` half of a capture, confined to the main thread: one
/// command queue and one renderer per (device, pixel format) pair.
#[derive(Debug, Default)]
struct NativeRenderer {
    command_queue: Option<Retained<ProtocolObject<dyn MTLCommandQueue>>>,
    pixel_format: Option<MTLPixelFormat>,
    renderer: Option<Retained<CARenderer>>,
    overlay: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
}

impl NativeRenderer {
    /// The private overlay texture for `target` — reused when size and
    /// format match.
    ///
    /// # Panics
    ///
    /// When the device cannot allocate the texture.
    fn overlay_texture(
        &mut self,
        device: &ProtocolObject<dyn MTLDevice>,
        format: MTLPixelFormat,
        width: usize,
        height: usize,
    ) -> Retained<ProtocolObject<dyn MTLTexture>> {
        if let Some(overlay) = &self.overlay
            && overlay.width() == width
            && overlay.height() == height
            && overlay.pixelFormat() == format
            && Retained::as_ptr(&overlay.device()) == core::ptr::from_ref(device)
        {
            return overlay.clone();
        }
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                format, width, height, false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        descriptor.setStorageMode(MTLStorageMode::Private);
        let texture = device
            .newTextureWithDescriptor(&descriptor)
            .expect("failed to create the native capture overlay texture");
        self.overlay = Some(texture.clone());
        texture
    }

    /// Renders `layer` into `texture` at `geometry`'s scale, returning the
    /// uncommitted fence the caller attaches its completion to, then commits.
    ///
    /// `CARenderer` never clears, so a private destination carries stale
    /// memory: one clear pass precedes the render, on the same queue, in
    /// commit order.
    ///
    /// # Panics
    ///
    /// When a command queue, encoder or buffer cannot be created.
    fn render_layer(
        &mut self,
        layer: &CALayer,
        texture: &ProtocolObject<dyn MTLTexture>,
        geometry: CaptureGeometry,
    ) -> Retained<ProtocolObject<dyn MTLCommandBuffer>> {
        let device = texture.device();
        let stale = self.pixel_format != Some(texture.pixelFormat())
            || self
                .command_queue
                .as_ref()
                .is_none_or(|queue| Retained::as_ptr(&queue.device()) != Retained::as_ptr(&device));
        if stale {
            self.command_queue = device.newCommandQueue();
            self.pixel_format = Some(texture.pixelFormat());
            self.renderer = None;
        }
        let queue = self
            .command_queue
            .as_ref()
            .expect("failed to create the native Metal capture command queue");

        let renderer = if let Some(existing) = &self.renderer {
            let renderer = existing.clone();
            // SAFETY: `texture` is on the renderer's queue's device — the
            // (device, format) key above guarantees it.
            unsafe { renderer.setDestination(texture) };
            renderer
        } else {
            let color_space = crate::metal::color_space(texture.pixelFormat());
            let options = car_renderer_options(&color_space, queue);
            // SAFETY: `rendererWithMTLTexture:options:` retains its inputs
            // for the call; the renderer is owned through `self.renderer`.
            let renderer =
                unsafe { CARenderer::rendererWithMTLTexture_options(texture, Some(&options)) };
            self.renderer = Some(renderer.clone());
            renderer
        };

        let clear = MTLRenderPassDescriptor::new();
        // SAFETY: index 0 is the single color attachment.
        let attachment = unsafe { clear.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(texture));
        attachment.setLoadAction(MTLLoadAction::Clear);
        attachment.setStoreAction(MTLStoreAction::Store);
        attachment.setClearColor(MTLClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 0.0,
        });
        let clear_buffer = queue
            .commandBuffer()
            .expect("failed to create the clear command buffer");
        clear_buffer
            .renderCommandEncoderWithDescriptor(&clear)
            .expect("failed to encode the native capture clear pass")
            .endEncoding();
        clear_buffer.commit();

        renderer.setLayer(Some(layer));
        // The destination rect is in pixels; the transform maps the
        // point-space tree onto it.
        renderer.setBounds(
            Rect::new(
                0.0,
                0.0,
                f64::from(u32::try_from(texture.width()).unwrap_or(u32::MAX)),
                f64::from(u32::try_from(texture.height()).unwrap_or(u32::MAX)),
            )
            .into(),
        );
        with_capture_transform(layer, geometry, || {
            // SAFETY: a null timestamp is the documented default clock.
            unsafe {
                renderer.beginFrameAtTime_timeStamp(
                    objc2_quartz_core::CACurrentMediaTime(),
                    core::ptr::null_mut(),
                );
            }
            renderer.addUpdateRect(renderer.bounds());
            renderer.render();
            renderer.endFrame();
        });

        // `CARenderer` encodes onto `queue` during `render()` but offers no
        // completion, so an empty buffer committed right behind it stands in
        // as the fence — in-order execution on one queue means this
        // completes only once the capture has. Returned uncommitted: Metal
        // refuses a handler on a committed buffer.
        queue
            .commandBuffer()
            .expect("failed to create the native Metal capture command buffer")
    }
}

/// A snapshot's two halves: the platform spec and the surface it came from.
struct CapturedSnapshot {
    spec: SurfaceSpec,
    surface: Rc<dyn CapturableSurface>,
}

/// Everything [`ViewCapture::capture`] decided on the main thread that the
/// compositor's queue needs — all `Send`.
struct Preparation {
    target: Retained<ProtocolObject<dyn MTLTexture>>,
    overlay: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    device: Retained<ProtocolObject<dyn MTLDevice>>,
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
    active: RefCell<HashMap<usize, Rc<dyn CapturableSurface>>>,
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
    /// thread with whether the frame landed.
    ///
    /// # Panics
    ///
    /// When called off the main thread.
    pub fn capture(
        self: &Rc<Self>,
        target: &ProtocolObject<dyn MTLTexture>,
        completion: impl Fn(bool) + Send + 'static,
    ) {
        let mtm = objc2::MainThreadMarker::new().expect("capture runs on the main thread");
        let (preparation, fence, specs) = self.prepare(target);
        let completion = Mutex::new(Some(Box::new(completion) as Box<dyn Fn(bool) + Send>));

        if specs.is_empty() {
            // No external surfaces to compose: the handler only runs the
            // completion on the main queue. It must not own anything that
            // ever hops back to the main thread — Metal releases the block
            // on its own completion queue, and `MainThreadBound`'s drop
            // dispatches *synchronously* to the main queue. With the main
            // thread blocked on the GPU submission this completion answers
            // (it holds the device lock the submission needs), that turns
            // capture into a three-thread deadlock.
            let handler = RcBlock::new(
                move |buffer: std::ptr::NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    // SAFETY: the handler's buffer is alive for the call.
                    let buffer = unsafe { buffer.as_ref() };
                    assert!(
                        buffer.status() == MTLCommandBufferStatus::Completed,
                        "native Metal capture failed: {:?}",
                        buffer.error()
                    );
                    let completion = completion.lock().expect("capture lock").take();
                    if let Some(completion) = completion {
                        enqueue(move |_mtm| {
                            completion(true);
                        });
                    }
                },
            );
            // SAFETY: Metal copies the block for the buffer's lifetime.
            unsafe {
                fence.addCompletedHandler(RcBlock::as_ptr(&handler));
            }
            fence.commit();
            return;
        }

        // Everything leaving the main thread is `Send`: the specs are Copy,
        // the compositor is shareable, `completion` is Send — and `this`
        // rides a `MainThreadBound`, only ever upgraded on the main queue.
        let compositor = self.compositor.clone();
        let this = Mutex::new(Some(MainThreadBound::new(Rc::downgrade(self), mtm)));
        let preparation = Mutex::new(Some(QueueSend(preparation)));
        let specs = Mutex::new(Some(specs));

        let handler = RcBlock::new(
            move |buffer: std::ptr::NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                // SAFETY: the handler's buffer is alive for the call.
                let buffer = unsafe { buffer.as_ref() };
                assert!(
                    buffer.status() == MTLCommandBufferStatus::Completed,
                    "native Metal capture failed: {:?}",
                    buffer.error()
                );
                let specs = specs
                    .lock()
                    .expect("capture lock")
                    .take()
                    .expect("native capture fires once");
                let this = this.lock().expect("capture lock").take();
                let completion = completion.lock().expect("capture lock").take();
                let preparation = preparation.lock().expect("capture lock").take();
                compositor.perform(move |guard| {
                    let rendered = QueueSend(
                        guard.prepare_surface_textures(
                            &specs,
                            &preparation
                                .as_ref()
                                .expect("native capture fires once")
                                .get()
                                .device,
                        ),
                    );
                    enqueue(move |mtm| {
                        // Whole-binding move keeps the `QueueSend` wrapper —
                        // capturing `rendered.0` would capture the bare Vec.
                        let rendered = rendered;
                        // The capture pipeline outlives its owner no one: a
                        // dropped effect view drops the frame.
                        if let (Some(capture), Some(completion), Some(preparation)) = (
                            this.and_then(|this| this.get(mtm).upgrade()),
                            completion,
                            preparation.map(|preparation| preparation.0),
                        ) {
                            capture.submit_surfaces(&rendered.0, preparation, completion);
                        }
                    });
                });
            },
        );
        // SAFETY: Metal copies the block for the buffer's lifetime.
        unsafe {
            fence.addCompletedHandler(RcBlock::as_ptr(&handler));
        }
        fence.commit();
    }

    /// Ends every external surface's presentation and releases GPU state.
    /// Call before the owning view drops.
    ///
    /// # Panics
    ///
    /// It is a precondition violation to drop the capture with external
    /// surfaces still registered — `shutdown` clears them.
    pub fn shutdown(&self) {
        for surface in self.active.borrow().values() {
            surface.end_external_rendering(true);
        }
        self.active.borrow_mut().clear();
        *self.renderer.borrow_mut() = NativeRenderer::default();
        self.compositor.discard_resources();
    }

    /// Everything `capture` needs decided on the main thread, plus the
    /// uncommitted native fence.
    fn prepare(
        &self,
        target: &ProtocolObject<dyn MTLTexture>,
    ) -> (
        Preparation,
        Retained<ProtocolObject<dyn MTLCommandBuffer>>,
        Vec<SurfaceSpec>,
    ) {
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

        let native_target = if snapshots.is_empty() {
            target.retain()
        } else {
            self.renderer.borrow_mut().overlay_texture(
                &target.device(),
                target.pixelFormat(),
                target.width(),
                target.height(),
            )
        };

        let native_fence = {
            let mut renderer = self.renderer.borrow_mut();
            if snapshots.is_empty() {
                renderer.render_layer(&layer, &native_target, geometry)
            } else {
                for snapshot in &snapshots {
                    snapshot.surface.begin_capture_suppression();
                }
                let fence = renderer.render_layer(&layer, &native_target, geometry);
                for snapshot in snapshots.iter().rev() {
                    snapshot.surface.end_capture_suppression();
                }
                fence
            }
        };

        let specs: Vec<SurfaceSpec> = snapshots.iter().map(|s| s.spec).collect();
        let preparation = Preparation {
            overlay: (!snapshots.is_empty()).then_some(native_target),
            target: target.retain(),
            device: target.device(),
        };
        drop(restore);
        (preparation, native_fence, specs)
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
        let next: HashMap<usize, Rc<dyn CapturableSurface>> = snapshots
            .iter()
            .map(|s| (s.spec.surface_id, s.surface.clone()))
            .collect();
        let mut active = self.active.borrow_mut();
        for (id, surface) in active.iter() {
            if !next.contains_key(id) {
                surface.end_external_rendering(true);
            }
        }
        for (id, surface) in &next {
            if !active.contains_key(id) {
                surface.begin_external_rendering(on_redraw.clone());
            }
        }
        *active = next;
    }

    /// Final GPU half: each surface renders into its private texture, then a
    /// fence batch composites them. Main thread.
    fn submit_surfaces(
        self: &Rc<Self>,
        rendered: &[RenderedSurface],
        preparation: Preparation,
        completion: Box<dyn Fn(bool) + Send>,
    ) {
        let surfaces: Vec<Rc<dyn CapturableSurface>> = rendered
            .iter()
            .map(|item| {
                self.active
                    .borrow()
                    .get(&item.spec.surface_id)
                    .cloned()
                    .expect("a rendered surface must still be active")
            })
            .collect();
        for (item, surface) in rendered.iter().zip(&surfaces) {
            if !surface.prepare_external_render(&item.texture) {
                // Setup still pending: the frame defers.
                completion(false);
                return;
            }
        }

        let batch = Arc::new(FenceBatch::new(rendered.len(), {
            let compositor = self.compositor.clone();
            let rendered = QueueSend(rendered.to_owned());
            let preparation = QueueSend(preparation);
            move |outcome| match outcome {
                Ok(()) => Self::compose(&compositor, preparation, rendered, completion),
                // No usable pixels: the prepared frame drops with the
                // closure and the capture reports failure.
                Err(_) => completion(false),
            }
        }));
        for (item, surface) in rendered.iter().zip(&surfaces) {
            let batch = Arc::clone(&batch);
            surface.render_prepared_external_texture(
                &item.texture,
                u32::try_from(item.spec.size.width).expect("a surface is smaller than u32"),
                u32::try_from(item.spec.size.height).expect("a surface is smaller than u32"),
                Box::new(move |outcome| batch.complete_one(outcome)),
            );
        }
    }

    /// The composition pass — deliberately free of `self` so it still lands
    /// if the owning view is torn down meanwhile.
    fn compose(
        compositor: &Compositor,
        preparation: QueueSend<Preparation>,
        rendered: QueueSend<Vec<RenderedSurface>>,
        completion: Box<dyn Fn(bool) + Send>,
    ) {
        compositor.perform(move |guard| {
            let preparation = preparation.get();
            let command_buffer = guard.make_command_buffer(&preparation.device);
            guard.encode_composition(
                rendered.get(),
                preparation.overlay.as_deref(),
                &preparation.target,
                &command_buffer,
                &preparation.device,
            );
            let completion = Mutex::new(Some(completion));
            let handler = RcBlock::new(
                move |buffer: std::ptr::NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    // SAFETY: the buffer is alive for the handler call.
                    let buffer = unsafe { buffer.as_ref() };
                    assert!(
                        buffer.status() == MTLCommandBufferStatus::Completed,
                        "Metal view composition failed: {:?}",
                        buffer.error()
                    );
                    let completion = completion.lock().expect("capture lock").take();
                    if let Some(completion) = completion {
                        enqueue(move |_mtm| {
                            completion(true);
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
    use core::ffi::c_void;

    use objc2::rc::Retained;
    use objc2_core_foundation::{CFRetained, ConcreteType};
    use objc2_core_graphics::CGColorSpace;
    use objc2_foundation::NSString;
    use objc2_metal::{
        MTLCopyAllDevices, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLPixelFormat,
        MTLResource, MTLSize, MTLTexture,
    };

    use super::{CaptureDeferred, CompositorState, FenceBatch, SurfaceSpec, car_renderer_options};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // `CFGetTypeID` distinguishes a live Core Foundation object from any
    // serialization of one. Declared here because `objc2-core-foundation`
    // keeps the symbol private.
    unsafe extern "C" {
        fn CFGetTypeID(object: *const c_void) -> usize;
    }

    /// Regression test for the colour-space defect: `kCARendererColorSpace`
    /// must carry the live `CGColorSpace`, not a `CFData` serialization of
    /// it — `CARenderer` type-checks its options and crashed on the data
    /// form.
    #[test]
    fn the_car_renderer_options_carry_a_live_color_space() {
        let Some(device) = MTLCreateSystemDefaultDevice() else {
            return; // No Metal on this runner — nothing to check.
        };
        let queue = device
            .newCommandQueue()
            .expect("failed to create a Metal command queue");
        let color_space: CFRetained<CGColorSpace> =
            crate::metal::color_space(MTLPixelFormat::BGRA8Unorm);
        let options = car_renderer_options(&color_space, &queue);

        // SAFETY: the option key is a system static.
        let key: &NSString = unsafe { objc2_quartz_core::kCARendererColorSpace };
        // SAFETY: the static's storage is `NSString`, a subclass of
        // `NSObject`, so the pointer re-interpretation stays in bounds.
        let key: &objc2::runtime::AnyObject =
            unsafe { &*std::ptr::from_ref::<NSString>(key).cast() };
        let value = options
            .objectForKey(key)
            .expect("the options must set kCARendererColorSpace");
        // SAFETY: `value` is a live `NSObject` — a valid `CFTypeRef`.
        let type_id = unsafe {
            CFGetTypeID(
                Retained::as_ptr(&value)
                    .cast::<objc2::runtime::AnyObject>()
                    .cast(),
            )
        };
        assert_eq!(
            type_id,
            CGColorSpace::type_id(),
            "kCARendererColorSpace must carry a CGColorSpace, not a serialization",
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
