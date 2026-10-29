//! View-subtree capture into a Metal texture.
//!
//! [`ViewCapture`] renders a view subtree — including any GPU surfaces
//! nested inside it — into a caller-owned texture for the filter and
//! view-effect pipelines: the layer tree is rasterized synchronously by
//! `CALayer.renderInContext` into a `CGContext` whose backing store is a
//! shared `MTLBuffer`, a Metal blit transfer into a private 2D texture
//! lets the compositor sample the native pixels with no CPU readback,
//! each [`CapturableSurface`] gets its own private texture, and a final
//! pass on a shared serial queue composites them. The raster itself is
//! still CPU work — the transfer is a GPU blit, not the drawing.
//!
//! # Tree order
//!
//! `renderInContext` cannot draw a GPU surface, and a surface may sit
//! anywhere in the native tree: under an opaque ancestor background, over
//! native siblings, beneath a translucent label, inside a clipped,
//! transformed or group-opacity ancestor. The capture therefore never
//! layers a surface over or under "the native raster" as a whole. It
//! rasterizes the tree once with every surface transparent, then again
//! with an opaque black and an opaque white *probe* layer standing exactly
//! where each surface's presentation layer sits — same superlayer, same
//! geometry, directly above it. Core Animation's compositing is affine in a
//! layer's premultiplied colour, so the three rasters give, per pixel, how
//! the native tree composites whatever the surface draws: what covers it,
//! what it covers, and every ancestor effect between. The composite pass
//! resolves each surface's real texture through that response — see
//! `capture_resolve_fragment` in `capture_composite.metal`. Surfaces whose
//! destination rects overlap take distinct probe slots, and their probes
//! are rasterized in every combination, so overlapping GPU surfaces keep
//! their own order too.
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
use dispatch2::{DispatchData, DispatchQoS, DispatchQueue, GlobalQueueIdentifier, MainThreadBound};
use filtrate_core::space::{
    P3_TO_SRGB, SRGB_ENCODED_BREAK, SRGB_GAMMA, SRGB_INVERSE_GAMMA, SRGB_LINEAR_BREAK, SRGB_OFFSET,
    SRGB_SCALE, SRGB_SLOPE, SRGB_TO_P3,
};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{ClassType, Message};
use objc2_core_foundation::{CFRetained, CGAffineTransform};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColor, CGColorSpace, CGContext, CGImageAlphaInfo,
    CGImageByteOrderInfo, CGImageComponentInfo, kCGColorSpaceExtendedSRGB,
};
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor,
    MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue, MTLDevice,
    MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType,
    MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLResource, MTLResourceOptions, MTLSamplerAddressMode,
    MTLSamplerDescriptor, MTLSamplerMinMagFilter, MTLSamplerState, MTLScissorRect, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};
use objc2_quartz_core::{CALayer, CAMetalLayer, CATransaction};

use crate::PlatformView;
use crate::core_animation::flush_transaction;
use crate::geometry::Rect;
use crate::main_queue::enqueue;
use crate::metal::{Primaries, ShaderColor, shader_color};

/// The compositor's metallib, precompiled from `capture_composite.metal`
/// at build time with `-ftexture-write-rounding-mode=rte` (#2827): a
/// source-compiled library always takes the platform's toward-zero float
/// store default — `MTLCompileOptions` carries no rounding switch — so
/// the composite pass would land every `RGBA16Float` channel one f16 step
/// low. The bytes are embedded, so the kit carries no bundle resources.
const CAPTURE_COMPOSITE_METALLIB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/capture_composite.metallib"));

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

    /// Where `point`, in the captured view's own layer space, lands in the
    /// destination's top-down pixel space — the mapping the native raster
    /// draws through ([`raster_transform`] plus the bitmap context's
    /// bottom-up rows).
    #[must_use]
    pub fn texel(&self, point: crate::geometry::Point) -> [f64; 2] {
        let origin = self.source.origin;
        let y = if self.y_down {
            point.y - origin.y
        } else {
            // A bottom-up source measures `y` from the bottom: its top
            // edge lands on the destination's first row.
            origin.y + self.source.size.height - point.y
        };
        [(point.x - origin.x) * self.scale_x, y * self.scale_y]
    }
}

/// Where a GPU surface's own texture lands inside a capture.
///
/// `full_size` is the producer texture's pixel size, and the texture
/// covers the surface's whole presentation rect. `clip` is the
/// destination rect inside the capture target its pixels can reach — the
/// bounding box of the rect as it is mapped, rotation and scale included,
/// clipped to the target — and `uv_*` is the inverse mapping: the texture
/// coordinate a capture-target point samples is
/// `uv_offset + x * uv_dx + y * uv_dy`. Clipping therefore crops the
/// destination without ever resizing or re-laying out the producer.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSpec {
    /// The capture's identity for the surface — the surface view's address.
    pub surface_id: usize,
    /// The producer texture's pixel size — the mapped rect's edge
    /// lengths, rounded up. Allocates the private texture and is the size
    /// [`CapturableSurface::render_prepared_external_texture`] renders at.
    pub full_size: MTLSize,
    /// The destination rect inside the capture target the surface can
    /// reach — non-negative and inside the target by construction, so no
    /// Metal scissor bound is exercised.
    pub clip: MTLScissorRect,
    /// The texture coordinate capture-target point (0, 0) maps to.
    pub uv_offset: [f32; 2],
    /// The texture coordinate's derivative along the target's x axis.
    pub uv_dx: [f32; 2],
    /// The texture coordinate's derivative along the target's y axis.
    pub uv_dy: [f32; 2],
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

/// The window a full-screen draw samples through — the whole texture.
const IDENTITY_UV: CompositeRegion = CompositeRegion {
    uv_origin: [0.0, 0.0],
    uv_scale: [1.0, 1.0],
};

/// The colour every capture source is blended in: sRGB-encoded values,
/// extended past [0, 1] so wide-gamut and HDR content survives.
///
/// Core Animation composites a window's layers on their sRGB-encoded
/// values, so a translucent native layer over a GPU surface appears on
/// screen as an encoded-space blend. Blending the same sources in linear
/// light comes out lighter (#3528), so the capture blends in this space
/// whatever its target stores.
const BLEND_COLOR: ShaderColor = ShaderColor {
    primaries: Primaries::Srgb,
    encoded: true,
};

/// The format that holds [`BLEND_COLOR`] at extended range — the native
/// raster of a half-float capture, the blend texture a target of another
/// colour is composited through, and the resolve's intermediate images.
const BLEND_FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA16Float;

/// The colour conversion handed to the composite fragment shaders — mirrors
/// `CaptureCompositeConversion` in `capture_composite.metal`: `float4`
/// fields are `f32` quadruples, and `align(16)` pads the struct to the
/// 96 bytes Metal lays it out in.
///
/// Every source — the native rasters ([`raster_color`]), each surface's
/// texture ([`shader_color`] of its format) — samples in its own colour, and
/// the composite must store the colour of what it draws into — the
/// [`BLEND_COLOR`] blend, then the target: a half-float surface holds linear
/// Display P3, which an 8-bit target read as sRGB-encoded would show darker
/// and shifted toward P3's primaries (#3528).
#[repr(C, align(16))]
struct CompositeConversion {
    rows: [[f32; 4]; 3],
    transfer: [f32; 4],
    breaks: [f32; 4],
    identity: u32,
    decode_source: u32,
    encode_target: u32,
}

impl CompositeConversion {
    /// The conversion from what a source samples as to what the target's
    /// store expects.
    fn between(source: ShaderColor, target: ShaderColor) -> Self {
        let matrix = match (source.primaries, target.primaries) {
            (Primaries::DisplayP3, Primaries::Srgb) => P3_TO_SRGB,
            (Primaries::Srgb, Primaries::DisplayP3) => SRGB_TO_P3,
            (Primaries::Srgb, Primaries::Srgb) | (Primaries::DisplayP3, Primaries::DisplayP3) => {
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            }
        };
        Self {
            rows: matrix.map(|[r, g, b]| [r, g, b, 0.0]),
            transfer: [SRGB_SLOPE, SRGB_OFFSET, SRGB_SCALE, SRGB_GAMMA],
            breaks: [
                SRGB_INVERSE_GAMMA,
                SRGB_LINEAR_BREAK,
                SRGB_ENCODED_BREAK,
                0.0,
            ],
            identity: u32::from(source == target),
            decode_source: u32::from(source.encoded),
            encode_target: u32::from(target.encoded),
        }
    }

    /// Binds the conversion to fragment buffer `index` of `encoder`.
    fn bind(&self, encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>, index: usize) {
        // SAFETY: `encoder` is a live render encoder, `index` is a
        // conversion slot the bound fragment function declares, and
        // `setFragmentBytes` copies `self`'s bytes into the command.
        unsafe {
            encoder.setFragmentBytes_length_atIndex(
                std::ptr::NonNull::from(self).cast::<core::ffi::c_void>(),
                core::mem::size_of::<Self>(),
                index,
            );
        }
    }
}

/// The per-draw parameters of `capture_resolve_fragment` — mirrors
/// `CaptureResolveParams` in `capture_composite.metal`: six `float2`
/// pairs and a `uint`, laid out alike under `repr(C)`.
#[repr(C)]
struct ResolveParams {
    output_origin: [f32; 2],
    x0_origin: [f32; 2],
    probe_origin: [f32; 2],
    uv_offset: [f32; 2],
    uv_dx: [f32; 2],
    uv_dy: [f32; 2],
    has_surface: u32,
}

/// A pixel extent as the `f64` Metal viewports and bounds take.
fn pixel_extent(value: usize) -> f64 {
    f64::from(u32::try_from(value).expect("a pixel extent fits in u32"))
}

/// A pixel position as the `f32` the resolve shader offsets by.
#[expect(
    clippy::cast_precision_loss,
    reason = "a capture texture is at most a few thousand pixels on a side"
)]
const fn pixel_offset(value: usize) -> f32 {
    value as f32
}

/// The spec a surface whose texture's top-left, top-right and
/// bottom-left corners land at `quad` — in the capture's top-down pixel
/// space — maps to in a `target_width` × `target_height` destination.
///
/// The producer keeps the mapped rect's full extent: its edge lengths,
/// rounded up, size the texture whatever part of it the target shows.
/// Only the destination rect is clipped against the target — the mapped
/// corners' bounding box, the low edge floored and the high edge ceiled so
/// every texel the rect touches is resolved. `None` when the rect has no
/// area or lands entirely outside.
///
/// `pixel_format` is a closure so it is asked only once the spec is known
/// to produce a rendered surface: a surface the geometry rejects has no
/// format to declare, and asking it can panic on a surface that never
/// configured one.
#[must_use]
pub fn surface_spec(
    surface_id: usize,
    quad: [[f64; 2]; 3],
    pixel_format: impl FnOnce() -> MTLPixelFormat,
    target_width: usize,
    target_height: usize,
) -> Option<SurfaceSpec> {
    let [origin, right, down] = quad;
    let across = [right[0] - origin[0], right[1] - origin[1]];
    let below = [down[0] - origin[0], down[1] - origin[1]];
    let det = across[0].mul_add(below[1], -(across[1] * below[0]));
    // A rect with no area touches no texels — quantizing its zero-area
    // span would still produce a 1-pixel producer for nothing.
    if det.abs() <= f64::EPSILON {
        return None;
    }
    let corners = [
        origin,
        right,
        down,
        [
            origin[0] + across[0] + below[0],
            origin[1] + across[1] + below[1],
        ],
    ];
    let (low, high) = corners.iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(low, high), corner| {
            (
                [low[0].min(corner[0]), low[1].min(corner[1])],
                [high[0].max(corner[0]), high[1].max(corner[1])],
            )
        },
    );
    #[expect(
        clippy::cast_precision_loss,
        reason = "a capture texture is at most a few thousand pixels on a side"
    )]
    let (target_width, target_height) = (target_width as f64, target_height as f64);
    let clip_x = low[0].floor().max(0.0);
    let clip_y = low[1].floor().max(0.0);
    let clip_w = high[0].ceil().min(target_width) - clip_x;
    let clip_h = high[1].ceil().min(target_height) - clip_y;
    if !(clip_w > 0.0 && clip_h > 0.0) {
        return None;
    }

    // The inverse of `uv ↦ origin + u·across + v·below`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "UV derivatives and offsets are small — f32 precision is ample"
    )]
    let ([uv_dx, uv_along_y], uv_offset) = {
        let horizontal = [below[1] / det, -across[1] / det];
        let vertical = [-below[0] / det, across[0] / det];
        let offset = [
            -origin[0].mul_add(horizontal[0], origin[1] * vertical[0]),
            -origin[0].mul_add(horizontal[1], origin[1] * vertical[1]),
        ];
        (
            [horizontal, vertical].map(|axis| axis.map(|value| value as f32)),
            offset.map(|value| value as f32),
        )
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "edge lengths are positive and clip extents are bounded by the non-negative target"
    )]
    let (full_width, full_height, clip_x, clip_y, clip_w, clip_h) = (
        across[0].hypot(across[1]).ceil().max(1.0) as usize,
        below[0].hypot(below[1]).ceil().max(1.0) as usize,
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
        uv_offset,
        uv_dx,
        uv_dy: uv_along_y,
        pixel_format: pixel_format(),
    })
}

