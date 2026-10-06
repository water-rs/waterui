//! View-subtree capture into a Metal texture.
//!
//! [`ViewCapture`] renders a view subtree — including any GPU surfaces
//! nested inside it — into a caller-owned texture for the filter and
//! view-effect pipelines: the layer tree is rasterized synchronously by
//! `CALayer.renderInContext` into a `CGContext` whose backing store is a
//! shared `MTLBuffer`, a Metal blit transfer into a private 2D texture
//! lets the compositor sample the native pixels with no CPU readback,
//! each
//! [`CapturableSurface`] gets its own private texture, and a final pass on
//! a shared serial queue composites them under the captured overlay. The
//! raster itself is still CPU work — the transfer is a GPU blit, not
//! the drawing.
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
use objc2_core_foundation::{CFRetained, CGAffineTransform};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo, CGImageComponentInfo,
};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor,
    MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue, MTLDevice,
    MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType,
    MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLResource, MTLResourceOptions, MTLSamplerAddressMode,
    MTLSamplerDescriptor, MTLSamplerMinMagFilter, MTLSamplerState, MTLScissorRect, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
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
    /// The captured view's bounds in its own points — the space the
    /// raster maps. Its height is the span a bottom-up source flips
    /// around.
    pub source: Rect,
    /// Horizontal point-to-pixel scale.
    pub scale_x: f64,
    /// Vertical point-to-pixel scale.
    pub scale_y: f64,
    /// Whether the source's y axis grows downward: `UIKit`'s top-left
    /// origin and `AppKit`'s flipped views are y-down; a plain `NSView`
    /// measures its min y from the bottom.
    pub y_down: bool,
}

impl CaptureGeometry {
    /// The geometry mapping `bounds` (non-empty, in points) onto a
    /// `width` × `height` pixel destination, `y_down` naming whether the
    /// source's y axis already points down the destination.
    ///
    /// # Panics
    ///
    /// When `bounds` is empty.
    #[must_use]
    pub fn new(bounds: Rect, width: usize, height: usize, y_down: bool) -> Self {
        assert!(
            bounds.size.width > 0.0 && bounds.size.height > 0.0,
            "capture content must have non-zero bounds"
        );
        #[expect(
            clippy::cast_precision_loss,
            reason = "a capture texture is at most a few thousand pixels on a side"
        )]
        Self {
            source: bounds,
            scale_x: width as f64 / bounds.size.width,
            scale_y: height as f64 / bounds.size.height,
            y_down,
        }
    }
}

/// Where a GPU surface's own texture lands inside a capture.
///
/// `full_size` is the producer texture's pixel size — the surface's whole
/// mapped rect. `clip` is the visible destination rect inside the capture
/// target, and the `uv_*` pair selects the window of the producer texture
/// that visible rect samples: `uv = uv_origin + unit * uv_scale` over the
/// clipped region. Clipping therefore crops the destination and the UV
/// window without ever resizing or re-laying out the producer.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSpec {
    /// The capture's identity for the surface — the surface view's address.
    pub surface_id: usize,
    /// The producer texture's pixel size — the surface's full mapped rect.
    /// Allocates the private texture and is the size
    /// [`CapturableSurface::render_prepared_external_texture`] renders at.
    pub full_size: MTLSize,
    /// The visible destination rect inside the capture target — the
    /// composition pass's viewport *and* scissor: non-negative and inside
    /// the target by construction, so no Metal viewport bound is exercised.
    pub clip: MTLScissorRect,
    /// Origin of the window the visible rect samples in the producer
    /// texture.
    pub uv_origin: [f32; 2],
    /// Size of the sampled window — see `uv_origin`.
    pub uv_scale: [f32; 2],
    /// The format the surface renders at.
    pub pixel_format: MTLPixelFormat,
}

/// The UV window handed to the composite vertex shader — mirrors
/// `CaptureCompositeRegion` in `capture_composite.metal`. `float2` fields
/// are `f32` pairs at matching offsets, so `repr(C)` preserves the Metal
/// ABI.
#[repr(C)]
struct CompositeRegion {
    uv_origin: [f32; 2],
    uv_scale: [f32; 2],
}

/// The window the native overlay draws through — the whole texture.
const IDENTITY_UV: CompositeRegion = CompositeRegion {
    uv_origin: [0.0, 0.0],
    uv_scale: [1.0, 1.0],
};

/// A pixel extent as the `f64` Metal viewports and bounds take.
fn pixel_extent(value: usize) -> f64 {
    f64::from(u32::try_from(value).expect("a pixel extent fits in u32"))
}

/// The spec `bounds` (already in the capture's content space) maps to in a
/// `width` × `height` pixel destination.
///
/// The full rect is mapped through the source's origin and y convention
/// *before* any clipping — endpoints quantize independently (floor the low
/// edge, ceil the high edge) so the producer texture keeps every texel its
/// content touches. Only the visible destination rect is clipped against
/// the target; a surface partially outside keeps its full producer size
/// and crops through `uv_origin`/`uv_scale` instead. `None` when the rect
/// is empty or lands entirely outside.
#[must_use]
pub fn surface_spec(
    surface_id: usize,
    bounds: Rect,
    geometry: CaptureGeometry,
    pixel_format: MTLPixelFormat,
    target_width: usize,
    target_height: usize,
) -> Option<SurfaceSpec> {
    let source_min_x = geometry.source.origin.x;
    let source_min_y = geometry.source.origin.y;
    let source_max_y = source_min_y + geometry.source.size.height;
    let child_min_x = bounds.origin.x;
    let child_min_y = bounds.origin.y;
    let child_max_x = child_min_x + bounds.size.width;
    let child_max_y = child_min_y + bounds.size.height;

    // An empty rect touches no texels — quantizing its zero-area span
    // would still produce a 1-pixel producer for nothing.
    if !(bounds.size.width > 0.0 && bounds.size.height > 0.0) {
        return None;
    }

    let low_x = (child_min_x - source_min_x) * geometry.scale_x;
    let high_x = (child_max_x - source_min_x) * geometry.scale_x;
    let (low_y, high_y) = if geometry.y_down {
        (
            (child_min_y - source_min_y) * geometry.scale_y,
            (child_max_y - source_min_y) * geometry.scale_y,
        )
    } else {
        // A bottom-up source measures `min_y` from the bottom: the child's
        // top edge `child_max_y` lands on the destination's upper rows.
        (
            (source_max_y - child_max_y) * geometry.scale_y,
            (source_max_y - child_min_y) * geometry.scale_y,
        )
    };

    // The mapped full rect: `floor` the low edge, `ceil` the high edge so
    // every touched texel survives — independent endpoints, not a floored
    // origin plus a ceiled size.
    let full_left = low_x.floor();
    let full_top = low_y.floor();
    let full_w = high_x.ceil() - full_left;
    let full_h = high_y.ceil() - full_top;

    // Only the destination region clips against the target.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a capture texture is at most a few thousand pixels on a side"
    )]
    let (target_width, target_height) = (target_width as f64, target_height as f64);
    let clip_x = full_left.max(0.0);
    let clip_y = full_top.max(0.0);
    let clip_w = (full_left + full_w).min(target_width) - clip_x;
    let clip_h = (full_top + full_h).min(target_height) - clip_y;
    if !(full_w > 0.0 && full_h > 0.0 && clip_w > 0.0 && clip_h > 0.0) {
        return None;
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "UV fractions stay inside [0, 1] — f32 precision is ample"
    )]
    let (uv_origin, uv_scale) = (
        [
            ((clip_x - full_left) / full_w) as f32,
            ((clip_y - full_top) / full_h) as f32,
        ],
        [(clip_w / full_w) as f32, (clip_h / full_h) as f32],
    );
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "full extents are positive and clip extents are bounded by the non-negative target"
    )]
    let (full_width, full_height, clip_x, clip_y, clip_w, clip_h) = (
        full_w as usize,
        full_h as usize,
        clip_x as usize,
        clip_y as usize,
        clip_w as usize,
        clip_h as usize,
    );
    Some(SurfaceSpec {
        surface_id,
        full_size: MTLSize {
            width: full_width,
            height: full_height,
            depth: 1,
        },
        clip: MTLScissorRect {
            x: clip_x,
            y: clip_y,
            width: clip_w,
            height: clip_h,
        },
        uv_origin,
        uv_scale,
        pixel_format,
    })
}

/// The affine transform the native raster draws a layer-space point
/// through: the destination's texel (0,0) holds the content at the
/// source's bounds origin, each axis scales by the capture's pixel ratio,
/// and a y-down source mirrors about the visible window so the raster's
/// bottom-up draw lands top-down.
fn raster_transform(geometry: CaptureGeometry, height: f64) -> CGAffineTransform {
    let sx = geometry.scale_x;
    let sy = geometry.scale_y;
    let origin = geometry.source.origin;
    let (d, ty) = if geometry.y_down {
        // The mirrored draw maps the visible window's layer-space top
        // edge onto destination row 0: the bounds height enters the
        // destination height in unscaled space.
        (-sy, origin.y.mul_add(sy, height))
    } else {
        (sy, -origin.y * sy)
    };
    CGAffineTransform {
        a: sx,
        b: 0.0,
        c: 0.0,
        d,
        tx: -origin.x * sx,
        ty,
    }
}

/// The signal that a prepared surface frame produced no usable pixels:
/// its texture must not be sampled.
///
/// [`Deferred`](CaptureError::Deferred) is the retryable outcome — the
/// frame was never submitted (a lost context between preparation and
/// submission, a stale lease, a parked wait) or its in-flight submission
/// was lost to device failure; the surface's redraw contract replays it
/// on the next publication. [`Failed`](CaptureError::Failed) is terminal
/// on this context: it carries the surface's typed failure, or a
/// [`CompositionFailed`] when the native-view composition command buffer
/// ended in any status other than `Completed` — a later redraw on the same
/// generation cannot produce the frame, so a capture must stop instead of
/// waiting forever. Every fence in the batch still
/// settles, and nothing composes the missing frame; fatal programming
/// errors still fail fast rather than reporting through this outcome.
#[derive(Debug)]
pub enum CaptureError {
    /// No usable pixels this attempt — the capture retries on the
    /// surface's next redraw wake.
    Deferred,
    /// The surface settled a typed failure, or the native-view composition
    /// did not complete ([`CompositionFailed`]) — the frame can never land
    /// on this context generation.
    Failed(Arc<dyn std::error::Error + Send + Sync + 'static>),
}

/// The callback a surface render request answers with — run on the main
/// thread, exactly once per accepted request.
///
/// `Ok(())` means the frame's texture carries usable pixels;
/// `Err(CaptureError)` says why it produced none.
pub type SurfaceCaptureCompletion = Box<dyn FnOnce(Result<(), CaptureError>) + Send>;

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
/// settles. Completed on the main thread.
///
/// The last fence answers `Err(CaptureError::Failed)` when a fence
/// settled a terminal failure, `Err(CaptureError::Deferred)` when one
/// reported only a retryable miss.
pub struct FenceBatch {
    remaining: AtomicUsize,
    failed: AtomicBool,
    /// The first terminal failure a fence reported — it wins over a
    /// deferred outcome, because a settled failure is never recovered by
    /// a retry this generation.
    terminal: Mutex<Option<Arc<dyn std::error::Error + Send + Sync>>>,
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
        completion: impl FnOnce(Result<(), CaptureError>) + Send + 'static,
    ) -> Self {
        assert!(
            count > 0,
            "a GPU fence batch must contain at least one submission"
        );
        Self {
            remaining: AtomicUsize::new(count),
            failed: AtomicBool::new(false),
            terminal: Mutex::new(None),
            completion: Mutex::new(Some(Box::new(completion))),
        }
    }