/// Where the texture of a surface presenting through `presentation` lands
/// in a capture of `root` — the texture's top-left, top-right and
/// bottom-left corners in the capture's top-down pixel space, the `quad`
/// [`surface_spec`] takes.
///
/// The corners go through Core Animation's own layer geometry —
/// `convertPoint:toLayer:` applies every transform, sublayer transform and
/// geometry flip between the two layers, exactly as the raster draws the
/// probe standing in for the surface — and then through
/// [`CaptureGeometry::texel`]. Which of the presentation layer's edges
/// carries the texture's first row is Core Animation's contents
/// orientation: the layer's minimum-y edge where its y axis points down
/// the screen — `UIKit`'s convention, and `AppKit`'s wherever
/// `contentsAreFlipped` — its maximum-y edge otherwise.
fn surface_quad(
    presentation: &CALayer,
    root: &CALayer,
    geometry: CaptureGeometry,
) -> [[f64; 2]; 3] {
    let bounds: Rect = presentation.bounds().into();
    let flipped = presentation.contentsAreFlipped();
    let first_row_at_min_y = if cfg!(target_os = "macos") {
        flipped
    } else {
        !flipped
    };
    let (top, bottom) = if first_row_at_min_y {
        (bounds.origin.y, bounds.origin.y + bounds.size.height)
    } else {
        (bounds.origin.y + bounds.size.height, bounds.origin.y)
    };
    let left = bounds.origin.x;
    let right = bounds.origin.x + bounds.size.width;
    [(left, top), (right, top), (left, bottom)].map(|(x, y)| {
        let point =
            presentation.convertPoint_toLayer(crate::geometry::Point::new(x, y).into(), Some(root));
        geometry.texel(point.into())
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
    /// The layer whose pixels this surface's texture stands for — the one
    /// its presentation draws into and its capture suppression hides —
    /// or `None` once the surface's owner is gone.
    ///
    /// The texture covers the layer's whole bounds, and a capture places
    /// it in the native tree exactly where this layer sits: a probe layer
    /// with its geometry and opacity stands directly above it in its
    /// superlayer while the capture measures how the tree composites it.
    /// The layer's own mask is not part of that geometry, so a
    /// presentation layer carries none.
    fn presentation_layer(&self) -> Option<Retained<CALayer>>;
    /// Suspends the surface's own presentation while its content is captured
    /// elsewhere — hides [`presentation_layer`](Self::presentation_layer).
    fn begin_capture_suppression(&self);
    /// Resumes the surface's presentation.
    fn end_capture_suppression(&self);
    /// Whether the surface's own presentation can put a frame in front of
    /// the user right now — the window-level half of `canPresentNow`: a
    /// surface inside a window that cannot present (never ordered in,
    /// occluded, scene-less, or not attached to a window at all) answers
    /// `false` no matter its own bounds. A `false` answer joins the
    /// surface to the capture's external rendering even while its bounds
    /// give it no [`SurfaceSpec`] — a zero-size surface included — so its
    /// redraw requests reach the capture's drive instead of dying on a
    /// presentation path that can never run.
    fn can_present(&self) -> bool;
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
    /// `true` once this surface's first presented frame can never land —
    /// a settled terminal failure the capture wait resolves on rather
    /// than hanging. Not a substitute for [`Self::has_presented_frame`]:
    /// the wait covers `presented || failed`, nothing else.
    fn presentation_failed(&self) -> bool;
    /// The last frame this surface put on screen, as an immutable GPU
    /// input, or `None` when it never presented — handed back with the
    /// surface's presentation layer. Every surface a retirement walk
    /// reaches renders under a transition declaration and is armed.
    /// Called once, during a transition's retirement walk, while the
    /// surface is still live.
    fn take_frozen_frame(&self) -> (Option<FrozenSurface>, Retained<CAMetalLayer>);
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

/// A retained Metal texture.
type Texture = Retained<ProtocolObject<dyn MTLTexture>>;

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
    blend_texture: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    /// The resolve's intermediate images by pixel size, kept at what the
    /// last composition used.
    scratch: HashMap<(usize, usize), Vec<Texture>>,
    /// How many of each size's `scratch` the composition being encoded
    /// has handed out — distinct images never alias inside one.
    scratch_handed: HashMap<(usize, usize), usize>,
    pipelines: HashMap<MTLPixelFormat, Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
    resolve_pipelines:
        HashMap<MTLPixelFormat, Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
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
            blend_texture: None,
            scratch: HashMap::new(),
            scratch_handed: HashMap::new(),
            pipelines: HashMap::new(),
            resolve_pipelines: HashMap::new(),
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

    /// The [`BLEND_FORMAT`] texture a target that does not store
    /// [`BLEND_COLOR`] is composited through — reused while the size
    /// matches.
    ///
    /// # Panics
    ///
    /// When the device cannot allocate a texture.
    fn blend_texture(
        &mut self,
        width: usize,
        height: usize,
    ) -> Retained<ProtocolObject<dyn MTLTexture>> {
        if let Some(texture) = &self.blend_texture
            && texture.width() == width
            && texture.height() == height
        {
            return texture.clone();
        }
        let texture = self.new_blend_format_texture(width, height);
        self.blend_texture = Some(texture.clone());
        texture
    }

    /// A private, sampled and renderable [`BLEND_FORMAT`] texture.
    ///
    /// # Panics
    ///
    /// When the device cannot allocate it.
    fn new_blend_format_texture(
        &self,
        width: usize,
        height: usize,
    ) -> Retained<ProtocolObject<dyn MTLTexture>> {
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                BLEND_FORMAT,
                width,
                height,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        descriptor.setStorageMode(MTLStorageMode::Private);
        self.device
            .newTextureWithDescriptor(&descriptor)
            .expect("failed to create a capture blend-format texture")
    }

    /// `count` intermediate images of `width` × `height`, distinct from
    /// every other image this composition was handed. Reused across
    /// compositions: the compositor's command buffers run in order on
    /// one queue, so a later composition's writes follow every earlier
    /// read.
    fn scratch_textures(
        &mut self,
        width: usize,
        height: usize,
        count: usize,
    ) -> Vec<Retained<ProtocolObject<dyn MTLTexture>>> {
        let size = (width, height);
        let start = self.scratch_handed.get(&size).copied().unwrap_or(0);
        let needed = start + count;
        let available = self.scratch.get(&size).map_or(0, Vec::len);
        let fresh: Vec<_> = (available..needed)
            .map(|_| self.new_blend_format_texture(width, height))
            .collect();
        let pool = self.scratch.entry(size).or_default();
        pool.extend(fresh);
        self.scratch_handed.insert(size, needed);
        pool[start..needed].to_vec()
    }

    /// Keeps only the intermediate images the composition just encoded
    /// used, and starts the next composition's hand-out from zero.
    fn settle_scratch(&mut self) {
        let handed = std::mem::take(&mut self.scratch_handed);
        self.scratch.retain(|size, pool| {
            let used = handed.get(size).copied().unwrap_or(0);
            pool.truncate(used);
            used > 0
        });
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
        if let Some(pipeline) = self.pipelines.get(&format) {
            return pipeline.clone();
        }
        let compiled = self.compile_pipeline(format, "capture_composite_fragment", true);
        self.pipelines.insert(format, compiled.clone());
        compiled
    }

    /// The ordered-resolve pipeline for `format` — `capture_resolve_fragment`
    /// with blending off: a resolve computes each pixel's whole value.
    ///
    /// # Panics
    ///
    /// When the in-tree shader fails to compile or lacks its entry points.
    fn resolve_pipeline(
        &mut self,
        format: MTLPixelFormat,
    ) -> Retained<ProtocolObject<dyn MTLRenderPipelineState>> {
        if let Some(pipeline) = self.resolve_pipelines.get(&format) {
            return pipeline.clone();
        }
        let compiled = self.compile_pipeline(format, "capture_resolve_fragment", false);
        self.resolve_pipelines.insert(format, compiled.clone());
        compiled
    }

    /// Compiles the composite vertex stage with `fragment` into a `format`
    /// pipeline — premultiplied source-over when `blending`, a plain store
    /// otherwise.
    ///
    /// # Panics
    ///
    /// When the in-tree shader fails to compile or lacks its entry points.
    fn compile_pipeline(
        &self,
        format: MTLPixelFormat,
        fragment: &str,
        blending: bool,
    ) -> Retained<ProtocolObject<dyn MTLRenderPipelineState>> {
        let library = self
            .device
            .newLibraryWithData_error(&DispatchData::from_static_bytes(CAPTURE_COMPOSITE_METALLIB))
            .expect("failed to load the precompiled capture composite Metal library");
        let vertex = library
            .newFunctionWithName(&NSString::from_str("capture_composite_vertex"))
            .expect("CaptureComposite is missing capture_composite_vertex");
        let fragment = library
            .newFunctionWithName(&NSString::from_str(fragment))
            .unwrap_or_else(|| panic!("CaptureComposite is missing {fragment}"));
        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(&vertex));
        descriptor.setFragmentFunction(Some(&fragment));
        // SAFETY: index 0 is the single color attachment.
        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setPixelFormat(format);
        attachment.setBlendingEnabled(blending);
        attachment.setRgbBlendOperation(MTLBlendOperation::Add);
        attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
        attachment.setSourceRGBBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        self.device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .expect("failed to compile the Metal capture composition pipeline")
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

    /// Encodes the composition of one capture into `target`: the base
    /// raster `base` — the native tree with every surface transparent —
    /// then each overlap cluster in `clusters` resolved through its probe
    /// rasters and the `surfaces` it owns (`capture_resolve_fragment`).
    ///
    /// The sources blend in [`BLEND_COLOR`], the encoded space Core
    /// Animation composites a window's layers in, whatever `target`
    /// stores: a translucent native layer over a GPU surface must come out
    /// as it does on screen. A target that stores [`BLEND_COLOR`] is
    /// composed into directly; any other is composed into a
    /// [`BLEND_FORMAT`] texture first, then converted into `target`'s
    /// colour in one more pass — the half-float blend texture keeps
    /// extended range, so nothing clips on the way.
    ///
    /// # Panics
    ///
    /// When a pipeline, encoder or texture cannot be created.
    pub(crate) fn encode_composition(
        &mut self,
        surfaces: &[RenderedSurface],
        base: &Placed,
        clusters: &[ClusterImages],
        target: &ProtocolObject<dyn MTLTexture>,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
        device: &ProtocolObject<dyn MTLDevice>,
    ) {
        let resources = self.state.device_resources(device);
        let finals: Vec<[Placed; 3]> = clusters
            .iter()
            .map(|cluster| resources.encode_levels(cluster, base, surfaces, command_buffer))
            .collect();
        let target_color = shader_color(target.pixelFormat());
        if target_color == BLEND_COLOR {
            resources.encode_main(surfaces, base, clusters, &finals, target, command_buffer);
        } else {
            let blend = resources.blend_texture(target.width(), target.height());
            resources.encode_main(surfaces, base, clusters, &finals, &blend, command_buffer);
            let pass = begin_pass(command_buffer, target, MTLLoadAction::Clear);
            resources.draw_overlay(&pass, &blend, BLEND_COLOR, target_color, target);
            pass.endEncoding();
        }
        resources.settle_scratch();
    }
}

/// A texture the composite reads, where its pixel (0, 0) lands in the
/// capture target, and what it samples as.
#[derive(Clone, Debug)]
pub(crate) struct Placed {
    /// The image.
    pub(crate) texture: Retained<ProtocolObject<dyn MTLTexture>>,
    /// Its pixel (0, 0) in capture-target pixels.
    pub(crate) origin: [usize; 2],
    /// What it samples as.
    pub(crate) color: ShaderColor,
}

/// One overlap cluster as the composite resolves it: surfaces whose
/// destination rects overlap one another, directly or through a chain,
/// and the probe rasters measuring them together.
#[derive(Debug)]
pub(crate) struct ClusterImages {
    /// The union of the members' destination rects — every probe raster's
    /// extent, in capture-target pixels.
    pub(crate) region: MTLScissorRect,
    /// How many probe slots the members take: overlapping members never
    /// share one.
    pub(crate) slots: usize,
    /// One raster of `region` per combination of the slots' probe states
    /// but the all-transparent one, which is the base raster — the
    /// combination `Σ stateᵢ·3ⁱ` (0 transparent, 1 black, 2 white) at
    /// index `combination − 1`. They sample as the base raster does.
    pub(crate) probes: Vec<Retained<ProtocolObject<dyn MTLTexture>>>,
    /// The member surfaces.
    pub(crate) members: Vec<ClusterMember>,
}

/// A cluster member: its probe slot and its index in the composition's
/// rendered surfaces.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClusterMember {
    /// The member's probe slot.
    pub(crate) slot: usize,
    /// The member's index in the rendered surfaces.
    pub(crate) surface: usize,
}

/// Opens a render pass on `texture`'s single colour attachment, cleared
/// to transparent or with its contents left undefined.
///
/// # Panics
///
/// When the encoder cannot be created.
fn begin_pass(
    command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    texture: &ProtocolObject<dyn MTLTexture>,
    load: MTLLoadAction,
) -> Retained<ProtocolObject<dyn MTLRenderCommandEncoder>> {
    let descriptor = MTLRenderPassDescriptor::new();
    // SAFETY: index 0 is the single color attachment.
    let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    attachment.setTexture(Some(texture));
    attachment.setLoadAction(load);
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
    encoder.setViewport(MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: pixel_extent(texture.width()),
        height: pixel_extent(texture.height()),
        znear: 0.0,
        zfar: 1.0,
    });
    // SAFETY: `encoder` is a live render encoder; vertex buffer 0 is the
    // region the composite vertex stage reads, and every draw of the
    // capture's passes covers the whole attachment — `setVertexBytes`
    // copies the bytes into the command.
    unsafe {
        encoder.setVertexBytes_length_atIndex(
            std::ptr::NonNull::from(&IDENTITY_UV).cast::<core::ffi::c_void>(),
            core::mem::size_of::<CompositeRegion>(),
            0,
        );
    }
    encoder
}

impl DeviceResources {
    /// Draws `source` — sampling as `source_color` — premultiplied-over
    /// across the whole of `target`, the pass's attachment, converted into
    /// `target_color`.
    fn draw_overlay(
        &mut self,
        encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
        source: &ProtocolObject<dyn MTLTexture>,
        source_color: ShaderColor,
        target_color: ShaderColor,
        target: &ProtocolObject<dyn MTLTexture>,
    ) {
        encoder.setRenderPipelineState(&self.render_pipeline(target.pixelFormat()));
        encoder.setScissorRect(MTLScissorRect {
            x: 0,
            y: 0,
            width: target.width(),
            height: target.height(),
        });
        CompositeConversion::between(source_color, target_color).bind(encoder, 0);
        let sampler = self.composite_sampler();
        // SAFETY: `encoder` is a live render encoder; index 0 is the
        // texture and sampler slot the composite fragment binds.
        unsafe {
            encoder.setFragmentTexture_atIndex(Some(source), 0);
            encoder.setFragmentSamplerState_atIndex(Some(&sampler), 0);
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
    }

    /// Encodes the main pass into `output`, which stores [`BLEND_COLOR`]:
    /// the base raster over the whole capture, then each cluster's last
    /// resolve level — slot 0 — over its members' rects.
    fn encode_main(
        &mut self,
        surfaces: &[RenderedSurface],
        base: &Placed,
        clusters: &[ClusterImages],
        finals: &[[Placed; 3]],
        output: &ProtocolObject<dyn MTLTexture>,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    ) {
        let pass = begin_pass(command_buffer, output, MTLLoadAction::Clear);
        self.draw_overlay(&pass, &base.texture, base.color, BLEND_COLOR, output);
        for (cluster, images) in clusters.iter().zip(finals) {
            self.encode_level(&pass, output, [0, 0], images, cluster, 0, surfaces);
        }
        pass.endEncoding();
    }

    /// Contracts `cluster`'s probe rasters down to the three images its
    /// slot-0 resolve reads — one level per further slot, highest first,
    /// each in passes of its own into intermediate images covering the
    /// cluster's region.
    fn encode_levels(
        &mut self,
        cluster: &ClusterImages,
        base: &Placed,
        surfaces: &[RenderedSurface],
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    ) -> [Placed; 3] {
        let region = cluster.region;
        let origin = [region.x, region.y];
        let mut images: Vec<Placed> = std::iter::once(base.clone())
            .chain(cluster.probes.iter().map(|texture| Placed {
                texture: texture.clone(),
                origin,
                color: base.color,
            }))
            .collect();
        assert_eq!(
            images.len(),
            3_usize.pow(u32::try_from(cluster.slots).expect("a cluster's slot count fits u32")),
            "a cluster carries a raster for every combination of its probe states"
        );
        for slot in (1..cluster.slots).rev() {
            let stride = images.len() / 3;
            let outputs = self.scratch_textures(region.width, region.height, stride);
            for (prefix, output) in outputs.iter().enumerate() {
                let pass = begin_pass(command_buffer, output, MTLLoadAction::DontCare);
                let triple = [
                    images[prefix].clone(),
                    images[prefix + stride].clone(),
                    images[prefix + 2 * stride].clone(),
                ];
                self.encode_level(&pass, output, origin, &triple, cluster, slot, surfaces);
                pass.endEncoding();
            }
            images = outputs
                .into_iter()
                .map(|texture| Placed {
                    texture,
                    origin,
                    color: BLEND_COLOR,
                })
                .collect();
        }
        let [transparent, black, white]: [Placed; 3] = images
            .try_into()
            .expect("the last contraction leaves one image per slot-0 probe state");
        [transparent, black, white]
    }

    /// Draws one resolve level into `output`, the pass's attachment, whose
    /// pixel (0, 0) is capture-target pixel `output_origin`: over every
    /// member's rect, `images[0]` — the level's transparent-state image —
    /// then over each `slot` member's rect, the member's texture resolved
    /// through the level's three images. The result stores
    /// [`BLEND_COLOR`].
    #[expect(
        clippy::too_many_arguments,
        reason = "one level's whole input: its pass, attachment, placement, images, cluster and slot"
    )]
    fn encode_level(
        &mut self,
        encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
        output: &ProtocolObject<dyn MTLTexture>,
        output_origin: [usize; 2],
        images: &[Placed; 3],
        cluster: &ClusterImages,
        slot: usize,
        surfaces: &[RenderedSurface],
    ) {
        let [transparent, black, white] = images;
        assert!(
            transparent.color == black.color && black.color == white.color,
            "a resolve level's images sample alike"
        );
        assert_eq!(
            black.origin, white.origin,
            "a resolve level's probe images cover one region"
        );
        encoder.setRenderPipelineState(&self.resolve_pipeline(output.pixelFormat()));
        let sampler = self.composite_sampler();
        CompositeConversion::between(transparent.color, BLEND_COLOR).bind(encoder, 0);
        // SAFETY: `encoder` is a live render encoder; slots 1–3 are the
        // level images and sampler 0 the surface sampler
        // `capture_resolve_fragment` binds.
        unsafe {
            encoder.setFragmentTexture_atIndex(Some(&transparent.texture), 1);
            encoder.setFragmentTexture_atIndex(Some(&black.texture), 2);
            encoder.setFragmentTexture_atIndex(Some(&white.texture), 3);
            encoder.setFragmentSamplerState_atIndex(Some(&sampler), 0);
        }
        let scissor = |spec: &SurfaceSpec| MTLScissorRect {
            x: spec.clip.x - output_origin[0],
            y: spec.clip.y - output_origin[1],
            width: spec.clip.width,
            height: spec.clip.height,
        };
        let params = |spec: Option<&SurfaceSpec>| ResolveParams {
            output_origin: output_origin.map(pixel_offset),
            x0_origin: transparent.origin.map(pixel_offset),
            probe_origin: black.origin.map(pixel_offset),
            uv_offset: spec.map_or([0.0; 2], |spec| spec.uv_offset),
            uv_dx: spec.map_or([0.0; 2], |spec| spec.uv_dx),
            uv_dy: spec.map_or([0.0; 2], |spec| spec.uv_dy),
            has_surface: u32::from(spec.is_some()),
        };
        // The copies: a member's rect outside every `slot` member still
        // carries the transparent state of this level.
        for member in &cluster.members {
            let spec = &surfaces[member.surface].spec;
            encoder.setScissorRect(scissor(spec));
            // A copy reads no surface; the transparent image stands in
            // the surface slot so every declared texture is bound.
            draw_resolve(encoder, &params(None), &transparent.texture, None);
        }
        for member in cluster.members.iter().filter(|member| member.slot == slot) {
            let surface = &surfaces[member.surface];
            encoder.setScissorRect(scissor(&surface.spec));
            draw_resolve(
                encoder,
                &params(Some(&surface.spec)),
                &surface.texture,
                Some(CompositeConversion::between(
                    shader_color(surface.spec.pixel_format),
                    BLEND_COLOR,
                )),
            );
        }
    }
}

/// One `capture_resolve_fragment` draw over the encoder's scissor: `params`
/// at fragment buffer 1, `surface` at texture 0 and its conversion — the
/// transparent-state conversion's stand-in for a copy — at buffer 2.
fn draw_resolve(
    encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    params: &ResolveParams,
    surface: &ProtocolObject<dyn MTLTexture>,
    conversion: Option<CompositeConversion>,
) {
    let conversion =
        conversion.unwrap_or_else(|| CompositeConversion::between(BLEND_COLOR, BLEND_COLOR));
    conversion.bind(encoder, 2);
    // SAFETY: `encoder` is a live render encoder; buffer 1 and texture 0
    // are the parameter and surface slots the resolve fragment binds, and
    // `setFragmentBytes` copies `params` into the command.
    unsafe {
        encoder.setFragmentBytes_length_atIndex(
            std::ptr::NonNull::from(params).cast::<core::ffi::c_void>(),
            core::mem::size_of::<ResolveParams>(),
            1,
        );
        encoder.setFragmentTexture_atIndex(Some(surface), 0);
        encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
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
    /// What the texture samples as — [`raster_color`] of the format.
    color: ShaderColor,
}

/// The colour space the native raster draws a `pixel_format` destination
/// in, and what its texture then samples as.
///
/// A `CGContext` blends in its own colour space, and Core Animation
/// composites a window's layers on their sRGB-encoded values. A raster in
/// a linear space would blend every translucent layer, antialiased edge and
/// shadow in linear light — lighter than the window shows it (#3528) — so
/// the raster always draws in an encoded space: sRGB as [`color_space`]
/// tags an 8-bit destination, and [`BLEND_COLOR`] — extended sRGB — for
/// the half-float one, whose extended range keeps wide-gamut content. The
/// composite pass then converts the raster into its blend colour like
/// every other source.
///
/// [`color_space`]: crate::metal::color_space
fn raster_color(pixel_format: MTLPixelFormat) -> (CFRetained<CGColorSpace>, ShaderColor) {
    if pixel_format == BLEND_FORMAT {
        // SAFETY: the colorspace statics are system constants.
        let name = unsafe { kCGColorSpaceExtendedSRGB };
        let space = CGColorSpace::with_name(Some(name))
            .expect("could not create the extended sRGB raster color space");
        (space, BLEND_COLOR)
    } else {
        (
            crate::metal::color_space(pixel_format),
            shader_color(pixel_format),
        )
    }
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
        let (color_space, color) = raster_color(pixel_format);
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
            color,
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

    /// Rasterizes `layer`'s tree into the shared buffer through
    /// `transform` — [`raster_transform`] for the whole capture, or that
    /// shifted onto a region — a synchronous CPU draw, completed when it
    /// returns.
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
    fn draw(&self, layer: &CALayer, transform: CGAffineTransform) {
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
        CGContext::concat_ctm(Some(context), transform);
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

    /// What [`texture`](Self::texture) samples as.
    const fn color(&self) -> ShaderColor {
        self.frame.as_ref().expect("a lease owns its frame").color
    }

    /// Encodes the blit transfer into the composition command buffer —
    /// see [`NativeRasterFrame::encode_transfer`].
    fn encode_transfer(&self, command_buffer: &ProtocolObject<dyn MTLCommandBuffer>) {
        self.frame
            .as_ref()
            .expect("a lease owns its frame")
            .encode_transfer(command_buffer);
    }

    /// Rasterizes `layer`'s tree into the leased destination through
    /// `transform` — the draw half of a capture, infallible once the frame
    /// exists.
    fn draw(&self, layer: &CALayer, transform: CGAffineTransform) {
        self.frame
            .as_ref()
            .expect("a lease owns its frame")
            .draw(layer, transform);
    }

    /// Hands the frame back — only the settle path may call this.
    fn into_frame(mut self) -> NativeRasterFrame {
        self.frame.take().expect("a lease owns its frame")
    }
}

/// The key a raster pool is bound to: the caller's generation token plus
/// the destination's device and pixel format. Any key change retires the
/// whole pool; within a key, frames pool by pixel size.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RasterKey {
    generation: u64,
    device: *const std::ffi::c_void,
    pixel_format: MTLPixelFormat,
}

impl fmt::Debug for RasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RasterKey")
            .field("generation", &self.generation)
            .field("device", &self.device)
            .field("pixel_format", &self.pixel_format)
            .finish()
    }
}