    /// One fence settled: `Ok` when its frame's pixels are usable,
    /// `Err(CaptureError)` when the surface never submitted it — the
    /// terminal kind is retained so the batch answers the first
    /// `Failed`, not the last fence's outcome. Either way the batch waits
    /// on every outstanding fence — a deferred frame never releases the
    /// submissions still in flight.
    ///
    /// # Panics
    ///
    /// When more fences land than the batch was built with.
    pub fn complete_one(&self, outcome: Result<(), CaptureError>) {
        match outcome {
            Ok(()) => {}
            Err(CaptureError::Deferred) => {
                self.failed.store(true, Ordering::Relaxed);
            }
            Err(CaptureError::Failed(error)) => {
                self.failed.store(true, Ordering::Relaxed);
                let mut terminal = self.terminal.lock().expect("fence batch error lock");
                if terminal.is_none() {
                    *terminal = Some(error);
                }
            }
        }
        // The store above is ordered before this release decrement, so the
        // fence that observes `remaining == 1` sees every reported failure.
        let remaining = self.remaining.fetch_sub(1, Ordering::AcqRel);
        assert!(remaining > 0, "a GPU fence batch completed more than once");
        if remaining == 1
            && let Some(completion) = self.completion.lock().expect("fence batch lock").take()
        {
            let terminal = self.terminal.lock().expect("fence batch error lock").take();
            completion(match terminal {
                Some(error) => Err(CaptureError::Failed(error)),
                None if self.failed.load(Ordering::Relaxed) => Err(CaptureError::Deferred),
                None => Ok(()),
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
            let matches = texture.width() == spec.full_size.width
                && texture.height() == spec.full_size.height
                && texture.pixelFormat() == spec.pixel_format;
            if matches {
                return texture.clone();
            }
        }
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                spec.pixel_format,
                spec.full_size.width,
                spec.full_size.height,
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
                originX: pixel_extent(spec.clip.x),
                originY: pixel_extent(spec.clip.y),
                width: pixel_extent(spec.clip.width),
                height: pixel_extent(spec.clip.height),
                znear: 0.0,
                zfar: 1.0,
            });
            encoder.setScissorRect(spec.clip);
            let region = CompositeRegion {
                uv_origin: spec.uv_origin,
                uv_scale: spec.uv_scale,
            };
            // SAFETY: `encoder` is a live render encoder, index 0 is the
            // vertex-constant and texture slots the shader binds, and
            // `setVertexBytes` copies `region`'s bytes into the command.
            unsafe {
                encoder.setVertexBytes_length_atIndex(
                    std::ptr::NonNull::from(&region).cast::<core::ffi::c_void>(),
                    core::mem::size_of::<CompositeRegion>(),
                    0,
                );
                encoder.setFragmentTexture_atIndex(Some(&surface.texture), 0);
                encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
            }
        }
        if let Some(overlay) = overlay {
            encoder.setViewport(MTLViewport {
                originX: 0.0,
                originY: 0.0,
                width: pixel_extent(target.width()),
                height: pixel_extent(target.height()),
                znear: 0.0,
                zfar: 1.0,
            });
            encoder.setScissorRect(MTLScissorRect {
                x: 0,
                y: 0,
                width: target.width(),
                height: target.height(),
            });
            // SAFETY: same encoder/slot contract as above; the overlay
            // samples its whole texture — identity window.
            unsafe {
                encoder.setVertexBytes_length_atIndex(
                    std::ptr::NonNull::from(&IDENTITY_UV).cast::<core::ffi::c_void>(),
                    core::mem::size_of::<CompositeRegion>(),
                    0,
                );
                encoder.setFragmentTexture_atIndex(Some(overlay), 0);
                encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
            }
        }
        encoder.endEncoding();
    }
}

/// One native raster destination: a shared `MTLBuffer` the `CGContext`
/// draws into, and a private 2D texture a Metal blit transfers the
/// pixels into — the compositor only ever samples the private texture,
/// so native pixels reach the composite pass with no CPU readback.
///
/// The simulator forbids render-target buffer-backed textures and
/// requires private storage for them; a separate private texture is the
/// one layout Apple documents for every device and simulator — Apple's
/// `developing-metal-apps-that-run-in-simulator` texture limitations
/// and `copying-data-to-a-private-resource` are the contract this
/// follows.
///
/// A frame is bound to its exact generation and geometry: the caller's
/// context-generation token, the capture device, the destination pixel
/// format, and the pixel size. Any change to those rebuilds it, because
/// the buffer stride, the context's bitmap layout, and the texture are
/// baked at creation — and because storage issued under one context
/// generation is invalid for another even when the device object is
/// identical.
#[derive(Debug)]
struct NativeRasterFrame {
    /// The capture device — retained so the buffer and texture outlive a
    /// teardown; device identity alone is NOT the generation key.
    _device: Retained<ProtocolObject<dyn MTLDevice>>,
    /// The shared pixel storage the context draws into — owned here so
    /// the context's target memory stays valid for the frame's lifetime,
    /// and the blit source on the composition command buffer.
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    /// The private destination texture the compositor samples.
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
    /// The padded row stride the buffer and the blit source layout share.
    row_bytes: usize,
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
        // Sample-only, private storage — the readback texture is written by
        // the capture's GPU pass and only ever sampled, so private storage
        // with no render-target usage is correct on every Apple device.
        descriptor.setStorageMode(MTLStorageMode::Private);
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        let texture = device
            .newTextureWithDescriptor(&descriptor)
            .expect("failed to create the private native capture texture");
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
            buffer,
            texture,
            context,
            generation,
            pixel_width,
            pixel_height,
            pixel_format,
            row_bytes,
        }
    }

    /// Encodes the shared-buffer → private-texture transfer on the
    /// command buffer the composite pass is encoded on, before the
    /// render encoder — the GPU blit that makes the CPU-drawn pixels
    /// samplable, per Apple's private-resource copy contract.
    ///
    /// # Panics
    ///
    /// When the blit encoder cannot be created.
    fn encode_transfer(&self, command_buffer: &ProtocolObject<dyn MTLCommandBuffer>) {
        let encoder = command_buffer
            .blitCommandEncoder()
            .expect("failed to create the native capture transfer encoder");
        // SAFETY: the frame owns `buffer` and `texture` past command
        // completion (the raster lease is held by the completed handler);
        // offset 0 with the buffer's actual padded stride covers the
        // whole image, one level, one slice.
        unsafe {
            encoder.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                &self.buffer,
                0,
                self.row_bytes,
                self.row_bytes * self.pixel_height,
                MTLSize {
                    width: self.pixel_width,
                    height: self.pixel_height,
                    depth: 1,
                },
                &self.texture,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
            );
        }
        encoder.endEncoding();
    }

    /// Rasterizes `layer`'s tree into the shared buffer at `geometry`'s
    /// scale — a synchronous CPU draw, completed when it returns.
    ///
    /// Geometry is expressed through the context's CTM alone: the live
    /// layer tree is never transformed or reparented. The CTM is applied
    /// after clearing and restored before returning, so a reused context
    /// carries no state across frames. `raster_transform` also anchors
    /// the layer's bounds origin to texel (0,0) — content at the bounds
    /// origin lands on the texture's first row and column — so a layer
    /// whose bounds start at a non-zero origin still fills the buffer
    /// from the corner. The output obeys the composite pass's contract —
    /// texel row 0 is the view's top edge: a bitmap context is
    /// bottom-left-origin like `AppKit`'s layer space, so a bottom-up
    /// source's plain-scale draw already lands each layer row on its
    /// matching texel row; a y-down source's draw must be mirrored
    /// about the destination midline inside the same transform —
    /// `renderInContext` draws the committed layer model in the layer's
    /// own space and does not apply a flipped view's geometry-flip on
    /// macOS, matching `UIKit`'s top-left-origin layer space, which is
    /// why both flip the same way.
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
        CGContext::concat_ctm(Some(context), raster_transform(geometry, height));
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
    /// The private texture the compositor samples — valid only after
    /// `encode_transfer` has run on the same command buffer.
    fn texture(&self) -> &Retained<ProtocolObject<dyn MTLTexture>> {
        &self.frame.as_ref().expect("a lease owns its frame").texture
    }

    /// Encodes the blit transfer into the composition command buffer —
    /// see [`NativeRasterFrame::encode_transfer`].
    fn encode_transfer(&self, command_buffer: &ProtocolObject<dyn MTLCommandBuffer>) {
        self.frame
            .as_ref()
            .expect("a lease owns its frame")
            .encode_transfer(command_buffer);
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
    /// pool. A frame settling after the pool was reset (shutdown) is
    /// dropped without re-arming the pool — an outstanding capture must
    /// never resurrect lifetime the owner already ended.
    fn return_frame(&mut self, frame: NativeRasterFrame) {
        let Some(key) = &self.key else {
            return;
        };
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

/// The one outer capture transaction, closed on every exit: `commit`
/// runs explicitly as the pass's sole commit; `Drop` closes it when the
/// pass exits early — suppression is restored (by [`SuppressionGuard`],
/// declared after it) before that close so no suppressed state can be
/// published. `CATransaction` has no abort: committing a restored-state
/// transaction is the only way to end it without leaking an open
/// transaction onto the thread's stack.
struct TransactionGuard {
    committed: bool,
}

impl TransactionGuard {
    /// Opens the capture transaction with actions disabled.
    fn begin() -> Self {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        Self { committed: false }
    }

    /// Performs the pass's sole commit.
    fn commit(mut self) {
        self.committed = true;
        CATransaction::commit();
    }
}

impl Drop for TransactionGuard {
    fn drop(&mut self) {
        if !self.committed {
            CATransaction::commit();
        }
    }
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
    completion: Box<dyn Fn(Result<(), CaptureError>) + Send>,
}

/// A view's capturable-surface slot: the mounted leaf installs its
/// surface once and unmount clears it.
///
/// Weak — the leaf retains both the surface and the view, so a strong
/// slot would keep the pair alive forever. One type serves every view
/// class the capture walk visits.
#[derive(Default)]
pub struct CapturableSlot(RefCell<Option<Weak<dyn CapturableSurface>>>);

impl CapturableSlot {
    /// The capturable surface installed on the slot, if any.
    #[must_use]
    pub fn get(&self) -> Option<Rc<dyn CapturableSurface>> {
        self.0.borrow().as_ref().and_then(Weak::upgrade)
    }

    /// Installs the leaf's capturable surface.
    ///
    /// # Panics
    ///
    /// When the slot is already occupied — a leaf installs its
    /// capturable exactly once between clears.
    pub fn install(&self, surface: &Rc<dyn CapturableSurface>) {
        let mut slot = self.0.borrow_mut();
        assert!(
            slot.is_none(),
            "a capturable surface is already installed on this view"
        );
        *slot = Some(Rc::downgrade(surface));
    }

    /// Removes the installed capturable surface.
    pub fn clear(&self) {
        *self.0.borrow_mut() = None;
    }
}

impl fmt::Debug for CapturableSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapturableSlot")
            .field("occupied", &self.0.borrow().is_some())
            .finish()
    }
}

/// The capturable surface `view` itself presents, if any.
///
/// A mounted leaf stores it on its own view — GPU surfaces on their
/// [`SurfaceView`], filtered outputs on their [`HostView`] — and the
/// capture walk finds it by downcasting the views it visits.
///
/// [`SurfaceView`]: crate::appkit::surface_view::SurfaceView
/// [`HostView`]: crate::appkit::HostView
#[must_use]
pub fn resolve_capturable(view: &PlatformView) -> Option<Rc<dyn CapturableSurface>> {
    #[cfg(target_os = "macos")]
    {
        use crate::appkit::{HostView, surface_view::SurfaceView};
        if let Some(view) = view.downcast_ref::<SurfaceView>() {
            return view.capturable();
        }
        if let Some(view) = view.downcast_ref::<HostView>() {
            return view.capturable();
        }
    }
    #[cfg(target_os = "ios")]
    {
        use crate::uikit::{HostView, surface_view::SurfaceView};
        if let Some(view) = view.downcast_ref::<SurfaceView>() {
            return view.capturable();
        }
        if let Some(view) = view.downcast_ref::<HostView>() {
            return view.capturable();
        }
    }
    let _ = view;
    None
}

/// The one subtree walk the capture paths share: `visit` receives each
/// resolved capturable surface and the view carrying it; a resolved
/// view stops the descent — its own subtree renders through its
/// producer, and its readiness covers them.
fn walk_capturables(
    view: &PlatformView,
    visit: &mut impl FnMut(&PlatformView, &Rc<dyn CapturableSurface>),
) {
    if let Some(surface) = resolve_capturable(view) {
        visit(view, &surface);
        return;
    }
    for subview in crate::view::subviews(view) {
        walk_capturables(&subview, visit);
    }
}

/// Every capturable surface inside `root`'s subtree — `f` receives each
/// live surface.
pub fn collect_capturables(root: &PlatformView, f: &mut impl FnMut(&Rc<dyn CapturableSurface>)) {
    walk_capturables(root, &mut |_, surface| f(surface));
}