/// The native raster half of a capture, confined to the main thread: a
/// small pool of `NativeRasterFrame`s leased per capture — the base raster
/// and every probe raster — and returned when the GPU settles, all under
/// one current [`RasterKey`] and the sizes the latest capture asked for.
/// It represents one current capture destination, not a cache of every
/// geometry it has ever seen.
#[derive(Debug, Default)]
struct NativeRenderer {
    /// The key the pool currently issues for. Any mismatch drains
    /// `available`: storage from another key — an old generation, a
    /// replaced device, a different format — can never answer this key's
    /// capture.
    key: Option<RasterKey>,
    /// The pixel sizes the latest capture issued — a settled frame of any
    /// other size is dropped, not pooled.
    sizes: Vec<(usize, usize)>,
    /// Settled frames under `key`, by pixel size, ready to reissue.
    available: HashMap<(usize, usize), Vec<NativeRasterFrame>>,
}

impl NativeRenderer {
    /// Binds the pool to the (`generation`, `device`, `format`) key of a
    /// capture that issues frames of `sizes` — dropping pooled frames of
    /// another key or of a size this capture does not use.
    fn begin(
        &mut self,
        device: &ProtocolObject<dyn MTLDevice>,
        format: MTLPixelFormat,
        generation: u64,
        sizes: Vec<(usize, usize)>,
    ) {
        let key = RasterKey {
            generation,
            device: core::ptr::from_ref(device).cast(),
            pixel_format: format,
        };
        if self.key != Some(key) {
            self.key = Some(key);
            self.available.clear();
        }
        self.available.retain(|size, _| sizes.contains(size));
        self.sizes = sizes;
    }

    /// Issues a `width` × `height` frame under the bound key and returns
    /// the lease owning it through consumption.
    ///
    /// # Panics
    ///
    /// When no capture bound the pool ([`begin`](Self::begin)), the size
    /// is not one it declared, or the frame cannot be created (see
    /// [`NativeRasterFrame::new`]).
    fn issue(
        &mut self,
        device: &ProtocolObject<dyn MTLDevice>,
        width: usize,
        height: usize,
    ) -> RasterLease {
        let key = self
            .key
            .expect("a raster pool issues only inside a capture");
        assert!(
            self.sizes.contains(&(width, height)),
            "a capture issues only the raster sizes it declared"
        );
        assert_eq!(
            key.device,
            core::ptr::from_ref(device).cast(),
            "a capture issues on the device it bound"
        );
        let frame = self
            .available
            .get_mut(&(width, height))
            .and_then(Vec::pop)
            .unwrap_or_else(|| {
                NativeRasterFrame::new(device, key.pixel_format, width, height, key.generation)
            });
        RasterLease { frame: Some(frame) }
    }

    /// Takes a settled frame back — main thread only, called from the
    /// compositor's completion path once the GPU stopped sampling it. A
    /// frame whose key or size no longer matches the pool's current key
    /// and sizes is dropped instead: outstanding old-key storage never
    /// joins the new pool. A frame settling after the pool was reset
    /// (shutdown) is dropped without re-arming the pool — an outstanding
    /// capture must never resurrect lifetime the owner already ended.
    fn return_frame(&mut self, frame: NativeRasterFrame) {
        let Some(key) = &self.key else {
            return;
        };
        let size = (frame.pixel_width, frame.pixel_height);
        if frame.generation == key.generation
            && frame.pixel_format == key.pixel_format
            && Retained::as_ptr(&frame.texture.device()) == key.device.cast()
            && self.sizes.contains(&size)
        {
            self.available.entry(size).or_default().push(frame);
        }
    }
}

/// A snapshot's parts: the platform spec, the surface it came from and
/// the presentation layer its texture stands for.
struct CapturedSnapshot {
    spec: SurfaceSpec,
    surface: Rc<dyn CapturableSurface>,
    layer: Retained<CALayer>,
}

/// Whether two destination rects share a pixel.
const fn overlaps(a: MTLScissorRect, b: MTLScissorRect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// One overlap cluster as the capture plans it — see [`ClusterImages`].
#[derive(Clone, Debug)]
struct ClusterPlan {
    region: MTLScissorRect,
    slots: usize,
    members: Vec<ClusterMember>,
}

impl ClusterPlan {
    /// How many probe rasters the cluster needs: every combination of its
    /// slots' three probe states but the all-transparent one.
    fn probe_count(&self) -> usize {
        3_usize.pow(u32::try_from(self.slots).expect("a cluster's slot count fits u32")) - 1
    }

    /// The probe state `slot` takes in probe raster `index`.
    fn state(index: usize, slot: usize) -> ProbeState {
        let combination = index + 1;
        let digit =
            combination / 3_usize.pow(u32::try_from(slot).expect("a probe slot fits u32")) % 3;
        match digit {
            0 => ProbeState::Transparent,
            1 => ProbeState::Black,
            _ => ProbeState::White,
        }
    }
}

/// What a probe layer shows while one raster is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeState {
    /// Hidden — the surface contributes nothing.
    Transparent,
    /// Opaque black.
    Black,
    /// Opaque white.
    White,
}

/// The union-find root of `index`, compressing the path on the way.
const fn cluster_root(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}

/// The overlap clusters of `specs`, in paint order: surfaces whose
/// destination rects overlap — directly or through a chain — share a
/// cluster, and each member takes the lowest probe slot no overlapping
/// earlier member holds. Members are indices into `specs`.
fn plan_clusters(specs: &[SurfaceSpec]) -> Vec<ClusterPlan> {
    // Union-find over the overlap graph — a capture holds a handful of
    // surfaces, so the pairwise scan is the whole cost.
    let mut parent: Vec<usize> = (0..specs.len()).collect();
    for a in 0..specs.len() {
        for b in a + 1..specs.len() {
            if overlaps(specs[a].clip, specs[b].clip) {
                let (ra, rb) = (cluster_root(&mut parent, a), cluster_root(&mut parent, b));
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut clusters: Vec<(usize, ClusterPlan)> = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        let root = cluster_root(&mut parent, index);
        let clip = spec.clip;
        let existing = clusters.iter().position(|(r, _)| *r == root);
        let position = existing.unwrap_or_else(|| {
            clusters.push((
                root,
                ClusterPlan {
                    region: clip,
                    slots: 0,
                    members: Vec::new(),
                },
            ));
            clusters.len() - 1
        });
        let plan = &mut clusters[position].1;
        let taken: Vec<usize> = plan
            .members
            .iter()
            .filter(|member| overlaps(specs[member.surface].clip, clip))
            .map(|member| member.slot)
            .collect();
        // `taken` holds fewer slots than `taken.len() + 1`, so one of
        // those is free.
        let slot = (0..=taken.len())
            .find(|slot| !taken.contains(slot))
            .expect("a free slot exists");
        let region = plan.region;
        let (left, top) = (region.x.min(clip.x), region.y.min(clip.y));
        let right = (region.x + region.width).max(clip.x + clip.width);
        let bottom = (region.y + region.height).max(clip.y + clip.height);
        plan.region = MTLScissorRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        };
        plan.slots = plan.slots.max(slot + 1);
        plan.members.push(ClusterMember {
            slot,
            surface: index,
        });
    }
    clusters.into_iter().map(|(_, plan)| plan).collect()
}

/// The raster transform of `geometry` shifted onto `region` of a
/// `height`-pixel-tall capture: the region's top-left texel lands on the
/// probe raster's texel (0, 0).
fn region_transform(
    geometry: CaptureGeometry,
    height: f64,
    region: MTLScissorRect,
) -> CGAffineTransform {
    let mut transform = raster_transform(geometry, height);
    // The bitmap context is bottom-up: the region's bottom edge, at
    // capture texel row `y + height`, is the probe raster's context y = 0.
    transform.tx -= pixel_extent(region.x);
    transform.ty -= height - pixel_extent(region.y + region.height);
    transform
}

/// The probe layers of one capture's rasters, one per snapshot whose
/// presentation layer is visible and attached: a plain layer with the
/// presentation layer's geometry and opacity, inserted directly above it
/// in its superlayer. Model-only mutations inside the capture's disabled
/// transaction; `Drop` removes every probe, so the render server never
/// sees one.
struct ProbeLayers {
    layers: Vec<Option<Retained<CALayer>>>,
    black: CFRetained<CGColor>,
    white: CFRetained<CGColor>,
}

impl ProbeLayers {
    /// Inserts a hidden probe beside each snapshot whose `visible` entry
    /// is set — the presentation layer's visibility before suppression
    /// hid it; a hidden surface shows nothing on screen, so it gets no
    /// probe and resolves to the native tree alone. The probe colours are
    /// made in `space`, the rasters' own colour space, so they reach the
    /// raster unconverted.
    fn insert(snapshots: &[CapturedSnapshot], visible: &[bool], space: &CGColorSpace) -> Self {
        let color = |value: f64| {
            let components = [value, value, value, 1.0];
            // SAFETY: `components` holds the space's three channels plus
            // alpha, and lives across the call.
            unsafe { CGColor::new(Some(space), components.as_ptr()) }
                .expect("could not create a capture probe colour")
        };
        let layers = snapshots
            .iter()
            .zip(visible)
            .map(|(snapshot, &visible)| {
                let presentation = &snapshot.layer;
                let superlayer = presentation.superlayer()?;
                visible.then(|| {
                    let probe = CALayer::new();
                    probe.setBounds(presentation.bounds());
                    probe.setPosition(presentation.position());
                    probe.setAnchorPoint(presentation.anchorPoint());
                    probe.setAnchorPointZ(presentation.anchorPointZ());
                    probe.setTransform(presentation.transform());
                    probe.setZPosition(presentation.zPosition());
                    probe.setOpacity(presentation.opacity());
                    probe.setHidden(true);
                    superlayer.insertSublayer_above(&probe, Some(presentation));
                    probe
                })
            })
            .collect();
        Self {
            layers,
            black: color(0.0),
            white: color(1.0),
        }
    }

    /// Shows snapshot `index`'s probe in `state` — a no-op for a snapshot
    /// with no probe.
    fn set(&self, index: usize, state: ProbeState) {
        let Some(probe) = &self.layers[index] else {
            return;
        };
        match state {
            ProbeState::Transparent => probe.setHidden(true),
            ProbeState::Black => {
                probe.setBackgroundColor(Some(&self.black));
                probe.setHidden(false);
            }
            ProbeState::White => {
                probe.setBackgroundColor(Some(&self.white));
                probe.setHidden(false);
            }
        }
    }
}

impl Drop for ProbeLayers {
    fn drop(&mut self) {
        for probe in self.layers.iter().flatten() {
            probe.removeFromSuperlayer();
        }
    }
}

/// One cluster's probe rasters, leased with the capture.
struct PreparedCluster {
    plan: ClusterPlan,
    probes: Vec<RasterLease>,
}

impl PreparedCluster {
    /// Rasterizes `root`'s tree into each probe raster through
    /// `transform` with the members' probes in that raster's states, and
    /// hides them again.
    fn draw(&self, root: &CALayer, probes: &ProbeLayers, transform: CGAffineTransform) {
        for (index, lease) in self.probes.iter().enumerate() {
            for member in &self.plan.members {
                probes.set(member.surface, ClusterPlan::state(index, member.slot));
            }
            lease.draw(root, transform);
        }
        for member in &self.plan.members {
            probes.set(member.surface, ProbeState::Transparent);
        }
    }
}

/// A resolved surface the capture joins without a spec — its id and the
/// surface it came from. Kept for redraw routing only: it renders
/// nothing this pass, so it enters `active` but never `Preparation`.
struct CapturedSurface {
    surface_id: usize,
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
    /// The overlap clusters and their probe rasters — leased like `raster`
    /// until the GPU stops sampling them.
    clusters: Vec<PreparedCluster>,
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

/// The one subtree walk the capture paths share.
///
/// `visit` receives each resolved capturable surface and the view
/// carrying it; a resolved view stops the descent — its own subtree
/// renders through its producer, and its readiness covers them.
pub fn for_each_capturable(
    view: &PlatformView,
    visit: &mut impl FnMut(&PlatformView, &Rc<dyn CapturableSurface>),
) {
    if let Some(surface) = resolve_capturable(view) {
        visit(view, &surface);
        return;
    }
    for subview in crate::view::subviews(view) {
        for_each_capturable(&subview, visit);
    }
}

/// Every capturable surface inside `root`'s subtree — `f` receives each
/// live surface.
pub fn collect_capturables(root: &PlatformView, f: &mut impl FnMut(&Rc<dyn CapturableSurface>)) {
    for_each_capturable(root, &mut |_, surface| f(surface));
}

/// The view's capturable slot, reached through the same downcast
/// [`resolve_capturable`] performs — `None` for a view that can never
/// carry a surface.
pub fn capturable_slot(view: &PlatformView) -> Option<&CapturableSlot> {
    #[cfg(target_os = "macos")]
    {
        use crate::appkit::{HostView, surface_view::SurfaceView};
        if let Some(view) = view.downcast_ref::<SurfaceView>() {
            return Some(view.capturable_slot());
        }
        if let Some(view) = view.downcast_ref::<HostView>() {
            return Some(view.capturable_slot());
        }
    }
    #[cfg(target_os = "ios")]
    {
        use crate::uikit::{HostView, surface_view::SurfaceView};
        if let Some(view) = view.downcast_ref::<SurfaceView>() {
            return Some(view.capturable_slot());
        }
        if let Some(view) = view.downcast_ref::<HostView>() {
            return Some(view.capturable_slot());
        }
    }
    let _ = view;
    None
}

/// The frame an armed surface last put on screen, handed to the
/// retirement walk as an immutable GPU input — private storage, the
/// presentation format, and the context generation it was made under.
#[derive(Clone)]
pub struct FrozenSurface {
    /// The frozen pixels — a GPU→GPU copy of the last presented frame;
    /// nothing ever writes to it again.
    pub texture: Retained<ProtocolObject<dyn MTLTexture>>,
    /// The surface's presentation pixel format.
    pub pixel_format: MTLPixelFormat,
    /// The context generation the texture belongs to.
    pub generation: u64,
}

impl fmt::Debug for FrozenSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenSurface")
            .field("pixel_format", &self.pixel_format)
            .field("generation", &self.generation)
            .field("size", &(self.texture.width(), self.texture.height()))
            .finish()
    }
}

/// An immutable surface: the last frame a retired surface put on screen,
/// answering real capture requests by blitting its frozen texture — or
/// clearing to transparent when the surface never presented.
///
/// It never redraws: suppression hides the presentation layer,
/// `can_present` is always `false`, and a frozen producer never requests
/// a redraw, so external-rendering redirection is a no-op.
///
/// Installed by a transition's retirement into the surface view's
/// capturable slot so an enclosing live filter keeps resolving the
/// ghost's surfaces — and the ghost keeps showing each layer's last
/// contents natively. It references no render state and no renderer.
pub struct FrozenCapturable {
    /// The surface's presentation `CAMetalLayer` — hidden while a
    /// capture suppresses this producer.
    layer: Retained<CAMetalLayer>,
    /// The last presented frame — `None` when the surface never landed
    /// one, answered as a transparent clear.
    frame: Option<FrozenSurface>,
    /// The serial queue the blit/composition submissions run on.
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl fmt::Debug for FrozenCapturable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenCapturable")
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl FrozenCapturable {
    /// The frozen stand-in for a surface view's just-released capturable:
    /// `presentation_layer` is the surface's own `CAMetalLayer`,
    /// `frame` its last presented frame, `device` the context's device —
    /// the queue a capture submission runs on is created from it.
    ///
    /// # Panics
    ///
    /// When `device` cannot create a command queue.
    #[must_use]
    pub fn new(
        presentation_layer: Retained<CAMetalLayer>,
        frame: Option<FrozenSurface>,
        device: &ProtocolObject<dyn MTLDevice>,
    ) -> Rc<Self> {
        Rc::new(Self {
            layer: presentation_layer,
            frame,
            queue: device
                .newCommandQueue()
                .expect("failed to create the frozen surface command queue"),
        })
    }
}

impl CapturableSurface for FrozenCapturable {
    fn capture_pixel_format(&self) -> MTLPixelFormat {
        self.frame
            .as_ref()
            .map_or_else(|| self.layer.pixelFormat(), |frame| frame.pixel_format)
    }

    fn presentation_layer(&self) -> Option<Retained<CALayer>> {
        Some(self.layer.clone().into_super())
    }

    fn begin_capture_suppression(&self) {
        self.layer.setHidden(true);
    }

    fn end_capture_suppression(&self) {
        self.layer.setHidden(false);
    }

    /// A frozen producer has no presenter — nothing ever puts its frame
    /// in front of the user again.
    fn can_present(&self) -> bool {
        false
    }

    /// A frozen producer never requests a redraw — there is nothing to
    /// redirect.
    fn begin_external_rendering(&self, _on_redraw: Rc<dyn Fn()>) {}

    fn end_external_rendering(&self, _resume: bool) {}

    fn prepare_external_render(&self, _texture: &ProtocolObject<dyn MTLTexture>) -> bool {
        true
    }

    /// Blits the frozen texture into `texture` — or clears it to
    /// transparent when the surface never presented — on this surface's
    /// queue; the buffer's completed status answers the completion on
    /// the main queue.
    fn render_prepared_external_texture(
        &self,
        texture: &ProtocolObject<dyn MTLTexture>,
        _width: u32,
        _height: u32,
        completion: SurfaceCaptureCompletion,
    ) {
        let Some(command_buffer) = self.queue.commandBuffer() else {
            completion(Err(CaptureError::Failed(Arc::new(CompositionFailed {
                status: MTLCommandBufferStatus::NotEnqueued,
                error: Some("the frozen surface could not create a command buffer".into()),
            }))));
            return;
        };
        if let Some(frame) = &self.frame {
            // A GPU→GPU copy on the surface's own queue — the frozen
            // pixels are the whole frame; a target larger than it keeps
            // the rest of its prepared contents.
            let encoder = command_buffer
                .blitCommandEncoder()
                .expect("failed to create the frozen surface blit encoder");
            // SAFETY: both textures are valid private-storage 2D
            // targets retained past command completion; the copy
            // region is the smaller of the two extents.
            unsafe {
                encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                    &frame.texture,
                    0,
                    0,
                    MTLOrigin { x: 0, y: 0, z: 0 },
                    MTLSize {
                        width: frame.texture.width().min(texture.width() as _),
                        height: frame.texture.height().min(texture.height() as _),
                        depth: 1,
                    },
                    texture,
                    0,
                    0,
                    MTLOrigin { x: 0, y: 0, z: 0 },
                );
            }
            encoder.endEncoding();
        } else {
            // The surface never presented — the region answers
            // transparent, like a frame with nothing drawn.
            let descriptor = MTLRenderPassDescriptor::new();
            // SAFETY: index 0 is the only color attachment.
            let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
            attachment.setTexture(Some(texture));
            attachment.setLoadAction(MTLLoadAction::Clear);
            attachment.setStoreAction(MTLStoreAction::Store);
            attachment.setClearColor(MTLClearColor {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 0.0,
            });
            command_buffer
                .renderCommandEncoderWithDescriptor(&descriptor)
                .expect("failed to create the frozen surface clear encoder")
                .endEncoding();
        }
        let completion = Mutex::new(Some(completion));
        let handler = RcBlock::new(
            move |buffer: std::ptr::NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                // SAFETY: the buffer is alive for the handler call.
                let buffer = unsafe { buffer.as_ref() };
                let status = buffer.status();
                let error = buffer
                    .error()
                    .map(|error| error.localizedDescription().to_string());
                let completion = completion.lock().expect("frozen capture lock").take();
                if let Some(completion) = completion {
                    enqueue(move |_mtm| {
                        completion(composition_outcome(status, error));
                    });
                }
            },
        );
        // SAFETY: Metal copies the block.
        unsafe {
            command_buffer.addCompletedHandler(RcBlock::as_ptr(&handler));
        }
        command_buffer.commit();
    }

    /// `true` once the surface landed a frame before it retired.
    fn has_presented_frame(&self) -> bool {
        self.frame.is_some()
    }

    /// A retired producer owes no first-paint frame.
    fn participates_in_first_paint(&self) -> bool {
        false
    }

    /// Nothing is pending — the waker fires at once.
    fn register_ready_waiter(&self, waker: std::task::Waker) {
        waker.wake();
    }

    /// An immutable producer can no longer fail.
    fn presentation_failed(&self) -> bool {
        false
    }

    fn take_frozen_frame(&self) -> (Option<FrozenSurface>, Retained<CAMetalLayer>) {
        (self.frame.clone(), self.layer.clone())
    }
}

/// Whether `view`'s subtree holds a `CAMetalLayer` that the capture walk
/// did not resolve to a [`CapturableSurface`]. `renderInContext`
/// composites a Metal layer as nothing — `MKMapView` is the known case
/// (#2104) — so its pixels would vanish from the raster without a
/// signal. The walk stops at resolved surfaces because a surface view's
/// own Metal layer renders through its producer; each subview's layer
/// tree is scanned under that subview, so the layer scan prunes at
/// subview roots.
fn has_unresolved_metal_layer(view: &PlatformView) -> bool {
    if resolve_capturable(view).is_some() {
        return false;
    }
    let mut subview_layers = Vec::new();
    for subview in crate::view::subviews(view) {
        // A hidden or fully transparent view renders nothing under
        // `renderInContext` — the layers in its subtree are absent by
        // design, not unresolved, and the scan skips the branch whole
        // (the layer-level check covers layers hidden without a view).
        if crate::view::is_hidden(&subview) || crate::view::alpha(&subview) <= 0.0 {
            continue;
        }
        if let Some(layer) = crate::layer::layer_of(&subview) {
            subview_layers.push(layer);
        }
        if has_unresolved_metal_layer(&subview) {
            return true;
        }
    }
    crate::layer::layer_of(view).is_some_and(|layer| metal_in_layer_tree(&layer, &subview_layers))
}