/// Captures `content`'s subtree into Metal textures. Main-thread only;
/// created once per effect view and shut down before the view drops.
pub struct ViewCapture {
    content: Retained<PlatformView>,
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
    /// A capture for `content`. Capturable GPU surfaces resolve through
    /// [`resolve_capturable`] — the slot each mounted leaf stores on its
    /// own view.
    #[must_use]
    pub fn new(_mtm: objc2::MainThreadMarker, content: Retained<PlatformView>) -> Self {
        Self {
            content,
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
        completion: impl Fn(Result<(), CaptureError>) + Send + 'static,
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
        let mut completion =
            Some(Box::new(completion) as Box<dyn Fn(Result<(), CaptureError>) + Send>);

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
                    // The owner is gone: teardown mid-capture is a
                    // deferral, never a silent drop.
                    None => completion(Err(CaptureError::Deferred)),
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

        // INVARIANT: every temporary mutation of the capture — the
        // reveal of a normally-hidden root, surface suppression, and
        // their restoration — happens inside ONE outer disabled-actions
        // `CATransaction`, and the sole commit runs only after every
        // original state is back. The ordering inside is fixed:
        // reveal → synchronous layout/display preparation → geometry
        // and snapshot collection — of the POST-layout tree: a pending
        // layout can resize bounds or mount a capturable child, so
        // nothing may be collected from the pre-layout tree → raster
        // allocation (still before any suppression mutation) →
        // suppression open → draw → suppression restore → re-hide →
        // sole commit. A normally-hidden filter-owned root is un-hidden
        // in-model, drawn, and re-hidden before that commit: the render
        // server never sees the revealed source tree. Nothing inside
        // may open, commit or flush a transaction of its own: a
        // mid-pass commit would publish the revealed or suppressed
        // state and flicker — or leak — the on-screen tree. The guards
        // restore exactly what they opened — synchronously before the
        // sole commit on success, from Drop on an early exit — and the
        // transaction closes last, so an early exit commits only the
        // already-restored model state. The layer tree itself is never
        // transformed or reparented — the destination geometry lives in
        // the context's CTM.
        let (preparation, specs) = {
            // Declared in drop order: `suppression` ends first, then
            // `restore` re-hides, then `transaction` closes — the
            // commit only ever sees the original state.
            let transaction = TransactionGuard::begin();
            let restore = HiddenRestore {
                owner: self,
                was: was_hidden,
            };
            if was_hidden {
                crate::view::set_hidden(content, false);
            }
            // Layout/display preparation is a synchronous model-side
            // pass — it never publishes; it must run while the subtree
            // is un-hidden or a hidden view may have deferred it, and
            // everything collected after it sees the laid-out tree.
            crate::view::prepare_for_capture(content);
            let layer = crate::view::layer(content).expect("a capture view must be layer-backed");
            let geometry = CaptureGeometry::new(
                crate::view::bounds(content),
                target.width(),
                target.height(),
                crate::view::is_flipped(content),
            );
            let snapshots = self.collect_snapshots(target, geometry);
            self.update_external_surfaces(&snapshots);
            // The registrations this capture owns — the immutable
            // snapshot of who must stay external until its frame
            // settles.
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
            // Fallible allocation still precedes suppression: an
            // allocation panic leaves suppression untouched and the
            // guards still restore what they opened.
            let raster = self.renderer.borrow_mut().issue(
                &target.device(),
                target.pixelFormat(),
                target.width(),
                target.height(),
                generation,
            );
            let mut suppression = SuppressionGuard::new(&snapshots);
            suppression.begin();
            raster.draw(&layer, geometry);
            suppression.end();
            let specs: Vec<SurfaceSpec> = snapshots.iter().map(|s| s.spec).collect();
            let preparation = Preparation {
                raster,
                target: target.retain(),
                device: target.device(),
                surfaces,
            };
            // Re-hide before the sole commit — `restore` would do it on
            // drop, but the restore must be explicit before commit.
            drop(restore);
            transaction.commit();
            flush_transaction();
            (preparation, specs)
        };
        (preparation, specs)
    }

    /// Restores the captured content's hidden state — a plain model
    /// mutation. The caller's transaction decides when it is published;
    /// in the capture pass that is the sole outer commit, after the
    //  raster has already read the revealed tree.
    fn set_content_hidden(&self, hidden: bool) {
        if crate::view::is_hidden(&self.content) == hidden {
            return;
        }
        crate::view::set_hidden(&self.content, hidden);
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

    /// Recursion over `view`'s subviews through [`walk_capturables`]:
    /// a resolved surface snapshots and stops the descent.
    fn collect_into(
        &self,
        view: &PlatformView,
        snapshots: &mut Vec<CapturedSnapshot>,
        geometry: CaptureGeometry,
        target_width: usize,
        target_height: usize,
    ) {
        walk_capturables(view, &mut |view, surface| {
            let bounds = surface.content_bounds(&self.content);
            if let Some(spec) = surface_spec(
                view_key(view),
                bounds,
                geometry,
                surface.capture_pixel_format(),
                target_width,
                target_height,
            ) {
                snapshots.push(CapturedSnapshot {
                    spec,
                    surface: surface.clone(),
                });
            }
        });
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
        // One pass over the snapshot: the next registration set is built
        // keyed by surface id — surviving surfaces keep their `Rc` lease,
        // parted ones drop out (their registration ends when the last
        // owner, map or outstanding capture, drops on this thread), and
        // each new surface is begun exactly once. No per-surface scan of
        // the snapshot, so nested-GPU-child scenes stay linear.
        let next: HashMap<usize, Rc<SurfaceRegistration>> = snapshots
            .iter()
            .map(|s| {
                let id = s.spec.surface_id;
                let registration = active.get(&id).cloned().unwrap_or_else(|| {
                    s.surface.begin_external_rendering(on_redraw.clone());
                    Rc::new(SurfaceRegistration {
                        surface: s.surface.clone(),
                    })
                });
                (id, registration)
            })
            .collect();
        *active = next;
    }

    /// Final GPU half: each surface renders into its private texture, then a
    /// fence batch composites them. Main thread.
    fn submit_surfaces(
        self: &Rc<Self>,
        rendered: &[RenderedSurface],
        preparation: Preparation,
        completion: Box<dyn Fn(Result<(), CaptureError>) + Send>,
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
                completion(Err(CaptureError::Deferred));
                return;
            }
        }

        let batch = Arc::new(FenceBatch::new(rendered.len(), {
            let compositor = self.compositor.clone();
            let rendered = QueueSend(rendered.to_owned());
            let preparation = QueueSend(preparation);
            move |outcome| {
                match outcome {
                    Ok(()) => {
                        Self::compose(&compositor, preparation, rendered, completion, return_to);
                    }
                    Err(error) => {
                        // No usable pixels: the frame never submitted —
                        // release its lease unreturned. Destruction AND the
                        // failure report both happen on the main queue, in
                        // that order: the completion contract is main-thread,
                        // and the caller settles only after the leased
                        // cleanup it owns has run. A terminal `Failed`
                        // forwards as-is — a settled surface failure does
                        // not retry.
                        enqueue(move |_| {
                            drop(preparation);
                            completion(Err(error));
                        });
                    }
                }
            }
        }));
        for (item, registration) in rendered.iter().zip(&surfaces) {
            let batch = Arc::clone(&batch);
            registration.surface.render_prepared_external_texture(
                &item.texture,
                u32::try_from(item.spec.full_size.width).expect("a surface is smaller than u32"),
                u32::try_from(item.spec.full_size.height).expect("a surface is smaller than u32"),
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
        completion: Box<dyn Fn(Result<(), CaptureError>) + Send>,
        return_to: MainThreadBound<Weak<Self>>,
    ) {
        compositor.perform(move |guard| {
            let command_buffer = guard.make_command_buffer(&preparation.get().device);
            // The CPU-drawn raster reaches the compositor's private
            // texture through a blit on this same command buffer — the
            // encoder is ended before the render pass samples it.
            preparation.get().raster.encode_transfer(&command_buffer);
            guard.encode_composition(
                rendered.get(),
                Some(preparation.get().raster.texture()),
                &preparation.get().target,
                &command_buffer,
                &preparation.get().device,
            );
            // The lease is held until this command buffer completes:
            // only then has the GPU stopped sampling the private texture
            // the blit filled from the shared buffer.
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
                    // Status is the authoritative outcome; the `NSError`
                    // payload is optional and rides along when present.
                    let status = buffer.status();
                    let error = buffer
                        .error()
                        .map(|error| error.localizedDescription().to_string());
                    let settle = settle.lock().expect("capture lock").take();
                    if let Some(settle) = settle {
                        enqueue(move |mtm| {
                            let Settle {
                                preparation,
                                return_to,
                                completion,
                            } = settle;
                            let preparation = preparation.0;
                            let outcome = composition_outcome(status, error);
                            if outcome.is_ok()
                                && let Some(capture) = return_to.get(mtm).upgrade()
                            {
                                capture
                                    .renderer
                                    .borrow_mut()
                                    .return_frame(preparation.raster.into_frame());
                            }
                            // Whatever `preparation` still holds drops
                            // here on the main thread on every outcome —
                            // a failed frame's lease releases unreturned.
                            completion(outcome);
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

/// A composition command buffer that ended without completing: the
/// capture's pixels never landed on its target.
#[derive(Debug)]
pub struct CompositionFailed {
    /// The status the command buffer ended with.
    pub status: MTLCommandBufferStatus,
    /// The command buffer's error description, when Metal attached one.
    pub error: Option<String>,
}

impl fmt::Display for CompositionFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "native view composition command buffer ended with status {:?}",
            self.status
        )?;
        if let Some(error) = &self.error {
            write!(f, ": {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CompositionFailed {}

/// The capture outcome a composition command buffer's final `status`
/// answers. Only a completed buffer landed its pixels; any other status is
/// a failed capture carrying that status and `error`, never a deferral —
/// no redraw is owed for a buffer that already ended.
fn composition_outcome(
    status: MTLCommandBufferStatus,
    error: Option<String>,
) -> Result<(), CaptureError> {
    if status == MTLCommandBufferStatus::Completed {
        Ok(())
    } else {
        Err(CaptureError::Failed(Arc::new(CompositionFailed {
            status,
            error,
        })))
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
        MTLCommandBufferStatus, MTLCopyAllDevices, MTLCreateSystemDefaultDevice, MTLPixelFormat,
        MTLResource, MTLScissorRect, MTLSize, MTLTexture,
    };

    use super::{
        CaptureError, CompositionFailed, CompositorState, FenceBatch, NativeRenderer, SurfaceSpec,
        composition_outcome,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// A composition command buffer's final status alone decides the
    /// capture: a completed buffer lands, and every other status fails the
    /// capture carrying that status and the buffer's error — never a
    /// deferral waiting on a redraw nothing issues.
    #[test]
    fn a_non_completed_composition_fails_with_its_status() {
        assert!(
            composition_outcome(MTLCommandBufferStatus::Completed, None).is_ok(),
            "a completed composition lands"
        );
        for status in [
            MTLCommandBufferStatus::NotEnqueued,
            MTLCommandBufferStatus::Enqueued,
            MTLCommandBufferStatus::Committed,
            MTLCommandBufferStatus::Scheduled,
            MTLCommandBufferStatus::Error,
        ] {
            match composition_outcome(status, Some("device hung".to_owned())) {
                Err(CaptureError::Failed(error)) => {
                    let failed = error
                        .downcast_ref::<CompositionFailed>()
                        .expect("the failure carries the composition's status");
                    assert_eq!(failed.status, status, "the failure carries its own status");
                    assert_eq!(failed.error.as_deref(), Some("device hung"));
                }
                other => panic!("status {status:?} must fail the capture, got {other:?}"),
            }
        }
    }

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
        // A different destination key drains the pool the same way:
        // frames issued under one geometry never answer another, and
        // stale-key returns drop rather than re-populate.
        let wide_a = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 128, 96, 1);
        let wide_b = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 128, 96, 1);
        let narrow = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 1);
        renderer.return_frame(wide_a.into_frame());
        renderer.return_frame(wide_b.into_frame());
        assert!(
            renderer.available.is_empty(),
            "frames from a superseded key must never rejoin the pool"
        );
        // And a pool the owner ended (shutdown) drops a still-outstanding
        // frame instead of re-arming — a post-teardown settle is
        // reachable, so it must be quiet, not a panic.
        renderer = NativeRenderer::default();
        renderer.return_frame(narrow.into_frame());
        assert!(
            renderer.available.is_empty(),
            "a frame settling after shutdown is dropped, not re-pooled"
        );
        let _rearmed = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 0);
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
            move |result: Result<(), CaptureError>| {
                calls.fetch_add(1, Ordering::Relaxed);
                *outcome.lock().expect("outcome lock") = Some(result);
            }
        });
        batch.complete_one(Ok(()));
        batch.complete_one(Err(CaptureError::Deferred));
        // The deferred fence does not end the batch — the third
        // submission is still in flight.
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        batch.complete_one(Ok(()));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let result = outcome.lock().expect("outcome lock").take();
        assert!(
            matches!(result, Some(Err(CaptureError::Deferred))),
            "a batch with a deferred surface must not report success"
        );

        // An all-submitted batch still reports success, once.
        let calls = Arc::new(AtomicUsize::new(0));
        let batch = FenceBatch::new(2, {
            let calls = Arc::clone(&calls);
            move |result: Result<(), CaptureError>| {
                assert!(result.is_ok());
                calls.fetch_add(1, Ordering::Relaxed);
            }
        });
        batch.complete_one(Ok(()));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        batch.complete_one(Ok(()));
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        // A terminal `Failed` wins over a `Deferred` whichever fence
        // settles first: `Deferred` first, then `Failed`…
        let outcome = Arc::new(Mutex::new(None));
        let batch = FenceBatch::new(2, {
            let outcome = Arc::clone(&outcome);
            move |result: Result<(), CaptureError>| {
                *outcome.lock().expect("outcome lock") = Some(result);
            }
        });
        let error: Arc<dyn std::error::Error + Send + Sync> =
            Arc::new(std::io::Error::other("surface rejected"));
        batch.complete_one(Err(CaptureError::Deferred));
        batch.complete_one(Err(CaptureError::Failed(error.clone())));
        let result = outcome.lock().expect("outcome lock").take();
        assert!(
            matches!(&result, Some(Err(CaptureError::Failed(failed))) if Arc::ptr_eq(failed, &error)),
            "a terminal failure answered even after the deferral landed first: {result:?}"
        );

        // …and `Failed` first, then `Deferred` — the same terminal
        // outcome.
        let outcome = Arc::new(Mutex::new(None));
        let batch = FenceBatch::new(2, {
            let outcome = Arc::clone(&outcome);
            move |result: Result<(), CaptureError>| {
                *outcome.lock().expect("outcome lock") = Some(result);
            }
        });
        batch.complete_one(Err(CaptureError::Failed(error.clone())));
        batch.complete_one(Err(CaptureError::Deferred));
        let result = outcome.lock().expect("outcome lock").take();
        assert!(
            matches!(&result, Some(Err(CaptureError::Failed(failed))) if Arc::ptr_eq(failed, &error)),
            "a terminal failure answered even with the deferral landing last: {result:?}"
        );
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
            full_size: MTLSize {
                width: 8,
                height: 8,
                depth: 1,
            },
            clip: MTLScissorRect {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            },
            uv_origin: [0.0, 0.0],
            uv_scale: [1.0, 1.0],
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
            assert_eq!(texture.width(), spec.full_size.width);
        }
    }

    /// `surface_spec` placements: the mapped full rect keeps every texel
    /// its content touches while the clip and UV window select what the
    /// destination samples.
    mod spec {
        use objc2_metal::{MTLPixelFormat, MTLScissorRect};

        use crate::geometry::Rect;

        use super::super::{CaptureGeometry, SurfaceSpec, surface_spec};

        /// The 2× geometry a 200×200 source draws on a 400×400 target.
        fn down(source: Rect) -> CaptureGeometry {
            CaptureGeometry::new(source, 400, 400, true)
        }

        /// The same scale under an unflipped (bottom-up) source.
        fn up(source: Rect) -> CaptureGeometry {
            CaptureGeometry::new(source, 400, 400, false)
        }

        fn spec_for(geometry: CaptureGeometry, child: Rect) -> SurfaceSpec {
            surface_spec(0, child, geometry, MTLPixelFormat::BGRA8Unorm, 400, 400)
                .expect("the child intersects the target")
        }

        /// (x, y, width, height) of the spec's clip rect.
        fn clip(spec: &SurfaceSpec) -> (usize, usize, usize, usize) {
            let MTLScissorRect {
                x,
                y,
                width,
                height,
            } = spec.clip;
            (x, y, width, height)
        }

        /// The UV window as a whole value so a single `assert_eq!` asserts
        /// the mapping — every expected component is exact by construction
        /// (integer pixel ratios through floor/ceil endpoints), and
        /// comparing the derived `PartialEq` value needs no
        /// float-comparison lint exception.
        #[derive(Debug, PartialEq)]
        struct UvWindow {
            origin: [f32; 2],
            scale: [f32; 2],
        }

        fn uv(spec: &SurfaceSpec) -> UvWindow {
            UvWindow {
                origin: spec.uv_origin,
                scale: spec.uv_scale,
            }
        }

        #[test]
        fn a_top_down_source_maps_straight_through() {
            let spec = spec_for(
                down(Rect::new(0.0, 0.0, 200.0, 200.0)),
                Rect::new(40.0, 20.0, 100.0, 100.0),
            );
            assert_eq!(clip(&spec), (80, 40, 200, 200));
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_eq!(
                uv(&spec),
                UvWindow {
                    origin: [0.0, 0.0],
                    scale: [1.0, 1.0],
                }
            );
        }

        #[test]
        fn a_bottom_up_source_mirrors_against_its_own_height() {
            let spec = spec_for(
                up(Rect::new(0.0, 0.0, 200.0, 200.0)),
                Rect::new(40.0, 20.0, 100.0, 100.0),
            );
            assert_eq!(clip(&spec), (80, 160, 200, 200));
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_eq!(
                uv(&spec),
                UvWindow {
                    origin: [0.0, 0.0],
                    scale: [1.0, 1.0],
                }
            );
        }

        #[test]
        fn a_non_zero_source_origin_offsets_the_child() {
            // The bounds' origin is where texel (0,0) samples: a child
            // sitting 40pt right and 20pt down of it lands at (80,40),
            // not at its own coordinates scaled.
            let spec = spec_for(
                down(Rect::new(50.0, 30.0, 200.0, 200.0)),
                Rect::new(90.0, 50.0, 100.0, 100.0),
            );
            assert_eq!(clip(&spec), (80, 40, 200, 200));
            let spec = spec_for(
                up(Rect::new(50.0, 30.0, 200.0, 200.0)),
                Rect::new(90.0, 50.0, 100.0, 100.0),
            );
            assert_eq!(clip(&spec), (80, 160, 200, 200));
        }

        #[test]
        fn a_surface_past_the_top_left_keeps_its_full_texture() {
            let spec = spec_for(
                down(Rect::new(0.0, 0.0, 200.0, 200.0)),
                Rect::new(-40.0, -20.0, 100.0, 100.0),
            );
            // The producer renders its whole 200×200 texture; only the
            // destination scissor and the sampled UV window shrink.
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_eq!(clip(&spec), (0, 0, 120, 160));
            assert_eq!(
                uv(&spec),
                UvWindow {
                    origin: [0.4, 0.2],
                    scale: [0.6, 0.8],
                }
            );
        }

        #[test]
        fn an_empty_child_rect_maps_to_none() {
            let geometry = down(Rect::new(0.0, 0.0, 200.0, 200.0));
            for rect in [
                // Zero extent at a fractional position must not quantize
                // into a 1-pixel producer.
                Rect::new(0.9, 0.9, 0.0, 10.0),
                Rect::new(0.9, 0.9, 10.0, 0.0),
                Rect::new(0.9, 0.9, -10.0, 10.0),
                Rect::new(0.9, 0.9, 10.0, -10.0),
            ] {
                assert!(
                    surface_spec(0, rect, geometry, MTLPixelFormat::BGRA8Unorm, 400, 400).is_none(),
                    "an empty child rect maps to None: {rect:?}"
                );
            }
        }

        #[test]
        fn a_surface_past_the_bottom_right_keeps_its_full_texture() {
            let spec = spec_for(
                down(Rect::new(0.0, 0.0, 200.0, 200.0)),
                Rect::new(160.0, 180.0, 100.0, 100.0),
            );
            // The producer covers the whole mapped rect; only the
            // destination scissor and the sampled UV window shrink.
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_eq!(clip(&spec), (320, 360, 80, 40));
            assert_eq!(
                uv(&spec),
                UvWindow {
                    origin: [0.0, 0.0],
                    scale: [0.4, 0.2],
                }
            );
        }

        #[test]
        fn a_bottom_up_source_crops_through_the_same_uv_window() {
            // Bottom-up source, child hanging off the bottom-left: the full
            // producer is preserved while clip and UV window describe the
            // visible corner — mirroring against the source height lands
            // the rect on the target's lower rows.
            let spec = spec_for(
                up(Rect::new(0.0, 0.0, 200.0, 200.0)),
                Rect::new(-40.0, -20.0, 100.0, 100.0),
            );
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_eq!(clip(&spec), (0, 240, 120, 160));
            assert_eq!(
                uv(&spec),
                UvWindow {
                    origin: [0.4, 0.0],
                    scale: [0.6, 0.8],
                }
            );
        }

        #[test]
        fn a_surface_entirely_outside_maps_to_none() {
            let geometry = down(Rect::new(0.0, 0.0, 200.0, 200.0));
            for child in [
                Rect::new(500.0, 0.0, 100.0, 100.0),
                Rect::new(-500.0, 0.0, 100.0, 100.0),
            ] {
                assert!(
                    surface_spec(0, child, geometry, MTLPixelFormat::BGRA8Unorm, 400, 400)
                        .is_none(),
                    "a surface entirely outside maps to None: {child:?}"
                );
            }
        }

        #[test]
        fn anisotropic_scaling_uses_each_axiss_own_scale() {
            // 200×200 points into 400×800 pixels: 2× horizontal, 4× vertical.
            let geometry = CaptureGeometry::new(Rect::new(0.0, 0.0, 200.0, 200.0), 400, 800, true);
            let spec = surface_spec(
                0,
                Rect::new(40.0, 20.0, 100.0, 100.0),
                geometry,
                MTLPixelFormat::BGRA8Unorm,
                400,
                800,
            )
            .expect("the child intersects the target");
            assert_eq!(clip(&spec), (80, 80, 200, 400));
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 400));
        }

        #[test]
        fn fractional_endpoints_size_from_ceil_high_minus_floor_low() {
            // A 1× geometry: (0.9, 0.9)–(1.4, 1.4) touches pixels 0 and 1,
            // so the size is 2 — not floor(0.9) + ceil(0.5) = 1.
            let spec = spec_for(
                CaptureGeometry::new(Rect::new(0.0, 0.0, 400.0, 400.0), 400, 400, true),
                Rect::new(0.9, 0.9, 0.5, 0.5),
            );
            assert_eq!(clip(&spec), (0, 0, 2, 2));
            assert_eq!((spec.full_size.width, spec.full_size.height), (2, 2));
        }
    }

    /// The raster's draw transform, mapped point by point.
    mod raster_transform {
        use objc2_core_foundation::CGAffineTransform;

        use crate::geometry::Rect;

        use super::super::{CaptureGeometry, raster_transform};

        /// A layer-space point through the transform.
        fn rendered(transform: CGAffineTransform, point: (f64, f64)) -> (f64, f64) {
            let CGAffineTransform { a, b, c, d, tx, ty } = transform;
            (
                point.1.mul_add(c, point.0 * a) + tx,
                point.1.mul_add(d, point.0 * b) + ty,
            )
        }

        #[test]
        fn the_raster_transform_scales_about_the_bounds_origin() {
            // The content at the bounds origin lands on texel (0,0); every
            // other point scales by the pixel ratio about it.
            let geometry =
                CaptureGeometry::new(Rect::new(50.0, 30.0, 200.0, 200.0), 400, 400, false);
            let transform = raster_transform(geometry, 400.0);
            assert_eq!(rendered(transform, (50.0, 30.0)), (0.0, 0.0));
            assert_eq!(rendered(transform, (90.0, 80.0)), (80.0, 100.0));
        }

        #[test]
        fn the_raster_transform_mirrors_a_y_down_source_about_the_window() {
            // A y-down source mirrors about the visible window: its
            // layer-space top edge lands on destination row 0 and its
            // bottom edge on the last row, anchored at the bounds origin,
            // not the destination's own origin.
            let geometry =
                CaptureGeometry::new(Rect::new(50.0, 30.0, 200.0, 200.0), 400, 400, true);
            let transform = raster_transform(geometry, 400.0);
            assert_eq!(rendered(transform, (50.0, 230.0)), (0.0, 0.0));
            assert_eq!(rendered(transform, (50.0, 30.0)), (0.0, 400.0));
            assert_eq!(rendered(transform, (150.0, 130.0)), (200.0, 200.0));
        }
    }
}