/// Whether `layer` or a descendant sublayer is a `CAMetalLayer`; roots in
/// `pruned` — the layers owned by subviews — cut the descent. A hidden or
/// fully transparent layer renders nothing under `renderInContext`, so a
/// Metal layer inside one is absent by design, not unresolved.
fn metal_in_layer_tree(layer: &CALayer, pruned: &[Retained<CALayer>]) -> bool {
    if layer.isHidden() || layer.opacity() <= 0.0 {
        return false;
    }
    if layer.isKindOfClass(CAMetalLayer::class()) {
        return true;
    }
    // SAFETY: `layer` is a live layer on the main thread.
    let Some(sublayers) = (unsafe { layer.sublayers() }) else {
        return false;
    };
    sublayers.iter().any(|sublayer| {
        pruned
            .iter()
            .all(|root| Retained::as_ptr(root) != Retained::as_ptr(&sublayer))
            && metal_in_layer_tree(&sublayer, pruned)
    })
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
    ///
    /// A surface binds the hook when it joins and keeps it for its whole
    /// external-rendering scope, so the hook is fixed once one has joined:
    /// an owner whose redraw answer changes dispatches inside its hook.
    ///
    /// # Panics
    ///
    /// When a surface has already joined this capture — a replacement
    /// would never reach it, stranding its redraws on the old hook.
    pub fn set_on_redraw(&self, redraw: impl Fn() + 'static) {
        assert!(
            self.active.borrow().is_empty(),
            "a view capture's redraw hook is fixed once a surface has joined"
        );
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
        let (preparation, specs) = match self.prepare(target, generation) {
            Ok(prepared) => prepared,
            // A capture that cannot rasterize the subtree reports on the
            // main thread it was asked on — never a frame that silently
            // lacks the unrasterizable layers.
            Err(error) => {
                completion(Err(error));
                return;
            }
        };

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
    ) -> Result<(Preparation, Vec<SurfaceSpec>), CaptureError> {
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
        // suppression open → base draw → probe insertion → probe draws →
        // probe removal → suppression restore → re-hide → sole commit. A normally-hidden filter-owned root is un-hidden
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
            // A native Metal layer the walk cannot resolve — `MKMapView`
            // (#2104) — rasterizes to nothing under `renderInContext`.
            // Fail the capture here, after layout so the mounted tree is
            // final and before any surface mutation, rather than hand
            // back a frame missing those pixels.
            if has_unresolved_metal_layer(content) {
                return Err(CaptureError::Failed(Arc::new(UnresolvedMetalLayer)));
            }
            let layer = crate::view::layer(content).expect("a capture view must be layer-backed");
            let geometry = CaptureGeometry::new(
                crate::view::bounds(content),
                target.width(),
                target.height(),
                crate::view::is_flipped(content),
            );
            let (snapshots, unpresentable) = self.collect_snapshots(target, &layer, geometry);
            self.update_external_surfaces(&snapshots, &unpresentable);
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
            let specs: Vec<SurfaceSpec> = snapshots.iter().map(|s| s.spec).collect();
            let plans = plan_clusters(&specs);
            // Fallible allocation still precedes suppression: an
            // allocation panic leaves suppression untouched and the
            // guards still restore what they opened.
            let device = target.device();
            let (raster, clusters) = {
                let mut renderer = self.renderer.borrow_mut();
                let full = (target.width(), target.height());
                let sizes = std::iter::once(full)
                    .chain(
                        plans
                            .iter()
                            .map(|plan| (plan.region.width, plan.region.height)),
                    )
                    .collect();
                renderer.begin(&device, target.pixelFormat(), generation, sizes);
                let raster = renderer.issue(&device, full.0, full.1);
                let clusters: Vec<PreparedCluster> = plans
                    .into_iter()
                    .map(|plan| PreparedCluster {
                        probes: (0..plan.probe_count())
                            .map(|_| renderer.issue(&device, plan.region.width, plan.region.height))
                            .collect(),
                        plan,
                    })
                    .collect();
                (raster, clusters)
            };
            let (probe_space, _) = raster_color(target.pixelFormat());
            // Read before suppression hides every presentation layer.
            let visible: Vec<bool> = snapshots.iter().map(|s| !s.layer.isHidden()).collect();
            let height = pixel_extent(target.height());
            {
                // Declared in drop order: the probes leave the tree
                // before suppression ends.
                let mut suppression = SuppressionGuard::new(&snapshots);
                suppression.begin();
                raster.draw(&layer, raster_transform(geometry, height));
                let probes = ProbeLayers::insert(&snapshots, &visible, &probe_space);
                for cluster in &clusters {
                    cluster.draw(
                        &layer,
                        &probes,
                        region_transform(geometry, height, cluster.plan.region),
                    );
                }
                drop(probes);
                suppression.end();
            }
            let preparation = Preparation {
                raster,
                clusters,
                target: target.retain(),
                device,
                surfaces,
            };
            // Re-hide before the sole commit — `restore` would do it on
            // drop, but the restore must be explicit before commit.
            drop(restore);
            transaction.commit();
            flush_transaction();
            (preparation, specs)
        };
        Ok((preparation, specs))
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

    /// The surface collection for one capture: the rendered snapshots
    /// plus the surfaces the capture must still join for redraw routing —
    /// resolved capturables whose bounds produced no spec this pass and
    /// that cannot present on their own. A windowless capture's zero-size
    /// image surface is the case: its redraws would otherwise die on a
    /// presentation path that never runs, so it joins the scope like a
    /// spec'd surface and wakes the capture's drive.
    ///
    /// Snapshots are in paint order — the walk visits subviews back to
    /// front — and each carries the presentation layer its spec was
    /// mapped from, through `root`, the captured view's own layer.
    fn collect_snapshots(
        &self,
        target: &ProtocolObject<dyn MTLTexture>,
        root: &CALayer,
        geometry: CaptureGeometry,
    ) -> (Vec<CapturedSnapshot>, Vec<CapturedSurface>) {
        let mut snapshots = Vec::new();
        let mut unpresentable = Vec::new();
        let (target_width, target_height) = (target.width(), target.height());
        for_each_capturable(&self.content, &mut |view, surface| {
            let layer = surface.presentation_layer();
            let spec = layer.as_ref().and_then(|layer| {
                surface_spec(
                    view_key(view),
                    surface_quad(layer, root, geometry),
                    || surface.capture_pixel_format(),
                    target_width,
                    target_height,
                )
            });
            if let (Some(spec), Some(layer)) = (spec, layer) {
                snapshots.push(CapturedSnapshot {
                    spec,
                    surface: surface.clone(),
                    layer,
                });
            } else if !surface.can_present() {
                unpresentable.push(CapturedSurface {
                    surface_id: view_key(view),
                    surface: surface.clone(),
                });
            }
        });
        (snapshots, unpresentable)
    }

    /// Joins and parts external surfaces against this capture's collection:
    /// the spec'd `snapshots` that render into the target and the
    /// `unpresentable` surfaces kept only for their redraw routing.
    ///
    /// # Panics
    ///
    /// When [`set_on_redraw`](Self::set_on_redraw) was never called.
    fn update_external_surfaces(
        &self,
        snapshots: &[CapturedSnapshot],
        unpresentable: &[CapturedSurface],
    ) {
        let on_redraw = self
            .on_redraw
            .borrow()
            .clone()
            .expect("a view capture's redraw hook must be installed before capture");
        let mut active = self.active.borrow_mut();
        // One pass over the collection: the next registration set is built
        // keyed by surface id — surviving surfaces keep their `Rc` lease,
        // parted ones drop out (their registration ends when the last
        // owner, map or outstanding capture, drops on this thread), and
        // each new surface is begun exactly once. No per-surface scan of
        // the collection, so nested-GPU-child scenes stay linear.
        let join = |id, surface: &Rc<dyn CapturableSurface>| {
            active.get(&id).cloned().unwrap_or_else(|| {
                surface.begin_external_rendering(on_redraw.clone());
                Rc::new(SurfaceRegistration {
                    surface: surface.clone(),
                })
            })
        };
        let next: HashMap<usize, Rc<SurfaceRegistration>> = snapshots
            .iter()
            .map(|s| (s.spec.surface_id, join(s.spec.surface_id, &s.surface)))
            .chain(
                unpresentable
                    .iter()
                    .map(|s| (s.surface_id, join(s.surface_id, &s.surface))),
            )
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
            let prepared = preparation.get();
            let command_buffer = guard.make_command_buffer(&prepared.device);
            // The CPU-drawn rasters reach the compositor's private
            // textures through blits on this same command buffer — each
            // encoder is ended before the render passes sample them.
            prepared.raster.encode_transfer(&command_buffer);
            for cluster in &prepared.clusters {
                for probe in &cluster.probes {
                    probe.encode_transfer(&command_buffer);
                }
            }
            let base = Placed {
                texture: prepared.raster.texture().clone(),
                origin: [0, 0],
                color: prepared.raster.color(),
            };
            let clusters: Vec<ClusterImages> = prepared
                .clusters
                .iter()
                .map(|cluster| ClusterImages {
                    region: cluster.plan.region,
                    slots: cluster.plan.slots,
                    probes: cluster
                        .probes
                        .iter()
                        .map(|probe| probe.texture().clone())
                        .collect(),
                    members: cluster.plan.members.clone(),
                })
                .collect();
            guard.encode_composition(
                rendered.get(),
                &base,
                &clusters,
                &prepared.target,
                &command_buffer,
                &prepared.device,
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
                            // The registrations and target bound here
                            // drop after the completion, as the whole
                            // preparation did.
                            let Preparation {
                                raster,
                                clusters,
                                target: _target,
                                device: _device,
                                surfaces: _surfaces,
                            } = preparation.0;
                            let outcome = composition_outcome(status, error);
                            if outcome.is_ok()
                                && let Some(capture) = return_to.get(mtm).upgrade()
                            {
                                let mut pool = capture.renderer.borrow_mut();
                                pool.return_frame(raster.into_frame());
                                for probe in clusters.into_iter().flat_map(|c| c.probes) {
                                    pool.return_frame(probe.into_frame());
                                }
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

/// The captured subtree holds a native `CAMetalLayer` that no
/// [`CapturableSurface`] claimed — `renderInContext` draws it as
/// nothing, so its pixels never reach the frame.
///
/// `MKMapView` is the known producer (#2104); capturing real map content
/// needs a `MapKit` channel such as `MKMapSnapshotter` or a
/// capture-resolvable surface, which is a design decision this type makes
/// visible instead of silently omitting the layer.
#[derive(Debug)]
pub struct UnresolvedMetalLayer;

impl fmt::Display for UnresolvedMetalLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "the captured subtree contains a native Metal layer that no \
             capturable surface claimed; renderInContext cannot draw it",
        )
    }
}

impl std::error::Error for UnresolvedMetalLayer {}

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
        MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
        MTLCopyAllDevices, MTLCreateSystemDefaultDevice, MTLDevice, MTLLoadAction, MTLOrigin,
        MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder,
        MTLRenderPassDescriptor, MTLResource, MTLScissorRect, MTLSize, MTLStoreAction, MTLTexture,
        MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
    };

    use super::{
        BLEND_COLOR, CaptureError, ClusterImages, CompositeConversion, CompositeRegion,
        CompositionFailed, CompositorGuard, CompositorState, DeviceResources, FenceBatch,
        NativeRenderer, Placed, RenderedSurface, SurfaceSpec, UnresolvedMetalLayer,
        composition_outcome, metal_in_layer_tree, plan_clusters, surface_spec,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// The unresolved-Metal-layer scan finds a `CAMetalLayer` anywhere in
    /// a layer tree — including through subclassing and nesting — and a
    /// pruned root (the layer a resolved surface view owns, or any
    /// subview's own layer, which the view-level walk scans separately)
    /// cuts the descent so nothing under it counts twice (#2104).
    #[test]
    fn a_metal_layer_is_detected_unless_it_is_a_pruned_root() {
        use objc2_quartz_core::{CALayer, CAMetalLayer};
        let metal = CAMetalLayer::new();
        let host = CALayer::new();
        host.addSublayer(&metal);
        assert!(metal_in_layer_tree(&host, &[]));
        assert!(metal_in_layer_tree(&metal, &[]));
        // Pruning at the subview's layer hides the Metal root and
        // anything nested under it — the resolved surface's own case.
        let nested = CALayer::new();
        nested.addSublayer(&metal);
        let outer = CALayer::new();
        outer.addSublayer(&nested);
        assert!(!metal_in_layer_tree(&outer, std::slice::from_ref(&nested)));
        // ...but an unpruned sibling holding a Metal layer is still
        // found, and pruning it too leaves a clean tree.
        let beside = CALayer::new();
        beside.addSublayer(&metal);
        outer.addSublayer(&beside);
        assert!(metal_in_layer_tree(&outer, std::slice::from_ref(&nested)));
        let both = [nested, beside];
        assert!(!metal_in_layer_tree(&outer, &both));
    }

    /// A `CAMetalLayer` inside a hidden or fully transparent subtree
    /// renders nothing under `renderInContext` — its absence is correct,
    /// so the scan must not fail the capture over it (#2104).
    #[test]
    fn an_invisible_metal_layer_is_not_unresolved() {
        use objc2_quartz_core::{CALayer, CAMetalLayer};
        // A hidden Metal root is invisible on its own.
        let hidden = CAMetalLayer::new();
        hidden.setHidden(true);
        assert!(!metal_in_layer_tree(&hidden, &[]));
        // ...and so is a visible Metal layer under a hidden or
        // fully-transparent ancestor.
        let metal = CAMetalLayer::new();
        let veiled = CALayer::new();
        veiled.addSublayer(&metal);
        veiled.setHidden(true);
        assert!(!metal_in_layer_tree(&veiled, &[]));
        veiled.setHidden(false);
        veiled.setOpacity(0.0);
        assert!(!metal_in_layer_tree(&veiled, &[]));
        // Restoring visibility flags it again — the gate is visibility,
        // not the layer type.
        veiled.setOpacity(1.0);
        assert!(metal_in_layer_tree(&veiled, &[]));
    }

    /// The failure type stays downcastable through `CaptureError::Failed`.
    #[test]
    fn an_unresolved_metal_layer_is_a_typed_failure() {
        let CaptureError::Failed(inner) = CaptureError::Failed(Arc::new(UnresolvedMetalLayer))
        else {
            panic!("the failure carries its typed error")
        };
        inner
            .downcast_ref::<UnresolvedMetalLayer>()
            .expect("the failure is an UnresolvedMetalLayer");
    }

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

    /// The compositor's `RGBA16Float` store rounds to nearest even (#2827):
    /// a source-compiled library takes the platform's toward-zero texture
    /// store default, so this composites an overlay whose blend result lands
    /// strictly between two f16s and reads back the stored half — under
    /// toward-zero it would land one step low.
    #[test]
    fn an_off_grid_rgba16float_composite_stores_the_nearest_half() {
        let Some(device) = MTLCreateSystemDefaultDevice() else {
            return; // No Metal on this runner — nothing to check.
        };
        let mut resources = DeviceResources::new(&device);
        let queue = device.newCommandQueue().expect("a Metal command queue");
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::RGBA16Float,
                1,
                1,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        let texture = |texel: [u16; 4]| {
            let texture = device
                .newTextureWithDescriptor(&descriptor)
                .expect("a 1x1 RGBA16Float texture");
            // SAFETY: `texel` is a live 8-byte buffer for a 4-channel
            // 16-bit texel, matching the texture's layout.
            unsafe {
                texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                    MTLRegion {
                        origin: MTLOrigin { x: 0, y: 0, z: 0 },
                        size: MTLSize {
                            width: 1,
                            height: 1,
                            depth: 1,
                        },
                    },
                    0,
                    std::ptr::NonNull::from(&texel).cast::<core::ffi::c_void>(),
                    8,
                );
            }
            texture
        };
        // 0x0300 is the f16 subnormal 3 * 2^-16; 0x2E66 is the f16 nearest
        // 0.1, 0.0999755859375. A transparent overlay adds the two: the
        // blend result 0.1000213623046875 sits above the midpoint of its
        // f16 cell, so rte stores 0x2E67 while toward-zero stores 0x2E66.
        let overlay = texture([0x0300, 0, 0, 0]);
        let target = texture([0x2E66, 0, 0, 0x3C00]);
        let command_buffer = queue.commandBuffer().expect("a command buffer");
        let pass = MTLRenderPassDescriptor::new();
        // SAFETY: index 0 is the single color attachment.
        let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(&target));
        attachment.setLoadAction(MTLLoadAction::Load);
        attachment.setStoreAction(MTLStoreAction::Store);
        let encoder = command_buffer
            .renderCommandEncoderWithDescriptor(&pass)
            .expect("a render encoder");
        encoder.setRenderPipelineState(&resources.render_pipeline(MTLPixelFormat::RGBA16Float));
        let sampler = resources.composite_sampler();
        encoder.setViewport(MTLViewport {
            originX: 0.0,
            originY: 0.0,
            width: 1.0,
            height: 1.0,
            znear: 0.0,
            zfar: 1.0,
        });
        let region = CompositeRegion {
            uv_origin: [0.0, 0.0],
            uv_scale: [1.0, 1.0],
        };
        let half_float = crate::metal::shader_color(MTLPixelFormat::RGBA16Float);
        CompositeConversion::between(half_float, half_float).bind(&encoder, 0);
        // SAFETY: `encoder` is a live render encoder; index 0 is the
        // vertex-constant, texture and sampler slots the shader binds, and
        // `setVertexBytes` copies `region`'s bytes into the command.
        unsafe {
            encoder.setVertexBytes_length_atIndex(
                std::ptr::NonNull::from(&region).cast::<core::ffi::c_void>(),
                core::mem::size_of::<CompositeRegion>(),
                0,
            );
            encoder.setFragmentTexture_atIndex(Some(&overlay), 0);
            encoder.setFragmentSamplerState_atIndex(Some(&sampler), 0);
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
        encoder.endEncoding();
        command_buffer.commit();
        command_buffer.waitUntilCompleted();
        let mut texel = [0u16; 4];
        // SAFETY: `texel` is a live 8-byte buffer for one 4-channel
        // 16-bit texel.
        unsafe {
            target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                std::ptr::NonNull::from(&mut texel).cast::<core::ffi::c_void>(),
                8,
                MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width: 1,
                        height: 1,
                        depth: 1,
                    },
                },
                0,
            );
        }
        assert_eq!(
            texel[0], 0x2E67,
            "the off-grid blend 0.1000213623046875 stores the nearest f16 \
             (0x2E67 = 0.10003662109375), not the toward-zero 0x2E66 a \
             runtime-compiled library produces"
        );
        assert_eq!(texel[3], 0x3C00, "the opaque alpha round-trips");
    }

    /// The lease contract, exercised behaviorally: while a frame's lease
    /// is outstanding the pool issues *different* storage — so the CPU
    /// raster can never overwrite pixels a delayed consumer still reads —
    /// a settled same-generation frame returns and is reissued, a
    /// capture's base and probe sizes pool side by side, and a
    /// caller-generation bump retires the pool even on the same device.
    #[test]
    fn an_outstanding_raster_lease_is_never_reissued_across_captures_or_generations() {
        let Some(device) = MTLCreateSystemDefaultDevice() else {
            return; // No Metal on this runner — nothing to check.
        };
        let format = MTLPixelFormat::BGRA8Unorm;
        let pooled =
            |renderer: &NativeRenderer| renderer.available.values().map(Vec::len).sum::<usize>();
        let mut renderer = NativeRenderer::default();
        renderer.begin(&device, format, 0, vec![(64, 48), (16, 8)]);
        let first = renderer.issue(&device, 64, 48);
        // A second capture issued before `first` settles must not share
        // its storage — two overlapping consumers can never alias.
        let second = renderer.issue(&device, 64, 48);
        assert_ne!(
            Retained::as_ptr(first.texture()),
            Retained::as_ptr(second.texture()),
            "two outstanding captures must not share one raster destination"
        );
        // Once `first`'s consumer settles it goes back and is reused; a
        // probe raster of the same capture pools beside it.
        let probe = renderer.issue(&device, 16, 8);
        let first_texture = Retained::as_ptr(first.texture());
        renderer.return_frame(first.into_frame());
        renderer.return_frame(probe.into_frame());
        assert_eq!(pooled(&renderer), 2, "both of the capture's sizes pool");
        let reissued = renderer.issue(&device, 64, 48);
        assert_eq!(
            Retained::as_ptr(reissued.texture()),
            first_texture,
            "a settled same-generation frame returns to the pool"
        );
        // A context replacement — possibly wrapping the same physical
        // device — retires the pool: the still-outstanding `second` can
        // never answer a new generation, and a stale return is dropped
        // rather than repopulating it.
        renderer.begin(&device, format, 1, vec![(64, 48)]);
        assert_eq!(pooled(&renderer), 0, "a new generation drains the pool");
        let _next = renderer.issue(&device, 64, 48);
        renderer.return_frame(second.into_frame());
        assert_eq!(
            pooled(&renderer),
            0,
            "a superseded generation's frame must never rejoin the pool"
        );
        // A size the latest capture no longer uses drops the same way:
        // frames issued under one geometry never answer another.
        renderer.begin(&device, format, 1, vec![(128, 96)]);
        let wide_a = renderer.issue(&device, 128, 96);
        let wide_b = renderer.issue(&device, 128, 96);
        renderer.begin(&device, format, 1, vec![(64, 48)]);
        let narrow = renderer.issue(&device, 64, 48);
        renderer.return_frame(wide_a.into_frame());
        renderer.return_frame(wide_b.into_frame());
        assert_eq!(
            pooled(&renderer),
            0,
            "frames of a size the latest capture does not use never rejoin the pool"
        );
        // And a pool the owner ended (shutdown) drops a still-outstanding
        // frame instead of re-arming — a post-teardown settle is
        // reachable, so it must be quiet, not a panic.
        renderer = NativeRenderer::default();
        renderer.return_frame(narrow.into_frame());
        assert_eq!(
            pooled(&renderer),
            0,
            "a frame settling after shutdown is dropped, not re-pooled"
        );
        renderer.begin(&device, format, 0, vec![(64, 48)]);
        let _rearmed = renderer.issue(&device, 64, 48);
    }

    /// Overlap clusters: a chain of overlapping rects shares one cluster
    /// whose members alternate slots — only rects that overlap need
    /// distinct ones — and a rect apart from them is a cluster of its own.
    #[test]
    fn overlapping_surfaces_cluster_with_distinct_slots() {
        let spec = |x, y, width, height| SurfaceSpec {
            surface_id: x,
            full_size: MTLSize {
                width,
                height,
                depth: 1,
            },
            clip: MTLScissorRect {
                x,
                y,
                width,
                height,
            },
            uv_offset: [0.0; 2],
            uv_dx: [0.0; 2],
            uv_dy: [0.0; 2],
            pixel_format: MTLPixelFormat::BGRA8Unorm,
        };
        let plans = plan_clusters(&[
            spec(0, 0, 10, 10),
            spec(5, 5, 10, 10),
            spec(12, 12, 5, 5),
            spec(50, 50, 4, 4),
        ]);
        let summary = |plan: &super::ClusterPlan| {
            (
                plan.members
                    .iter()
                    .map(|m| (m.surface, m.slot))
                    .collect::<Vec<_>>(),
                plan.slots,
                [
                    plan.region.x,
                    plan.region.y,
                    plan.region.width,
                    plan.region.height,
                ],
            )
        };
        assert_eq!(plans.len(), 2, "a chain and a loner: {plans:?}");
        assert_eq!(
            summary(&plans[0]),
            (vec![(0, 0), (1, 1), (2, 0)], 2, [0, 0, 17, 17])
        );
        assert_eq!(summary(&plans[1]), (vec![(3, 0)], 1, [50, 50, 4, 4]));
    }

    /// A 2D `format` texture of `width` × `height` filled from `bytes`
    /// (`bytes_per_row` per row), sampled and renderable.
    fn filled_texture(
        device: &objc2::runtime::ProtocolObject<dyn MTLDevice>,
        format: MTLPixelFormat,
        (width, height): (usize, usize),
        bytes: &[u8],
        bytes_per_row: usize,
    ) -> Retained<objc2::runtime::ProtocolObject<dyn MTLTexture>> {
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                format, width, height, false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        let texture = device
            .newTextureWithDescriptor(&descriptor)
            .expect("a test texture");
        assert_eq!(bytes.len(), bytes_per_row * height);
        // SAFETY: `bytes` covers `height` rows of `bytes_per_row`, the
        // layout of the region written.
        unsafe {
            texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width,
                        height,
                        depth: 1,
                    },
                },
                0,
                std::ptr::NonNull::from(bytes).cast::<core::ffi::c_void>(),
                bytes_per_row,
            );
        }
        texture
    }

    /// The ordered resolve reproduces the composite it measured. A model
    /// tree — an opaque background, a group at half opacity holding GPU
    /// surface `a` under a translucent native layer, GPU surface `b`
    /// partly covering `a`, a translucent native foreground over all — is
    /// evaluated on the CPU for every probe combination the capture
    /// rasterizes; the GPU resolve of the real surface textures through
    /// those rasters must equal the same tree evaluated with the real
    /// surface pixels, to the 8-bit target's precision. The surfaces sit
    /// off the target's origin, so the probe rasters, the intermediate
    /// level and each surface's mapping all carry an offset.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one end-to-end fixture: the model tree, its probe rasters, the resolve and the check"
    )]
    fn the_ordered_resolve_reproduces_the_measured_composite() {
        type Rgba = [f32; 4];
        /// The target's edge; `a` covers (1,1)–(5,5), `b` (2,2)–(4,4).
        const EDGE: usize = 6;

        let Some(device) = MTLCreateSystemDefaultDevice() else {
            return; // No Metal on this runner — nothing to check.
        };
        let over = |top: Rgba, under: Rgba| -> Rgba {
            std::array::from_fn(|c| (1.0 - top[3]).mul_add(under[c], top[c]))
        };
        let scale = |value: Rgba, by: f32| value.map(|c| c * by);
        let background: Rgba = [0.9, 0.5, 0.1, 1.0];
        let middle = scale([0.2, 0.8, 0.4, 1.0], 0.5);
        let foreground = scale([1.0, 1.0, 0.2, 1.0], 0.25);
        // The tree's composite for surface colours `a` and `b`.
        let tree = |a: Rgba, b: Rgba| -> Rgba {
            let group = scale(over(middle, a), 0.5);
            over(foreground, over(b, over(group, background)))
        };
        let inside = |x: usize, y: usize, low: usize, high: usize| {
            (low..high).contains(&x) && (low..high).contains(&y)
        };
        // Premultiplied RGBA8 texels of each surface, by target pixel.
        let a_pixel = |x: usize, y: usize| -> [u8; 4] {
            let alpha = [255_u16, 128, 64, 0][x - 1];
            let tone = [255_u16, 40, 160, 90][y - 1];
            let premultiplied = |c: u16| u8::try_from(c * alpha / 255).expect("a byte");
            [
                premultiplied(tone),
                premultiplied(255 - tone),
                premultiplied(128),
                u8::try_from(alpha).expect("a byte"),
            ]
        };
        let b_pixel = |x: usize, y: usize| -> [u8; 4] {
            [[200, 0, 30, 200], [0, 120, 120, 120]][(x + y) % 2]
        };
        let unit = |texel: [u8; 4]| texel.map(|c| f32::from(c) / 255.0);
        // What a surface shows in probe `state` — 0 transparent, 1 black,
        // 2 white — where it covers the pixel.
        let probe = |state: usize, covers: bool| -> Rgba {
            match (covers, state) {
                (false, _) | (_, 0) => [0.0; 4],
                (_, 1) => [0.0, 0.0, 0.0, 1.0],
                _ => [1.0; 4],
            }
        };
        // A raster of the region at `origin`, `edge` pixels on a side.
        let raster = |state_a: usize, state_b: usize, origin: usize, edge: usize| -> Vec<u8> {
            let mut bytes = Vec::new();
            for y in origin..origin + edge {
                for x in origin..origin + edge {
                    let value = tree(
                        probe(state_a, inside(x, y, 1, 5)),
                        probe(state_b, inside(x, y, 2, 4)),
                    );
                    bytes.extend(value.iter().flat_map(|c| c.to_ne_bytes()));
                }
            }
            bytes
        };
        let float_texture = |bytes: &[u8], edge: usize| {
            filled_texture(
                &device,
                MTLPixelFormat::RGBA32Float,
                (edge, edge),
                bytes,
                edge * 16,
            )
        };
        let base = Placed {
            texture: float_texture(&raster(0, 0, 0, EDGE), EDGE),
            origin: [0, 0],
            color: BLEND_COLOR,
        };

        let quad = |at: f64, edge: f64| [[at, at], [at + edge, at], [at, at + edge]];
        let texels = |low: usize, high: usize, pixel: &dyn Fn(usize, usize) -> [u8; 4]| {
            (low..high)
                .flat_map(|y| (low..high).flat_map(move |x| pixel(x, y)))
                .collect::<Vec<u8>>()
        };
        let surfaces = [
            (0, quad(1.0, 4.0), 4, texels(1, 5, &a_pixel)),
            (1, quad(2.0, 2.0), 2, texels(2, 4, &b_pixel)),
        ]
        .map(|(id, quad, edge, bytes)| RenderedSurface {
            spec: surface_spec(id, quad, || MTLPixelFormat::RGBA8Unorm, EDGE, EDGE)
                .expect("the surface lands in the target"),
            texture: filled_texture(
                &device,
                MTLPixelFormat::RGBA8Unorm,
                (edge, edge),
                &bytes,
                edge * 4,
            ),
        });
        let plans = plan_clusters(&surfaces.each_ref().map(|s| s.spec));
        let [plan] = plans.as_slice() else {
            panic!("the overlapping surfaces form one cluster: {plans:?}");
        };
        assert_eq!(
            (
                plan.slots,
                [plan.region.x, plan.region.y, plan.region.width]
            ),
            (2, [1, 1, 4]),
            "`a` takes slot 0 and `b` slot 1 over `a`'s rect"
        );
        // Combination `state_a + 3·state_b`, the all-transparent one being
        // the base raster.
        let clusters = [ClusterImages {
            region: plan.region,
            slots: plan.slots,
            probes: (1..9)
                .map(|c| float_texture(&raster(c % 3, c / 3, 1, 4), 4))
                .collect(),
            members: plan.members.clone(),
        }];

        let target = filled_texture(
            &device,
            MTLPixelFormat::RGBA8Unorm,
            (EDGE, EDGE),
            &[0; EDGE * EDGE * 4],
            EDGE * 4,
        );
        let state = Mutex::new(CompositorState { resources: None });
        let mut guard = CompositorGuard {
            state: state.lock().expect("an unpoisoned state"),
        };
        let command_buffer = guard.make_command_buffer(&device);
        guard.encode_composition(
            &surfaces,
            &base,
            &clusters,
            &target,
            &command_buffer,
            &device,
        );
        drop(guard);
        command_buffer.commit();
        command_buffer.waitUntilCompleted();
        assert_eq!(command_buffer.status(), MTLCommandBufferStatus::Completed);

        let mut stored = [0_u8; EDGE * EDGE * 4];
        // SAFETY: `stored` holds the whole RGBA8 target, `EDGE * 4` bytes
        // a row.
        unsafe {
            target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                std::ptr::NonNull::from(&mut stored).cast::<core::ffi::c_void>(),
                EDGE * 4,
                MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width: EDGE,
                        height: EDGE,
                        depth: 1,
                    },
                },
                0,
            );
        }
        for y in 0..EDGE {
            for x in 0..EDGE {
                let a = if inside(x, y, 1, 5) {
                    unit(a_pixel(x, y))
                } else {
                    [0.0; 4]
                };
                let b = if inside(x, y, 2, 4) {
                    unit(b_pixel(x, y))
                } else {
                    [0.0; 4]
                };
                let expected = tree(a, b);
                let offset = (y * EDGE + x) * 4;
                let got = unit(stored[offset..offset + 4].try_into().expect("one texel"));
                assert!(
                    (0..4).all(|c| (got[c] - expected[c]).abs() <= 1.5 / 255.0),
                    "pixel ({x}, {y}) resolved to {got:?}, the tree composites {expected:?}"
                );
            }
        }
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
            uv_offset: [0.0, 0.0],
            uv_dx: [0.125, 0.0],
            uv_dy: [0.0, 0.125],
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
                        .pipelines
                        .get(&spec.pixel_format)
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
            assert!(resources.pipelines.is_empty());
            assert!(resources.blend_texture.is_none());
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

    /// `surface_spec` placements: the producer keeps the mapped rect's
    /// full extent while the clip bounds the destination and the inverse
    /// mapping selects what each destination point samples.
    mod spec {
        use objc2_metal::{MTLPixelFormat, MTLScissorRect};

        use crate::geometry::{Point, Rect};

        use super::super::{CaptureGeometry, SurfaceSpec, surface_spec};

        /// The 2× geometry a 200×200 source draws on a 400×400 target.
        fn down(source: Rect) -> CaptureGeometry {
            CaptureGeometry::new(source, 400, 400, true)
        }

        /// The same scale under an unflipped (bottom-up) source.
        fn up(source: Rect) -> CaptureGeometry {
            CaptureGeometry::new(source, 400, 400, false)
        }

        /// The quad of an unrotated `rect` whose texture's first row is
        /// its visual top edge — the layer's minimum-y edge in a y-down
        /// source, its maximum-y edge in a bottom-up one.
        fn quad(geometry: CaptureGeometry, rect: Rect) -> [[f64; 2]; 3] {
            let (left, right) = (rect.origin.x, rect.origin.x + rect.size.width);
            let (min_y, max_y) = (rect.origin.y, rect.origin.y + rect.size.height);
            let (top, bottom) = if geometry.y_down {
                (min_y, max_y)
            } else {
                (max_y, min_y)
            };
            [(left, top), (right, top), (left, bottom)]
                .map(|(x, y)| geometry.texel(Point::new(x, y)))
        }

        fn spec_for(geometry: CaptureGeometry, child: Rect) -> SurfaceSpec {
            surface_spec(
                0,
                quad(geometry, child),
                || MTLPixelFormat::BGRA8Unorm,
                400,
                400,
            )
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

        /// Asserts the texture coordinate capture-target point `at` samples.
        #[track_caller]
        fn assert_uv(spec: &SurfaceSpec, at: [f32; 2], expected: [f32; 2]) {
            let uv = [0, 1].map(|axis| {
                at[1].mul_add(
                    spec.uv_dy[axis],
                    at[0].mul_add(spec.uv_dx[axis], spec.uv_offset[axis]),
                )
            });
            assert!(
                (uv[0] - expected[0]).abs() < 1e-5 && (uv[1] - expected[1]).abs() < 1e-5,
                "point {at:?} samples {uv:?}, expected {expected:?}"
            );
        }

        #[test]
        fn a_top_down_source_maps_straight_through() {
            let spec = spec_for(
                down(Rect::new(0.0, 0.0, 200.0, 200.0)),
                Rect::new(40.0, 20.0, 100.0, 100.0),
            );
            assert_eq!(clip(&spec), (80, 40, 200, 200));
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_uv(&spec, [80.0, 40.0], [0.0, 0.0]);
            assert_uv(&spec, [280.0, 240.0], [1.0, 1.0]);
        }

        #[test]
        fn a_bottom_up_source_mirrors_against_its_own_height() {
            let spec = spec_for(
                up(Rect::new(0.0, 0.0, 200.0, 200.0)),
                Rect::new(40.0, 20.0, 100.0, 100.0),
            );
            assert_eq!(clip(&spec), (80, 160, 200, 200));
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            // The texture's first row is the rect's visual top, on the
            // destination's upper rows.
            assert_uv(&spec, [80.0, 160.0], [0.0, 0.0]);
            assert_uv(&spec, [280.0, 360.0], [1.0, 1.0]);
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
            // destination rect shrinks, and the visible corner samples
            // the matching window.
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_eq!(clip(&spec), (0, 0, 120, 160));
            assert_uv(&spec, [0.0, 0.0], [0.4, 0.2]);
        }

        #[test]
        fn a_rotated_surface_clips_to_its_bounding_box() {
            // A 200×200 texture turned a quarter clockwise: its first row
            // runs down the right edge of the destination square.
            let spec = surface_spec(
                0,
                [[300.0, 100.0], [300.0, 300.0], [100.0, 100.0]],
                || MTLPixelFormat::BGRA8Unorm,
                400,
                400,
            )
            .expect("the rotated surface lands in the target");
            assert_eq!(clip(&spec), (100, 100, 200, 200));
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 200));
            assert_uv(&spec, [300.0, 100.0], [0.0, 0.0]);
            assert_uv(&spec, [300.0, 300.0], [1.0, 0.0]);
            assert_uv(&spec, [100.0, 100.0], [0.0, 1.0]);
        }

        #[test]
        fn an_empty_child_rect_maps_to_none() {
            let geometry = down(Rect::new(0.0, 0.0, 200.0, 200.0));
            for rect in [
                // Zero extent at a fractional position must not quantize
                // into a 1-pixel producer.
                Rect::new(0.9, 0.9, 0.0, 10.0),
                Rect::new(0.9, 0.9, 10.0, 0.0),
            ] {
                assert!(
                    surface_spec(
                        0,
                        quad(geometry, rect),
                        || MTLPixelFormat::BGRA8Unorm,
                        400,
                        400
                    )
                    .is_none(),
                    "an empty child rect maps to None: {rect:?}"
                );
            }
        }

        #[test]
        fn a_surface_entirely_outside_maps_to_none() {
            let geometry = down(Rect::new(0.0, 0.0, 200.0, 200.0));
            for child in [
                Rect::new(500.0, 0.0, 100.0, 100.0),
                Rect::new(-500.0, 0.0, 100.0, 100.0),
            ] {
                assert!(
                    surface_spec(
                        0,
                        quad(geometry, child),
                        || MTLPixelFormat::BGRA8Unorm,
                        400,
                        400
                    )
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
                quad(geometry, Rect::new(40.0, 20.0, 100.0, 100.0)),
                || MTLPixelFormat::BGRA8Unorm,
                400,
                800,
            )
            .expect("the child intersects the target");
            assert_eq!(clip(&spec), (80, 80, 200, 400));
            assert_eq!((spec.full_size.width, spec.full_size.height), (200, 400));
        }

        #[test]
        fn fractional_endpoints_clip_from_floor_low_to_ceil_high() {
            // A 1× geometry: (0.9, 0.9)–(1.4, 1.4) touches pixels 0 and 1,
            // so the destination is 2 wide — while the producer needs only
            // the rect's own half-pixel span, rounded up.
            let spec = spec_for(
                CaptureGeometry::new(Rect::new(0.0, 0.0, 400.0, 400.0), 400, 400, true),
                Rect::new(0.9, 0.9, 0.5, 0.5),
            );
            assert_eq!(clip(&spec), (0, 0, 2, 2));
            assert_eq!((spec.full_size.width, spec.full_size.height), (1, 1));
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
