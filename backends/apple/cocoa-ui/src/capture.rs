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
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
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
use objc2_quartz_core::{CACornerMask, CALayer, CATransaction, CATransform3D};

use crate::PlatformView;
use crate::core_animation::flush_transaction;
use crate::geometry::{Rect, Size};
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

    /// Consumes the wrapper, yielding the value — unlike `.0`, a method
    /// call captures the whole `QueueSend` into a `move` closure.
    fn into_inner(self) -> T {
        self.0
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

/// The private texture a GPU surface's full content renders into.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSpec {
    /// The capture's identity for the surface — the surface view's address.
    pub surface_id: usize,
    /// The surface's full-content pixel size — its placement is the
    /// paint-order plan's business, not the spec's.
    pub size: MTLSize,
    /// The format the surface renders at.
    pub pixel_format: MTLPixelFormat,
}

/// The pixel size `content_size` (in points) maps to at `geometry`'s
/// scale: the texture covering the surface's whole content. `None` when
/// it is empty.
#[must_use]
pub fn surface_spec(
    surface_id: usize,
    content_size: Size,
    geometry: CaptureGeometry,
    pixel_format: MTLPixelFormat,
) -> Option<SurfaceSpec> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "captured rects are small and finite"
    )]
    let width = (content_size.width * geometry.scale_x).ceil() as usize;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "captured rects are small and finite"
    )]
    let height = (content_size.height * geometry.scale_y).ceil() as usize;
    (width > 0 && height > 0).then_some(SurfaceSpec {
        surface_id,
        size: MTLSize {
            width,
            height,
            depth: 1,
        },
        pixel_format,
    })
}

// =====================================================================
// Paint-order capture plan
// =====================================================================
//
// The composite can't draw the resolved surfaces under one full-frame
// native raster: an opaque ancestor background would paint over every
// GPU child, and the reverse order would lift them over native content
// that must cover them. Instead the capture walks the model layer tree
// in Core Animation paint order — `sublayers` sorted stably by
// `zPosition` — and emits `CaptureNode`s the compositor draws front to
// back on one command buffer:
//
// * A subtree without GPU descendants collapses into one `Native` node —
//   `renderInContext` reproduces its transforms, group opacities, masks,
//   and clips wholesale, so a native-only capture stays a single raster.
// * A layer with GPU descendants splits into `Native` own-content
//   (rastered with its direct sublayers hidden, inside the capture
//   transaction — `renderInContext` reads the model tree, so the render
//   server never sees it) followed by its children in paint order.
// * An ancestor needing offscreen compositing — `mask`,
//   `shouldRasterize`, or `opacity < 1` under `allowsGroupOpacity` —
//   becomes a `Group` whose children render into a transient texture
//   the node's own draw then composites at the accumulated opacity.
// * A resolved surface draws as a `Surface` node: the producer's
//   full-size texture composited as a transformed, clipped quad where
//   its host layer paints.

/// `CATransform3DIdentity` spelled out — the extern static isn't const.
const IDENTITY_3D: CATransform3D = CATransform3D {
    m11: 1.0,
    m12: 0.0,
    m13: 0.0,
    m14: 0.0,
    m21: 0.0,
    m22: 1.0,
    m23: 0.0,
    m24: 0.0,
    m31: 0.0,
    m32: 0.0,
    m33: 1.0,
    m34: 0.0,
    m41: 0.0,
    m42: 0.0,
    m43: 0.0,
    m44: 1.0,
};

/// How many rounded-rect clip shapes one draw's parameter block carries.
/// Deeper chains lose their outermost clips — an approximation, not a
/// visual break: inner clips dominate in practice.
const MAX_CLIP_SHAPES: usize = 8;

/// A layer's identity in the plan — its address.
fn layer_key(layer: &CALayer) -> usize {
    core::ptr::from_ref::<CALayer>(layer) as usize
}

/// The layer's sublayers in paint order: `zPosition` ascending, ties in
/// array order — a stable sort reproduces both.
fn ordered_sublayers(layer: &CALayer) -> Vec<Retained<CALayer>> {
    // SAFETY: the layer outlives the returned array.
    let Some(sublayers) = (unsafe { layer.sublayers() }) else {
        return Vec::new();
    };
    let mut sublayers: Vec<Retained<CALayer>> = sublayers.iter().collect();
    sublayers.sort_by(|a, b| a.zPosition().total_cmp(&b.zPosition()));
    sublayers
}

/// The `layer` → `parent` boundary transform, exactly as Core Animation
/// composes it — derived against `convertPoint`/`convertRect` on a live
/// layer tree. Application order:
/// 1. `geometryFlipped` mirrors Y about the bounds midline, in the
///    layer's own local space (`F_L`);
/// 2. the anchor point translates to the layer's origin;
/// 3. `transform` rotates/scales about that anchor;
/// 4. `position`/`zPosition` translate into the parent;
/// 5. the parent's `sublayerTransform` applies about the parent's
///    anchor point — but only for real sublayers: pass `None` for the
///    mask layer, which isn't one.
fn boundary_transform(layer: &CALayer, parent: Option<&CALayer>) -> CATransform3D {
    let bounds = layer.bounds();
    let anchor = layer.anchorPoint();
    let anchor_x = anchor.x.mul_add(bounds.size.width, bounds.origin.x);
    let anchor_y = anchor.y.mul_add(bounds.size.height, bounds.origin.y);
    let mut transform = if layer.isGeometryFlipped() {
        CATransform3D::new_scale(1.0, -1.0, 1.0).concat(CATransform3D::new_translation(
            0.0,
            2.0_f64.mul_add(bounds.origin.y, bounds.size.height),
            0.0,
        ))
    } else {
        IDENTITY_3D
    };
    transform = transform.concat(CATransform3D::new_translation(
        -anchor_x,
        -anchor_y,
        -layer.anchorPointZ(),
    ));
    transform = transform.concat(layer.transform());
    let position = layer.position();
    transform = transform.concat(CATransform3D::new_translation(
        position.x,
        position.y,
        layer.zPosition(),
    ));
    if let Some(parent) = parent {
        let sublayer_transform = parent.sublayerTransform();
        if !sublayer_transform.is_identity() {
            let parent_bounds = parent.bounds();
            let parent_anchor = parent.anchorPoint();
            let px = parent_anchor
                .x
                .mul_add(parent_bounds.size.width, parent_bounds.origin.x);
            let py = parent_anchor
                .y
                .mul_add(parent_bounds.size.height, parent_bounds.origin.y);
            let pz = parent.anchorPointZ();
            transform = transform.concat(
                CATransform3D::new_translation(-px, -py, -pz)
                    .concat(sublayer_transform)
                    .concat(CATransform3D::new_translation(px, py, pz)),
            );
        }
    }
    transform
}

/// A point under a `CATransform3D`, perspective-divided.
fn project_point(point: CGPoint, transform: &CATransform3D) -> Option<CGPoint> {
    let w = transform
        .m14
        .mul_add(point.x, transform.m24.mul_add(point.y, transform.m44));
    if w.abs() < 1e-9 {
        return None;
    }
    Some(CGPoint::new(
        transform
            .m11
            .mul_add(point.x, transform.m21.mul_add(point.y, transform.m41))
            / w,
        transform
            .m12
            .mul_add(point.x, transform.m22.mul_add(point.y, transform.m42))
            / w,
    ))
}

/// The axis-aligned box the rect covers under `transform`. When a corner
/// projects through the horizon the box is unbounded — report the
/// extent collapsed to `None` as "covers everything" upstream.
fn project_rect(rect: CGRect, transform: &CATransform3D) -> CGRect {
    let (min, max) = (rect.min(), rect.max());
    let mut lo = CGPoint::new(f64::INFINITY, f64::INFINITY);
    let mut hi = CGPoint::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (x, y) in [
        (min.x, min.y),
        (max.x, min.y),
        (min.x, max.y),
        (max.x, max.y),
    ] {
        if let Some(point) = project_point(CGPoint::new(x, y), transform) {
            lo.x = lo.x.min(point.x);
            lo.y = lo.y.min(point.y);
            hi.x = hi.x.max(point.x);
            hi.y = hi.y.max(point.y);
        }
    }
    if lo.x.is_finite() {
        CGRect::new(lo, CGSize::new(hi.x - lo.x, hi.y - lo.y))
    } else {
        // Every corner behind the projection plane: the quad's geometry
        // is degenerate — an empty box culls it downstream.
        CGRect::ZERO
    }
}

/// The intersection of two rects; empty when they don't overlap.
fn rect_intersect(a: CGRect, b: CGRect) -> CGRect {
    let min_x = a.origin.x.max(b.origin.x);
    let min_y = a.origin.y.max(b.origin.y);
    let max_x = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let max_y = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    CGRect::new(
        CGPoint::new(min_x, min_y),
        CGSize::new((max_x - min_x).max(0.0), (max_y - min_y).max(0.0)),
    )
}

/// The smallest rect covering both inputs.
fn rect_union(a: CGRect, b: CGRect) -> CGRect {
    let min_x = a.origin.x.min(b.origin.x);
    let min_y = a.origin.y.min(b.origin.y);
    let max_x = (a.origin.x + a.size.width).max(b.origin.x + b.size.width);
    let max_y = (a.origin.y + a.size.height).max(b.origin.y + b.size.height);
    CGRect::new(
        CGPoint::new(min_x, min_y),
        CGSize::new(max_x - min_x, max_y - min_y),
    )
}

/// The `cornerRadius` fanned out per corner per `maskedCorners` —
/// `MinXMinY` first, the order the shader's quadrant select expects.
fn clip_radii(layer: &CALayer) -> [f32; 4] {
    #[expect(clippy::cast_possible_truncation, reason = "corner radii fit in f32")]
    let radius = layer.cornerRadius() as f32;
    let masked = layer.maskedCorners();
    [
        if masked.contains(CACornerMask::LayerMinXMinYCorner) {
            radius
        } else {
            0.0
        },
        if masked.contains(CACornerMask::LayerMaxXMinYCorner) {
            radius
        } else {
            0.0
        },
        if masked.contains(CACornerMask::LayerMinXMaxYCorner) {
            radius
        } else {
            0.0
        },
        if masked.contains(CACornerMask::LayerMaxXMaxYCorner) {
            radius
        } else {
            0.0
        },
    ]
}

/// A rounded-rect clip an ancestor applies to everything under it,
/// evaluated in the shader in the clipping layer's local space.
#[derive(Clone, Debug)]
struct ClipShape {
    /// Root-space → the clipping layer's local space.
    inverse: CATransform3D,
    /// The clip rect — the layer's bounds — in its local space.
    bounds: CGRect,
    /// Per-corner radius honoring `maskedCorners`, in local units.
    radii: [f32; 4],
}

/// The accumulated clip list shared down the walk.
type Clip = Rc<Vec<ClipShape>>;

/// One `renderInContext` raster the plan asks the main thread to
/// produce — the CPU half of a `Native` node or a group's alpha mask.
#[derive(Debug)]
struct RasterSegment {
    /// The layer whose subtree (or own content) is rastered.
    layer: Retained<CALayer>,
    /// Affine applied to the layer's space before the platform mapping —
    /// `None` rasters the layer in its own local space, for a non-affine
    /// node transform or a mask (whose transform the owner's own
    /// boundary carries instead).
    space_transform: Option<CGAffineTransform>,
    /// The rect the raster covers in the space `space_transform`
    /// outputs — destination space for a standard node, the layer's own
    /// space for a non-affine one.
    extent: CGRect,
    /// Hide the layer's direct sublayers for the draw.
    own_content_only: bool,
    /// Draw at full opacity — the group composite applies the layer's
    /// own opacity itself.
    suppress_opacity: bool,
}

impl RasterSegment {
    /// The raster's pixel size at `scale` points-per-pixel.
    fn pixel_size(&self, scale: CGSize) -> (usize, usize) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "raster extents are small and finite"
        )]
        let width = (self.extent.size.width * scale.width).ceil() as usize;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "raster extents are small and finite"
        )]
        let height = (self.extent.size.height * scale.height).ceil() as usize;
        (width, height)
    }
}

/// One draw in the composite pass, in Core Animation paint order.
#[derive(Debug)]
enum CaptureNode {
    /// A raster segment composited as a quad.
    Native {
        /// Index into `CapturePlan::segments` / the raster lease list.
        raster: usize,
        /// Source space → root space for the quad's `source_rect`.
        transform: CATransform3D,
        /// The quad's rect in the raster's source space.
        source_rect: CGRect,
        /// Ancestor rounded-rect clips.
        clip: Clip,
        /// Accumulated ancestor opacity — the layer's own is inside the
        /// raster, which `renderInContext` applies itself.
        opacity: f32,
    },
    /// A resolved GPU surface composited as a quad.
    Surface {
        /// The surface's prepared texture spec.
        spec: SurfaceSpec,
        /// Host-layer space → root space.
        transform: CATransform3D,
        /// The quad's rect in host-layer space — the host's bounds.
        source_rect: CGRect,
        /// Sample the producer texture V-flipped — a flipped host layer.
        source_flip: bool,
        clip: Clip,
        /// Accumulated opacity, including the host layer's own.
        opacity: f32,
    },
    /// An offscreen group: `children` render into a transient texture
    /// the node composites at `opacity` through an optional mask.
    Group(GroupDraw),
}

/// The payload of a `CaptureNode::Group`: children render into a
/// transient texture covering `extent` in root space; the draw
/// composites it through the accumulated opacity, the ancestors'
/// clips, and the optional alpha mask.
#[derive(Debug)]
struct GroupDraw {
    /// Root-space extent the group texture covers — also the draw's
    /// quad rect.
    extent: CGRect,
    /// Accumulated opacity × the group layer's own.
    opacity: f32,
    /// Ancestor clips — applied to the group's composite draw.
    clip: Clip,
    /// Index into `CapturePlan::mask_segments` / the mask rasters.
    mask: Option<usize>,
    /// The extent the mask texture covers, in the masked layer's local
    /// space.
    mask_extent: CGRect,
    /// Root space → the masked layer's local space.
    mask_inverse: CATransform3D,
    /// The group's content, painted into its transient texture.
    children: Vec<CaptureNode>,
}

/// What the plan needs of one resolved surface: the layer whose paint
/// slot its quad occupies — the resolved view's backing layer — and the
/// spec its texture was prepared with.
#[derive(Clone, Debug)]
struct PlanSurface {
    /// The resolved view's backing layer.
    layer: Retained<CALayer>,
    /// The prepared texture's spec.
    spec: SurfaceSpec,
}

/// The plan: paint-ordered nodes plus the rasters the main thread must
/// produce before the GPU composite.
#[derive(Debug)]
struct CapturePlan {
    /// Draws in paint order.
    nodes: Vec<CaptureNode>,
    /// `Native` rasters, in node `raster` index order.
    segments: Vec<RasterSegment>,
    /// Group mask rasters, in node `mask` index order.
    mask_segments: Vec<RasterSegment>,
    /// The destination's rect in root-layer space — the captured view's
    /// bounds — and the top pass's extent.
    extent: CGRect,
    /// Points-to-pixels scale of the destination.
    scale: CGSize,
}

/// Walks the model layer tree accumulating transforms, clips, and
/// opacity, emitting `CaptureNode`s.
struct PlanBuilder {
    /// Resolved surfaces keyed by host-layer pointer.
    surfaces: HashMap<usize, PlanSurface>,
    /// `has_gpu` memo keyed by layer pointer.
    has_gpu_memo: HashMap<usize, bool>,
    /// Emitted native segments.
    segments: Vec<RasterSegment>,
    /// Emitted mask segments.
    mask_segments: Vec<RasterSegment>,
}

impl PlanBuilder {
    /// Whether `layer`'s subtree resolves any surface — memoized; hidden
    /// or fully-transparent subtrees draw nothing and don't count.
    fn has_gpu(&mut self, layer: &CALayer) -> bool {
        let key = layer_key(layer);
        if let Some(has) = self.has_gpu_memo.get(&key) {
            return *has;
        }
        if layer.isHidden() || layer.opacity() <= 0.0 {
            return false;
        }
        let children = ordered_sublayers(layer);
        let has =
            self.surfaces.contains_key(&key) || children.iter().any(|child| self.has_gpu(child));
        self.has_gpu_memo.insert(key, has);
        has
    }

    /// Emits a `Native` node for `layer` and queues its raster. An
    /// affine transform rides the raster's CTM — the quad is the pass
    /// extent; a non-affine one rasters the layer in its own space and
    /// lets the quad carry the full 4×4.
    #[expect(
        clippy::too_many_arguments,
        reason = "the paint walk threads its accumulated context through each emit"
    )]
    fn native_node(
        &mut self,
        out: &mut Vec<CaptureNode>,
        layer: &Retained<CALayer>,
        transform: CATransform3D,
        clip: &Clip,
        opacity: f32,
        pass_extent: CGRect,
        own_content_only: bool,
        suppress_opacity: bool,
    ) {
        let raster = self.segments.len();
        if transform.is_affine() {
            self.segments.push(RasterSegment {
                layer: layer.clone(),
                space_transform: Some(transform.affine_transform()),
                extent: pass_extent,
                own_content_only,
                suppress_opacity,
            });
            out.push(CaptureNode::Native {
                raster,
                transform: IDENTITY_3D,
                source_rect: pass_extent,
                clip: clip.clone(),
                opacity,
            });
        } else {
            // The layer's own space covers its bounds plus whatever
            // content spills outside them (borders stroke inside, but
            // shadows extend out); a whole subtree uses its union.
            let extent = if own_content_only {
                let mut extent = layer.bounds();
                if layer.shadowOpacity() > 0.0 {
                    let offset = layer.shadowOffset();
                    let outset = layer.shadowRadius() * 2.0;
                    let grow = |side: f64, away: f64| 2.0_f64.mul_add(outset + away.abs(), side);
                    extent = CGRect::new(
                        CGPoint::new(
                            extent.origin.x - outset - offset.width.abs(),
                            extent.origin.y - outset - offset.height.abs(),
                        ),
                        CGSize::new(
                            grow(extent.size.width, offset.width),
                            grow(extent.size.height, offset.height),
                        ),
                    );
                }
                extent
            } else {
                let Some(extent) = subtree_extent(layer) else {
                    return;
                };
                extent
            };
            if extent.is_empty() {
                return;
            }
            self.segments.push(RasterSegment {
                layer: layer.clone(),
                space_transform: None,
                extent,
                own_content_only,
                suppress_opacity,
            });
            out.push(CaptureNode::Native {
                raster,
                transform,
                source_rect: extent,
                clip: clip.clone(),
                opacity,
            });
        }
    }

    /// Emits `layer`'s nodes in paint order. `transform` maps the
    /// layer's local space into root space; `clip` and
    /// `ancestor_opacity` accumulate down the walk; `pass_extent` is the
    /// destination's root-space rect.
    fn emit(
        &mut self,
        out: &mut Vec<CaptureNode>,
        layer: &Retained<CALayer>,
        transform: CATransform3D,
        clip: &Clip,
        ancestor_opacity: f32,
        pass_extent: CGRect,
    ) {
        if layer.isHidden() {
            return;
        }
        let own_opacity = layer.opacity();
        if own_opacity <= 0.0 || ancestor_opacity <= 0.0 {
            return;
        }
        let bounds = layer.bounds();
        if bounds.is_empty() {
            return;
        }

        if !self.has_gpu(layer) {
            // Collapsed native subtree.
            let visible = subtree_extent(layer).is_some_and(|extent| {
                !rect_intersect(project_rect(extent, &transform), pass_extent).is_empty()
            });
            if !visible {
                return;
            }
            self.native_node(
                out,
                layer,
                transform,
                clip,
                ancestor_opacity,
                pass_extent,
                false,
                false,
            );
            return;
        }

        // The layer's own masksToBounds clip, for descendants and a
        // hosted surface — own content carries it inside its raster.
        let own_shape = layer.masksToBounds().then(|| ClipShape {
            inverse: transform.invert(),
            bounds,
            radii: clip_radii(layer),
        });
        let child_clip: Clip = match (&own_shape, clip.len() < MAX_CLIP_SHAPES) {
            (Some(shape), true) => {
                let mut shapes = clip.as_ref().clone();
                shapes.push(shape.clone());
                Rc::new(shapes)
            }
            _ => clip.clone(),
        };

        let is_host = self.surfaces.get(&layer_key(layer)).cloned();
        let needs_group = layer.mask().is_some()
            || layer.shouldRasterize()
            || (own_opacity < 1.0 && layer.allowsGroupOpacity());

        if needs_group {
            self.emit_group(
                out,
                layer,
                transform,
                clip,
                ancestor_opacity,
                own_opacity,
                pass_extent,
                own_shape,
                is_host,
            );
        } else {
            self.emit_split(
                out,
                layer,
                transform,
                clip,
                &child_clip,
                ancestor_opacity,
                own_opacity,
                pass_extent,
                is_host,
            );
        }
    }

    /// A `Group` node for a layer whose subtree must composite
    /// offscreen — `mask`, `shouldRasterize`, or translucent group
    /// opacity. Children render into the group's transient texture with
    /// a clean opacity accumulator; the group's draw applies the
    /// accumulated opacity and the ancestors' clips once.
    #[expect(
        clippy::too_many_arguments,
        reason = "the paint walk threads its accumulated context through each emit"
    )]
    fn emit_group(
        &mut self,
        out: &mut Vec<CaptureNode>,
        layer: &Retained<CALayer>,
        transform: CATransform3D,
        clip: &Clip,
        ancestor_opacity: f32,
        own_opacity: f32,
        pass_extent: CGRect,
        own_shape: Option<ClipShape>,
        is_host: Option<PlanSurface>,
    ) {
        let Some(extent) = subtree_extent(layer)
            .map(|local| rect_intersect(project_rect(local, &transform), pass_extent))
            .filter(|extent| !extent.is_empty())
        else {
            return;
        };
        let inside_clip: Clip = Rc::new(own_shape.map_or_else(Vec::new, |shape| vec![shape]));
        let mut children = Vec::new();
        // `renderInContext` applies the layer's own opacity itself;
        // exactly 1.0 is its identity — any other value must be
        // suppressed in the raster so the group draw supplies it.
        #[expect(clippy::float_cmp, reason = "1.0 is renderInContext's identity")]
        let suppress_own = own_opacity != 1.0;
        self.native_node(
            &mut children,
            layer,
            transform,
            &inside_clip,
            1.0,
            extent,
            true,
            suppress_own,
        );
        for child in ordered_sublayers(layer) {
            let child_transform = boundary_transform(&child, Some(layer)).concat(transform);
            self.emit(
                &mut children,
                &child,
                child_transform,
                &inside_clip,
                1.0,
                extent,
            );
        }
        if let Some(host) = is_host
            && let Some(node) = surface_node(&host, layer, transform, &inside_clip, 1.0, extent)
        {
            children.push(node);
        }
        let (mask, mask_extent, mask_inverse) = layer.mask().map_or_else(
            || (None, CGRect::ZERO, IDENTITY_3D),
            |mask_layer| {
                let mask_inverse = transform.invert();
                let mask_extent = project_rect(extent, &mask_inverse);
                let index = self.mask_segments.len();
                self.mask_segments.push(RasterSegment {
                    layer: mask_layer.clone(),
                    space_transform: Some(boundary_transform(&mask_layer, None).affine_transform()),
                    extent: mask_extent,
                    own_content_only: false,
                    suppress_opacity: false,
                });
                (Some(index), mask_extent, mask_inverse)
            },
        );
        out.push(CaptureNode::Group(GroupDraw {
            extent,
            opacity: ancestor_opacity * own_opacity,
            clip: clip.clone(),
            mask,
            mask_extent,
            mask_inverse,
            children,
        }));
    }

    /// The ordinary split: own content, then the children in paint
    /// order, then a hosted surface last — its presentation sits
    /// topmost in the host layer.
    #[expect(
        clippy::too_many_arguments,
        reason = "the paint walk threads its accumulated context through each emit"
    )]
    fn emit_split(
        &mut self,
        out: &mut Vec<CaptureNode>,
        layer: &Retained<CALayer>,
        transform: CATransform3D,
        clip: &Clip,
        child_clip: &Clip,
        ancestor_opacity: f32,
        own_opacity: f32,
        pass_extent: CGRect,
        is_host: Option<PlanSurface>,
    ) {
        self.native_node(
            out,
            layer,
            transform,
            clip,
            ancestor_opacity,
            pass_extent,
            true,
            false,
        );
        let child_opacity = ancestor_opacity * own_opacity;
        for child in ordered_sublayers(layer) {
            let child_transform = boundary_transform(&child, Some(layer)).concat(transform);
            self.emit(
                out,
                &child,
                child_transform,
                child_clip,
                child_opacity,
                pass_extent,
            );
        }
        if let Some(host) = is_host
            && let Some(node) = surface_node(
                &host,
                layer,
                transform,
                child_clip,
                child_opacity,
                pass_extent,
            )
        {
            out.push(node);
        }
    }
}

impl CapturePlan {
    /// Walks `root`'s model layer tree and emits the paint-order plan.
    /// `surfaces` keys each resolved surface by its host layer;
    /// `extent` is the destination's rect in root space; `scale` the
    /// points-to-pixels scale.
    fn build(
        root: &Retained<CALayer>,
        surfaces: Vec<PlanSurface>,
        extent: CGRect,
        scale: CGSize,
    ) -> Self {
        let mut builder = PlanBuilder {
            surfaces: surfaces
                .into_iter()
                .map(|surface| (layer_key(&surface.layer), surface))
                .collect(),
            has_gpu_memo: HashMap::new(),
            segments: Vec::new(),
            mask_segments: Vec::new(),
        };
        let mut nodes = Vec::new();
        builder.emit(
            &mut nodes,
            root,
            IDENTITY_3D,
            &Rc::new(Vec::new()),
            1.0,
            extent,
        );
        Self {
            nodes,
            segments: builder.segments,
            mask_segments: builder.mask_segments,
            extent,
            scale,
        }
    }
}

/// The subtree's extent in `layer`'s own local space — `None` when it
/// draws nothing. Own bounds plus every visible child's extent
/// projected across the child→layer boundary.
fn subtree_extent(layer: &CALayer) -> Option<CGRect> {
    if layer.isHidden() || layer.opacity() <= 0.0 {
        return None;
    }
    let bounds = layer.bounds();
    if bounds.is_empty() {
        return None;
    }
    let mut extent = bounds;
    for child in ordered_sublayers(layer) {
        if let Some(child_extent) = subtree_extent(&child) {
            let projected = project_rect(child_extent, &boundary_transform(&child, Some(layer)));
            if !projected.is_empty() {
                extent = rect_union(extent, projected);
            }
        }
    }
    Some(extent)
}

/// The `Surface` node a resolved host layer contributes: the producer
/// texture composited as a quad over the host's bounds — `None` when
/// the quad is entirely outside `pass_extent`.
fn surface_node(
    host: &PlanSurface,
    layer: &CALayer,
    transform: CATransform3D,
    clip: &Clip,
    opacity: f32,
    pass_extent: CGRect,
) -> Option<CaptureNode> {
    let source_rect = layer.bounds();
    if rect_intersect(project_rect(source_rect, &transform), pass_extent).is_empty() {
        return None;
    }
    Some(CaptureNode::Surface {
        spec: host.spec,
        transform,
        source_rect,
        source_flip: layer.isGeometryFlipped(),
        clip: clip.clone(),
        opacity,
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
    /// Pooled transient textures group nodes render into — returned by
    /// the settle once the composite buffer completes.
    transients: Vec<Retained<ProtocolObject<dyn MTLTexture>>>,
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
            transients: Vec::new(),
            pipeline: None,
            pipeline_format: None,
            sampler: None,
        }
    }

    /// A transient texture of `format` × `width` × `height` for a
    /// group's inner pass — the most recently returned match, else a
    /// fresh private texture.
    ///
    /// # Panics
    ///
    /// When the device cannot allocate a texture.
    fn transient(
        &mut self,
        format: MTLPixelFormat,
        width: usize,
        height: usize,
    ) -> Retained<ProtocolObject<dyn MTLTexture>> {
        if let Some(index) = self.transients.iter().rposition(|texture| {
            texture.pixelFormat() == format
                && texture.width() == width
                && texture.height() == height
        }) {
            return self.transients.remove(index);
        }
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                format, width, height, false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        descriptor.setStorageMode(MTLStorageMode::Private);
        self.device
            .newTextureWithDescriptor(&descriptor)
            .expect("failed to create a group transient texture")
    }

    /// Takes issued transients back for reuse — the pool stays small:
    /// group passes are transient-sized and transient-lived.
    fn return_transients(&mut self, transients: Vec<Retained<ProtocolObject<dyn MTLTexture>>>) {
        const CAPACITY: usize = 8;
        self.transients.extend(transients);
        let over = self.transients.len().saturating_sub(CAPACITY);
        self.transients.drain(..over);
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

    /// Encodes the plan's composite: the nodes draw into `target` in
    /// paint order, each `Native`/`Surface` a transformed, clipped,
    /// opacity-scaled quad and each `Group` a pooled transient texture
    /// its children render into before its own draw composites it.
    /// Returns the transient textures issued, for the settle to pool
    /// back.
    ///
    /// Every raster's blit must already be encoded on `command_buffer`.
    /// Premultiplied-over blending (`.one` / `.oneMinusSourceAlpha`) and
    /// a linear clamp-to-edge sampler — the `CaptureComposite` shader.
    ///
    /// # Panics
    ///
    /// When the pipeline, a transient texture, or an encoder cannot be
    /// created.
    fn encode_composition(
        &mut self,
        preparation: &Preparation,
        rendered: &[RenderedSurface],
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    ) -> Vec<Retained<ProtocolObject<dyn MTLTexture>>> {
        let plan = &preparation.plan;
        let target = &preparation.target;
        let resources = self.state.device_resources(&preparation.device);
        let pipeline = resources.render_pipeline(target.pixelFormat());
        let sampler = resources.composite_sampler();
        let mut ctx = CompositeCtx {
            pipeline,
            sampler,
            resources,
            command_buffer,
            surfaces: rendered
                .iter()
                .map(|surface| (surface.spec.surface_id, surface.texture.clone()))
                .collect(),
            raster_textures: preparation
                .rasters
                .iter()
                .map(|lease| lease.texture().clone())
                .collect(),
            mask_textures: preparation
                .mask_rasters
                .iter()
                .map(|lease| lease.texture().clone())
                .collect(),
            transients: Vec::new(),
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "a capture target is at most a few thousand pixels on a side"
        )]
        let dst = PassSpace {
            extent: plan.extent,
            pixels: CGSize::new(target.width() as f64, target.height() as f64),
            scale: plan.scale,
        };
        encode_nodes(&mut ctx, &plan.nodes, target, &dst);
        ctx.transients
    }

    /// Returns issued transient textures to the pool — the settle path
    /// calls it once the composite buffer completed.
    fn return_transients(&mut self, transients: Vec<Retained<ProtocolObject<dyn MTLTexture>>>) {
        if let Some(resources) = &mut self.state.resources {
            resources.return_transients(transients);
        }
    }
}

/// The shared state one composite pass wires through the encoder —
/// the device bundle, the resolved texture maps, and the transient
/// textures group passes have issued.
struct CompositeCtx<'a> {
    /// The device bundle — pipelines, the sampler, the transient pool.
    resources: &'a mut DeviceResources,
    /// The command buffer passes open on.
    command_buffer: &'a ProtocolObject<dyn MTLCommandBuffer>,
    /// The composite pipeline.
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    /// The linear-clamp sampler every texture reads through.
    sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    /// Surface id → prepared texture.
    surfaces: HashMap<usize, Retained<ProtocolObject<dyn MTLTexture>>>,
    /// `Native` raster textures, in node `raster` index order.
    raster_textures: Vec<Retained<ProtocolObject<dyn MTLTexture>>>,
    /// Mask raster textures, in node `mask` index order.
    mask_textures: Vec<Retained<ProtocolObject<dyn MTLTexture>>>,
    /// Transient textures issued by group passes — returned to the
    /// caller for pooling once the buffer completes.
    transients: Vec<Retained<ProtocolObject<dyn MTLTexture>>>,
}

/// The destination space one pass renders in — the rect the target
/// covers in root-layer points, its pixel size, and the points→pixels
/// scale.
struct PassSpace {
    extent: CGRect,
    pixels: CGSize,
    scale: CGSize,
}

/// The per-draw parameter block — mirrored field-for-field by
/// `NodeParams` in `capture_composite.metal`.
#[repr(C)]
struct NodeParams {
    /// Node source space → root space.
    transform: [f32; 16],
    /// Root space → mask-owner local space (only when `has_mask`).
    mask_inverse: [f32; 16],
    /// Root space → each clip layer's local space.
    clip_inverses: [[f32; 16]; MAX_CLIP_SHAPES],
    /// Each clip's bounds in its layer's local space.
    clip_rects: [[f32; 4]; MAX_CLIP_SHAPES],
    /// Per-corner radii for each clip.
    clip_radii: [[f32; 4]; MAX_CLIP_SHAPES],
    /// The quad's rect in node source space.
    source_rect: [f32; 4],
    /// The mask texture's coverage in owner-local space.
    mask_extent: [f32; 4],
    /// The pass extent's origin in root space.
    dst_origin: [f32; 2],
    /// Points → pixels.
    dst_scale: [f32; 2],
    /// The destination's pixel size.
    dst_pixels: [f32; 2],
    /// Accumulated draw opacity.
    opacity: f32,
    /// Nonzero samples the source texture V-flipped.
    source_v_flip: f32,
    /// Nonzero samples the mask texture V-flipped.
    mask_v_flip: f32,
    /// The platform's destination convention: +1 maps the root space's
    /// max-Y edge to texel row 0 (macOS), −1 the min-Y edge (iOS).
    dst_v_flip: f32,
    /// How many clip shapes apply.
    clip_count: u32,
    /// Nonzero binds and applies the mask texture.
    has_mask: u32,
}

/// A `CATransform3D` as a Metal `float4x4` — the struct's field order IS
/// the shader's column-major layout (column i is `mi1`..`mi4`).
fn matrix_f32(transform: &CATransform3D) -> [f32; 16] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "capture transforms fit f32"
    )]
    let m = |value: f64| value as f32;
    [
        m(transform.m11),
        m(transform.m12),
        m(transform.m13),
        m(transform.m14),
        m(transform.m21),
        m(transform.m22),
        m(transform.m23),
        m(transform.m24),
        m(transform.m31),
        m(transform.m32),
        m(transform.m33),
        m(transform.m34),
        m(transform.m41),
        m(transform.m42),
        m(transform.m43),
        m(transform.m44),
    ]
}

/// The destination texel convention's V sign: +1 on macOS, −1 on iOS.
const DST_V_FLIP: f32 = if cfg!(target_os = "ios") { -1.0 } else { 1.0 };

/// The source sampling convention for pass-space textures (rasters,
/// group and mask transient): +1 inverts V (macOS), 0 passes it (iOS).
const PASS_V_FLIP: f32 = if cfg!(target_os = "ios") { 0.0 } else { 1.0 };

/// The pixel box `source_rect` covers under `transform`, clipped to the
/// pass — `None` when it misses the destination entirely.
fn draw_scissor(
    source_rect: CGRect,
    transform: &CATransform3D,
    dst: &PassSpace,
) -> Option<MTLScissorRect> {
    let hit = rect_intersect(project_rect(source_rect, transform), dst.extent);
    if hit.is_empty() {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clipped pixel boxes are small and nonnegative"
    )]
    let to_px = |value: f64| value.max(0.0) as usize;
    let x0 = to_px((hit.min().x - dst.extent.origin.x) * dst.scale.width);
    let x1 = to_px(
        ((hit.max().x - dst.extent.origin.x) * dst.scale.width)
            .ceil()
            .min(dst.pixels.width),
    );
    let (lo, hi) = (
        (hit.min().y - dst.extent.origin.y) * dst.scale.height,
        (hit.max().y - dst.extent.origin.y) * dst.scale.height,
    );
    // Texture rows run opposite the root space's Y on macOS.
    let (y0, y1) = if cfg!(target_os = "ios") {
        (to_px(lo.floor()), to_px(hi.ceil().min(dst.pixels.height)))
    } else {
        (
            to_px((dst.pixels.height - hi).floor()),
            to_px((dst.pixels.height - lo).ceil().min(dst.pixels.height)),
        )
    };
    (x1 > x0 && y1 > y0).then_some(MTLScissorRect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// The draw's scissor-free parameter block, assembled per node.
fn node_params(
    transform: &CATransform3D,
    source_rect: CGRect,
    clip: &Clip,
    opacity: f32,
    source_v_flip: f32,
    dst: &PassSpace,
) -> NodeParams {
    #[expect(clippy::cast_possible_truncation, reason = "clip extents fit f32")]
    let mut params = NodeParams {
        transform: matrix_f32(transform),
        mask_inverse: [0.0; 16],
        clip_inverses: [[0.0; 16]; MAX_CLIP_SHAPES],
        clip_rects: [[0.0; 4]; MAX_CLIP_SHAPES],
        clip_radii: [[0.0; 4]; MAX_CLIP_SHAPES],
        source_rect: [
            source_rect.origin.x as f32,
            source_rect.origin.y as f32,
            source_rect.size.width as f32,
            source_rect.size.height as f32,
        ],
        mask_extent: [0.0; 4],
        dst_origin: [dst.extent.origin.x as f32, dst.extent.origin.y as f32],
        dst_scale: [dst.scale.width as f32, dst.scale.height as f32],
        dst_pixels: [dst.pixels.width as f32, dst.pixels.height as f32],
        opacity,
        source_v_flip,
        mask_v_flip: PASS_V_FLIP,
        dst_v_flip: DST_V_FLIP,
        clip_count: 0,
        has_mask: 0,
    };
    for (index, shape) in clip.iter().take(MAX_CLIP_SHAPES).enumerate() {
        params.clip_inverses[index] = matrix_f32(&shape.inverse);
        #[expect(clippy::cast_possible_truncation, reason = "clip bounds fit f32")]
        let rect = [
            shape.bounds.origin.x as f32,
            shape.bounds.origin.y as f32,
            shape.bounds.size.width as f32,
            shape.bounds.size.height as f32,
        ];
        params.clip_rects[index] = rect;
        params.clip_radii[index] = shape.radii;
    }
    params.clip_count = u32::try_from(clip.len().min(MAX_CLIP_SHAPES)).unwrap_or_default();
    params
}

/// One textured-quad draw: the node's params to both stages, the source
/// texture at slot 0, the mask (or the source again — never sampled) at
/// slot 1, then six vertices.
fn encode_draw(
    encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    texture: &ProtocolObject<dyn MTLTexture>,
    mask: Option<&ProtocolObject<dyn MTLTexture>>,
    mut params: NodeParams,
    source_rect: CGRect,
    transform: &CATransform3D,
    dst: &PassSpace,
) {
    let Some(scissor) = draw_scissor(source_rect, transform, dst) else {
        return;
    };
    params.has_mask = u32::from(mask.is_some());
    // SAFETY: `encoder` is a live render encoder, `params` outlives the
    // call — Metal copies bytes — and 0/1 are the slots the shader binds.
    unsafe {
        encoder.setVertexBytes_length_atIndex(
            core::ptr::NonNull::from(&params).cast(),
            core::mem::size_of::<NodeParams>(),
            0,
        );
        encoder.setFragmentBytes_length_atIndex(
            core::ptr::NonNull::from(&params).cast(),
            core::mem::size_of::<NodeParams>(),
            0,
        );
        encoder.setFragmentTexture_atIndex(Some(texture), 0);
        encoder.setFragmentTexture_atIndex(mask.or(Some(texture)), 1);
        encoder.setScissorRect(scissor);
        encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
    }
}

/// Opens a pass on `target`, `Clear` on first use and `Load` when a
/// group returns to its parent's pass.
fn begin_pass(
    command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    target: &ProtocolObject<dyn MTLTexture>,
    load: MTLLoadAction,
) -> Retained<ProtocolObject<dyn MTLRenderCommandEncoder>> {
    let descriptor = MTLRenderPassDescriptor::new();
    // SAFETY: index 0 is the single color attachment.
    let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    attachment.setTexture(Some(target));
    attachment.setLoadAction(load);
    attachment.setStoreAction(MTLStoreAction::Store);
    attachment.setClearColor(MTLClearColor {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 0.0,
    });
    command_buffer
        .renderCommandEncoderWithDescriptor(&descriptor)
        .expect("failed to create the Metal capture composition encoder")
}

/// A `Group` draw: children into a pooled transient, then the group's
/// quad composited back through the re-opened parent pass. `pass`
/// holds the caller's live encoder — closed here and replaced.
fn encode_group(
    ctx: &mut CompositeCtx<'_>,
    group: &GroupDraw,
    target: &ProtocolObject<dyn MTLTexture>,
    dst: &PassSpace,
    pass: &mut Option<Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>>,
) {
    if let Some(encoder) = pass.take() {
        encoder.endEncoding();
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "group extents are small and finite"
    )]
    let (width, height) = (
        (group.extent.size.width * dst.scale.width).ceil().max(1.0) as usize,
        (group.extent.size.height * dst.scale.height)
            .ceil()
            .max(1.0) as usize,
    );
    let group_texture = ctx.resources.transient(target.pixelFormat(), width, height);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a group texture is at most a few thousand pixels on a side"
    )]
    let inner_dst = PassSpace {
        extent: group.extent,
        pixels: CGSize::new(width as f64, height as f64),
        scale: dst.scale,
    };
    encode_nodes(ctx, &group.children, &group_texture, &inner_dst);
    ctx.transients.push(group_texture.clone());
    let mut params = node_params(
        &IDENTITY_3D,
        group.extent,
        &group.clip,
        group.opacity,
        PASS_V_FLIP,
        dst,
    );
    if group.mask.is_some() {
        params.mask_inverse = matrix_f32(&group.mask_inverse);
        #[expect(clippy::cast_possible_truncation, reason = "mask extents fit f32")]
        {
            params.mask_extent = [
                group.mask_extent.origin.x as f32,
                group.mask_extent.origin.y as f32,
                group.mask_extent.size.width as f32,
                group.mask_extent.size.height as f32,
            ];
        }
    }
    let encoder = begin_pass(ctx.command_buffer, target, MTLLoadAction::Load);
    encoder.setRenderPipelineState(&ctx.pipeline);
    encoder.setViewport(MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: dst.pixels.width,
        height: dst.pixels.height,
        znear: 0.0,
        zfar: 1.0,
    });
    // SAFETY: `encoder` is a live render encoder and 0 is the sampler
    // slot the shader binds.
    unsafe {
        encoder.setFragmentSamplerState_atIndex(Some(&*ctx.sampler), 0);
    }
    encode_draw(
        &encoder,
        &group_texture,
        group.mask.map(|index| &*ctx.mask_textures[index]),
        params,
        group.extent,
        &IDENTITY_3D,
        dst,
    );
    *pass = Some(encoder);
}

/// Encodes `nodes` front to back into `target` — one pass per
/// contiguous run, reopening after each group's inner pass.
fn encode_nodes(
    ctx: &mut CompositeCtx<'_>,
    nodes: &[CaptureNode],
    target: &ProtocolObject<dyn MTLTexture>,
    dst: &PassSpace,
) {
    if nodes.is_empty() {
        return;
    }
    let mut pass = Some(begin_pass(ctx.command_buffer, target, MTLLoadAction::Clear));
    if let Some(encoder) = &pass {
        encoder.setRenderPipelineState(&ctx.pipeline);
        encoder.setViewport(MTLViewport {
            originX: 0.0,
            originY: 0.0,
            width: dst.pixels.width,
            height: dst.pixels.height,
            znear: 0.0,
            zfar: 1.0,
        });
        // SAFETY: `encoder` is a live render encoder and 0 is the
        // sampler slot the shader binds.
        unsafe {
            encoder.setFragmentSamplerState_atIndex(Some(&*ctx.sampler), 0);
        }
    }
    for node in nodes {
        match node {
            CaptureNode::Native {
                raster,
                transform,
                source_rect,
                clip,
                opacity,
            } => {
                let params = node_params(transform, *source_rect, clip, *opacity, PASS_V_FLIP, dst);
                if let Some(encoder) = &pass {
                    encode_draw(
                        encoder,
                        &ctx.raster_textures[*raster],
                        None,
                        params,
                        *source_rect,
                        transform,
                        dst,
                    );
                }
            }
            CaptureNode::Surface {
                spec,
                transform,
                source_rect,
                source_flip,
                clip,
                opacity,
            } => {
                let Some(texture) = ctx.surfaces.get(&spec.surface_id) else {
                    continue;
                };
                // The producer texture's row 0 is the surface's visual
                // top — the bounds.min.y edge when the host's local
                // space is flipped, bounds.max.y when it isn't.
                let params = node_params(
                    transform,
                    *source_rect,
                    clip,
                    *opacity,
                    if *source_flip { 0.0 } else { 1.0 },
                    dst,
                );
                if let Some(encoder) = &pass {
                    encode_draw(encoder, texture, None, params, *source_rect, transform, dst);
                }
            }
            CaptureNode::Group(group) => {
                encode_group(ctx, group, target, dst, &mut pass);
            }
        }
    }
    if let Some(encoder) = pass.take() {
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
        // Sample-only, private storage — the buffer-aliased texture this
        // replaces violated both simulator rules (private storage for
        // buffer-backed textures, no render-target usage); one private
        // texture is correct on every Apple device.
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

    /// Rasterizes `segment`'s layer into the shared buffer — a
    /// synchronous CPU draw, completed when it returns.
    ///
    /// Geometry is expressed through the context's CTM alone: the live
    /// layer tree is never transformed or reparented. The CTM places
    /// `segment.extent` on the buffer, then `space_transform` (when the
    /// node's transform is affine) maps the layer's local space into
    /// that rect; a non-affine node rasters the layer in its own space
    /// and lets the composite quad carry the full transform. Both
    /// mutations the plan asks for — hiding direct sublayers for
    /// `own_content_only`, pinning opacity for `suppress_opacity` — are
    /// temporary model edits inside the capture's disabled-actions
    /// transaction, restored before it commits: `renderInContext` reads
    /// the model tree, so the render server never sees them.
    ///
    /// The output obeys the composite pass's contract — texel row 0 is
    /// the drawn space's top edge: a bitmap context is bottom-left-
    /// origin like `AppKit`'s layer space, so on macOS a plain affine
    /// lands each row on its matching texel row; `UIKit`'s layer space
    /// is top-left-origin, so the iOS mapping flips the texel Y.
    fn draw(&self, segment: &RasterSegment) {
        let context: &CGContext = &self.context;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a capture texture is at most a few thousand pixels on a side"
        )]
        let (width, height) = (self.pixel_width as f64, self.pixel_height as f64);
        CGContext::clear_rect(
            Some(context),
            CGRect::new(CGPoint::ZERO, CGSize::new(width, height)),
        );
        CGContext::save_g_state(Some(context));
        // The platform affine maps `extent` onto the buffer; the
        // concatenated space transform applies first, so a layer-local
        // point lands where the node projects it.
        CGContext::concat_ctm(
            Some(context),
            platform_affine(segment.extent, self.pixel_width, self.pixel_height),
        );
        if let Some(space_transform) = segment.space_transform {
            CGContext::concat_ctm(Some(context), space_transform);
        }
        let _hide = segment
            .own_content_only
            .then(|| SublayerHidden::hide_all(&segment.layer));
        let _opacity = segment
            .suppress_opacity
            .then(|| LayerOpacity::full(&segment.layer));
        segment.layer.renderInContext(context);
        CGContext::restore_g_state(Some(context));
        CGContext::flush(Some(context));
    }
}

/// The affine CTM placing `extent` onto `pixel_width` × `pixel_height`
/// — the same platform convention the identity draw uses: macOS maps a
/// bottom-origin buffer space straight on, iOS flips Y so the space's
/// top edge lands on texel row 0.
fn platform_affine(extent: CGRect, pixel_width: usize, pixel_height: usize) -> CGAffineTransform {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a capture texture is at most a few thousand pixels on a side"
    )]
    let (width, height) = (pixel_width as f64, pixel_height as f64);
    let sx = width / extent.size.width;
    let sy = height / extent.size.height;
    if cfg!(target_os = "ios") {
        CGAffineTransform {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: -sy,
            tx: -extent.origin.x * sx,
            ty: extent.origin.y.mul_add(sy, height),
        }
    } else {
        CGAffineTransform {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            tx: -extent.origin.x * sx,
            ty: -extent.origin.y * sy,
        }
    }
}

/// Hides a layer's direct sublayers for one `renderInContext` and
/// restores each one's `hidden` on drop — the model-tree-only
/// `own_content_only` mechanism. Never reparents.
struct SublayerHidden {
    saved: Vec<(Retained<CALayer>, bool)>,
}

impl SublayerHidden {
    /// Sets `hidden` on every direct sublayer, remembering each flag.
    fn hide_all(layer: &CALayer) -> Self {
        // SAFETY: the layer outlives the returned array.
        let saved = unsafe { layer.sublayers() }.map_or_else(Vec::new, |sublayers| {
            sublayers
                .iter()
                .map(|sublayer| {
                    let was = sublayer.isHidden();
                    sublayer.setHidden(true);
                    (sublayer, was)
                })
                .collect()
        });
        Self { saved }
    }
}

impl Drop for SublayerHidden {
    fn drop(&mut self) {
        for (sublayer, was) in self.saved.drain(..) {
            sublayer.setHidden(was);
        }
    }
}

/// Pins a layer's `opacity` to 1 for one raster and restores it on
/// drop — group-internal own content must draw at full strength because
/// the group's composite applies the layer's own opacity itself.
struct LayerOpacity {
    layer: Retained<CALayer>,
    was: f32,
}

impl LayerOpacity {
    /// Forces `layer`'s opacity to full, remembering the original.
    fn full(layer: &Retained<CALayer>) -> Self {
        let was = layer.opacity();
        layer.setOpacity(1.0);
        Self {
            layer: layer.clone(),
            was,
        }
    }
}

impl Drop for LayerOpacity {
    fn drop(&mut self) {
        self.layer.setOpacity(self.was);
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

    /// Rasterizes `segment`'s layer into the leased destination — the
    /// draw half of a capture, infallible once the frame exists.
    fn draw(&self, segment: &RasterSegment) {
        self.frame
            .as_ref()
            .expect("a lease owns its frame")
            .draw(segment);
    }

    /// Hands the frame back — only the settle path may call this.
    fn into_frame(mut self) -> NativeRasterFrame {
        self.frame.take().expect("a lease owns its frame")
    }
}

/// The key a raster pool generation runs under: the caller's generation
/// token plus the destination's device and pixel format. Any change
/// retires the whole pool; within one key frames are bucketed by pixel
/// size so a plan with segments of several sizes shares the pool.
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
/// small pool of `NativeRasterFrame`s leased per capture and returned
/// when the GPU settles, all under one current [`RasterKey`].
#[derive(Debug, Default)]
struct NativeRenderer {
    /// The key the pool currently issues for. Any mismatch drains
    /// `available`: storage from another key — an old generation, a
    /// replaced device, a different format — can never answer this
    /// key's capture.
    key: Option<RasterKey>,
    /// Settled frames under `key` bucketed by `(width, height)` — a
    /// paint-order plan issues one segment per native node, at as many
    /// sizes.
    available: HashMap<(usize, usize), Vec<NativeRasterFrame>>,
}

impl NativeRenderer {
    /// Issues a `width` × `height` frame under the pool's key — reusing
    /// a same-size settled frame — and returns the lease owning it
    /// through consumption.
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
        };
        if self.key != Some(key) {
            self.key = Some(key);
            self.available.clear();
        }
        let frame = self
            .available
            .get_mut(&(width, height))
            .and_then(Vec::pop)
            .unwrap_or_else(|| NativeRasterFrame::new(device, format, width, height, generation));
        RasterLease { frame: Some(frame) }
    }

    /// Takes a settled frame back — main thread only, called from the
    /// compositor's completion path once the GPU stopped sampling it. A
    /// frame whose key no longer matches the pool's current key is
    /// dropped instead: outstanding old-key storage never joins the new
    /// pool. A frame settling after the pool was reset (shutdown) is
    /// dropped without re-arming the pool — an outstanding capture must
    /// never resurrect lifetime the owner already ended.
    fn return_frame(&mut self, frame: NativeRasterFrame) {
        let Some(key) = &self.key else {
            return;
        };
        if frame.generation == key.generation
            && frame.pixel_format == key.pixel_format
            && Retained::as_ptr(&frame.texture.device()) == key.device.cast()
        {
            self.available
                .entry((frame.pixel_width, frame.pixel_height))
                .or_default()
                .push(frame);
        }
    }
}

/// A snapshot's three halves: the platform spec, the surface it came
/// from, and the resolved view's backing layer the paint-order plan
/// keys it by.
struct CapturedSnapshot {
    spec: SurfaceSpec,
    surface: Rc<dyn CapturableSurface>,
    layer: Retained<CALayer>,
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
/// compositor's queue needs. `rasters` are the leased native output —
/// one per `Native` segment the plan emitted, in plan order — and
/// `surfaces` the registrations this capture's immutable snapshot owns:
/// both must outlive the last GPU read, so the preparation only releases
/// them through the composite buffer's completion.
struct Preparation {
    target: Retained<ProtocolObject<dyn MTLTexture>>,
    rasters: Vec<RasterLease>,
    mask_rasters: Vec<RasterLease>,
    plan: CapturePlan,
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
    /// Transient textures the composite issued — pooled back when the
    /// buffer completes.
    transients: QueueSend<Vec<Retained<ProtocolObject<dyn MTLTexture>>>>,
    /// The compositor the transients return to.
    compositor: Compositor,
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

        // The raster pass lands its pixels in the leased frames' shared
        // buffers, not `target` — and it is synchronous: no GPU fence
        // stands in for it. Every capture still runs the composition
        // pass drawing the plan over `target`, external surfaces or
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
            );
            let snapshots = self.collect_snapshots(geometry);
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
            let mut suppression = SuppressionGuard::new(&snapshots);
            suppression.begin();
            // The paint-order plan reads the post-suppression model
            // tree: a surface's hidden presentation sublayers emit no
            // nodes and draw nothing.
            let plan = CapturePlan::build(
                &layer,
                snapshots
                    .iter()
                    .map(|snapshot| PlanSurface {
                        layer: snapshot.layer.clone(),
                        spec: snapshot.spec,
                    })
                    .collect(),
                layer.bounds(),
                CGSize::new(geometry.scale_x, geometry.scale_y),
            );
            let (device, format) = (target.device(), target.pixelFormat());
            let (rasters, mask_rasters) = {
                let mut renderer = self.renderer.borrow_mut();
                let rasters = plan
                    .segments
                    .iter()
                    .map(|segment| {
                        let (width, height) = segment.pixel_size(plan.scale);
                        let lease = renderer.issue(&device, format, width, height, generation);
                        lease.draw(segment);
                        lease
                    })
                    .collect::<Vec<_>>();
                let mask_rasters = plan
                    .mask_segments
                    .iter()
                    .map(|segment| {
                        let (width, height) = segment.pixel_size(plan.scale);
                        let lease = renderer.issue(&device, format, width, height, generation);
                        lease.draw(segment);
                        lease
                    })
                    .collect::<Vec<_>>();
                (rasters, mask_rasters)
            };
            suppression.end();
            let specs: Vec<SurfaceSpec> = snapshots.iter().map(|s| s.spec).collect();
            let preparation = Preparation {
                rasters,
                mask_rasters,
                plan,
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
    fn collect_snapshots(&self, geometry: CaptureGeometry) -> Vec<CapturedSnapshot> {
        let mut snapshots = Vec::new();
        self.collect_into(&self.content, &mut snapshots, geometry);
        snapshots
    }

    /// Recursion over `view`'s subviews: a resolved surface snapshots and
    /// stops the descent.
    fn collect_into(
        &self,
        view: &PlatformView,
        snapshots: &mut Vec<CapturedSnapshot>,
        geometry: CaptureGeometry,
    ) {
        if let Some(surface) = (self.resolve)(view) {
            // The spec covers the surface's whole content — the host
            // layer's bounds — and the plan places it; a
            // non-layer-backed resolved view cannot be captured.
            if let Some(layer) = crate::view::layer(view)
                && let Some(spec) = surface_spec(
                    view_key(view),
                    Size::new(layer.bounds().size.width, layer.bounds().size.height),
                    geometry,
                    surface.capture_pixel_format(),
                )
            {
                snapshots.push(CapturedSnapshot {
                    spec,
                    surface,
                    layer,
                });
            }
            return;
        }
        for subview in crate::view::subviews(view) {
            self.collect_into(&subview, snapshots, geometry);
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
                    // release its lease unreturned. Destruction AND the
                    // failure report both happen on the main queue, in
                    // that order: the completion contract is main-thread,
                    // and the caller settles only after the leased
                    // cleanup it owns has run.
                    enqueue(move |_| {
                        drop(preparation);
                        completion(false);
                    });
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
        let settle_compositor = compositor.clone();
        compositor.perform(move |guard| {
            let command_buffer = guard.make_command_buffer(&preparation.get().device);
            // Every CPU-drawn raster reaches its private texture
            // through a blit on this same command buffer — each encoder
            // is ended before the render pass samples it.
            for raster in &preparation.get().rasters {
                raster.encode_transfer(&command_buffer);
            }
            for raster in &preparation.get().mask_rasters {
                raster.encode_transfer(&command_buffer);
            }
            let transients =
                guard.encode_composition(preparation.get(), rendered.get(), &command_buffer);
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
                transients: QueueSend(transients),
                compositor: settle_compositor,
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
                    let status = buffer.status();
                    let completed = status == MTLCommandBufferStatus::Completed;
                    // Status is the authoritative outcome; the `NSError`
                    // payload is optional. A non-completed buffer always
                    // reports — with the error's description when there
                    // is one, with the status itself when there isn't.
                    let error = (!completed).then(|| {
                        buffer.error().map_or_else(
                            || format!("command buffer ended with status {status:?}"),
                            |error| error.localizedDescription().to_string(),
                        )
                    });
                    let settle = settle.lock().expect("capture lock").take();
                    if let Some(settle) = settle {
                        enqueue(move |mtm| {
                            let Settle {
                                preparation,
                                transients,
                                compositor,
                                return_to,
                                completion,
                            } = settle;
                            let preparation = preparation.0;
                            if completed {
                                // The composite buffer finished: pooled
                                // group transients return on the
                                // compositor's own queue.
                                compositor.perform(move |guard| {
                                    guard.return_transients(transients.into_inner());
                                });
                                if let Some(capture) = return_to.get(mtm).upgrade() {
                                    let mut pool = capture.renderer.borrow_mut();
                                    for lease in preparation.rasters {
                                        pool.return_frame(lease.into_frame());
                                    }
                                    for lease in preparation.mask_rasters {
                                        pool.return_frame(lease.into_frame());
                                    }
                                }
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
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_metal::{
        MTLCopyAllDevices, MTLCreateSystemDefaultDevice, MTLPixelFormat, MTLResource, MTLSize,
        MTLTexture,
    };
    use objc2_quartz_core::{CACornerMask, CALayer, CATransform3D};

    use super::{
        CaptureDeferred, CaptureNode, CapturePlan, CompositorState, FenceBatch, NativeRenderer,
        PlanSurface, RasterSegment, SurfaceSpec, layer_key, project_point,
    };
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
        // Same-generation frames of different sizes coexist — each
        // returns to its own bucket, so a segmented plan keeps every
        // segment size pooled at once.
        let wide_a = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 128, 96, 1);
        let wide_b = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 128, 96, 1);
        let narrow = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 1);
        renderer.return_frame(wide_a.into_frame());
        renderer.return_frame(wide_b.into_frame());
        assert_eq!(
            renderer.available.values().map(Vec::len).sum::<usize>(),
            2,
            "same-generation frames return to their own size bucket"
        );
        // A generation bump retires every bucket at once: the
        // still-outstanding `narrow` must never rejoin.
        let _next = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 2);
        renderer.return_frame(narrow.into_frame());
        assert!(
            renderer.available.is_empty(),
            "a superseded generation's frame must never rejoin the pool"
        );
        // And a pool the owner ended (shutdown) drops a still-outstanding
        // frame instead of re-arming — a post-teardown settle is
        // reachable, so it must be quiet, not a panic.
        let last = renderer.issue(&device, MTLPixelFormat::BGRA8Unorm, 64, 48, 2);
        renderer = NativeRenderer::default();
        renderer.return_frame(last.into_frame());
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

    /// A `CALayer` at `position` with `size` bounds and the default
    /// centered anchor.
    fn layer(x: f64, y: f64, w: f64, h: f64) -> Retained<CALayer> {
        let layer = CALayer::layer();
        layer.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w, h)));
        layer.setPosition(CGPoint::new(x, y));
        layer
    }

    /// The `PlanSurface` `layer` resolves — its spec points at a
    /// synthetic 4×4 producer texture.
    fn surface_for(layer: &Retained<CALayer>) -> PlanSurface {
        PlanSurface {
            layer: layer.clone(),
            spec: SurfaceSpec {
                surface_id: layer_key(layer),
                size: MTLSize {
                    width: 4,
                    height: 4,
                    depth: 1,
                },
                pixel_format: MTLPixelFormat::BGRA8Unorm,
            },
        }
    }

    /// Builds the plan for `root` at unit scale over its own bounds.
    fn plan(root: &Retained<CALayer>, surfaces: Vec<PlanSurface>) -> CapturePlan {
        CapturePlan::build(root, surfaces, root.bounds(), CGSize::new(1.0, 1.0))
    }

    /// The `RasterSegment` `index` refers to — the layer it rasters.
    fn segment_layer(plan: &CapturePlan, index: usize) -> Retained<CALayer> {
        plan.segments[index].layer.clone()
    }

    /// A tree without resolved surfaces stays one raster, as before —
    /// nested native content needs no segmentation.
    #[test]
    fn a_native_only_subtree_collapses_to_one_raster() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let child = layer(10.0, 10.0, 40.0, 40.0);
        root.addSublayer(&child);
        child.addSublayer(&layer(5.0, 5.0, 10.0, 10.0));
        let plan = plan(&root, Vec::new());
        assert_eq!(plan.nodes.len(), 1);
        assert_eq!(plan.segments.len(), 1);
        let [CaptureNode::Native { raster, .. }] = plan.nodes.as_slice() else {
            panic!("a native-only tree must emit exactly one Native node");
        };
        assert_eq!(*raster, 0);
        assert!(
            !plan.segments[0].own_content_only,
            "the collapsed raster draws the whole subtree"
        );
        assert_eq!(layer_key(&segment_layer(&plan, 0)), layer_key(&root));
    }

    /// The defect's core case: the opaque root's own content paints
    /// under the GPU child, the native foreground over it — every
    /// native ancestor with GPU descendants splits into own content
    /// plus children in paint order.
    #[test]
    fn an_opaque_root_paints_between_and_around_a_gpu_child() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let host = layer(10.0, 10.0, 40.0, 40.0);
        let front = layer(50.0, 50.0, 20.0, 20.0);
        root.addSublayer(&host);
        root.addSublayer(&front);
        let plan = plan(&root, vec![surface_for(&host)]);

        // [root own content, host own content, surface, foreground].
        assert_eq!(plan.nodes.len(), 4);
        let [
            CaptureNode::Native { raster: r0, .. },
            CaptureNode::Native { raster: r1, .. },
            CaptureNode::Surface { spec, .. },
            CaptureNode::Native { raster: r3, .. },
        ] = plan.nodes.as_slice()
        else {
            panic!(
                "expected [root own content, host own content, surface, foreground], \
                 got {:?}",
                plan.nodes
            );
        };
        assert!(plan.segments[*r0].own_content_only);
        assert!(plan.segments[*r1].own_content_only);
        assert!(!plan.segments[*r3].own_content_only);
        assert_eq!(layer_key(&segment_layer(&plan, *r0)), layer_key(&root));
        assert_eq!(layer_key(&segment_layer(&plan, *r1)), layer_key(&host));
        assert_eq!(layer_key(&segment_layer(&plan, *r3)), layer_key(&front));
        assert_eq!(spec.surface_id, layer_key(&host));
        // The surface's quad covers the host's bounds projected to
        // root space: position (10,10) anchored at the center.
        let CaptureNode::Surface {
            transform,
            source_rect,
            ..
        } = &plan.nodes[2]
        else {
            unreachable!();
        };
        assert_eq!(*source_rect, host.bounds());
        let projected = project_point(CGPoint::new(0.0, 0.0), transform)
            .expect("an affine host projects every corner");
        let expected = host.convertPoint_toLayer(CGPoint::new(0.0, 0.0), Some(&root));
        assert!(
            (projected.x - expected.x).abs() < 1e-6 && (projected.y - expected.y).abs() < 1e-6,
            "the surface quad must land where the host layer paints"
        );
    }

    /// Group opacity wraps the GPU subtree in an offscreen group: its
    /// children see a clean opacity accumulator and the group's own
    /// draw applies the ancestor's opacity once.
    #[test]
    fn group_opacity_wraps_a_gpu_subtree() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let mid = layer(20.0, 20.0, 60.0, 60.0);
        mid.setOpacity(0.5);
        let host = layer(10.0, 10.0, 20.0, 20.0);
        mid.addSublayer(&host);
        root.addSublayer(&mid);
        let plan = plan(&root, vec![surface_for(&host)]);

        // [root own content, Group{[mid own content, host own content,
        // surface]}] — the group's children carry no ancestor opacity.
        assert_eq!(plan.nodes.len(), 2);
        let CaptureNode::Group(group) = &plan.nodes[1] else {
            panic!("a translucent group-opacity ancestor must emit a Group");
        };
        assert!(
            (group.opacity - 0.5).abs() < 1e-6,
            "the group draw applies the ancestor's opacity once"
        );
        assert_eq!(group.children.len(), 3);
        let CaptureNode::Surface { opacity, .. } = &group.children[2] else {
            panic!("the group's last child must be the surface");
        };
        assert!(
            (*opacity - 1.0).abs() < 1e-6,
            "children of a group see a clean opacity accumulator"
        );
        // The group layer's own-content raster suppresses its opacity —
        // the group draw supplies it.
        let CaptureNode::Native { raster, .. } = &group.children[0] else {
            panic!("the group's first child must be its own content");
        };
        assert!(
            plan.segments[*raster].suppress_opacity,
            "the own-content raster must not double-apply the group opacity"
        );
    }

    /// `zPosition` decides sibling paint order, ties keeping the array
    /// order — the plan emits nodes in that order.
    #[test]
    fn sublayers_paint_in_z_order() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let back = layer(5.0, 5.0, 30.0, 30.0);
        back.setZPosition(10.0);
        let front = layer(60.0, 60.0, 20.0, 20.0);
        front.setZPosition(-5.0);
        let host = layer(30.0, 30.0, 20.0, 20.0); // z = 0, default
        root.addSublayer(&back);
        root.addSublayer(&front);
        root.addSublayer(&host);
        let plan = plan(&root, vec![surface_for(&host)]);

        // Paint order front(-5) < host(0) < back(10):
        // [root own, front, host own, surface, back].
        assert_eq!(plan.nodes.len(), 5);
        let CaptureNode::Native { raster: first, .. } = &plan.nodes[1] else {
            panic!("the z-lowest sibling must paint first");
        };
        assert_eq!(layer_key(&segment_layer(&plan, *first)), layer_key(&front));
        let CaptureNode::Native { raster: last, .. } = &plan.nodes[4] else {
            panic!("the z-highest sibling must paint last");
        };
        assert_eq!(layer_key(&segment_layer(&plan, *last)), layer_key(&back));
        assert!(matches!(plan.nodes[3], CaptureNode::Surface { .. }));
    }

    /// A `masksToBounds` ancestor hands its rounded rect to descendants
    /// — the GPU child's quad gets the clip shape with the per-corner
    /// radii `maskedCorners` selects.
    #[test]
    fn a_masks_to_bounds_ancestor_clips_the_gpu_child() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let clipper = layer(20.0, 20.0, 50.0, 50.0);
        clipper.setMasksToBounds(true);
        clipper.setCornerRadius(12.0);
        clipper.setMaskedCorners(CACornerMask::LayerMinXMinYCorner);
        let host = layer(5.0, 5.0, 20.0, 20.0);
        clipper.addSublayer(&host);
        root.addSublayer(&clipper);
        let plan = plan(&root, vec![surface_for(&host)]);

        let clip = plan
            .nodes
            .iter()
            .find_map(|node| match node {
                CaptureNode::Surface { clip, .. } => Some(clip),
                _ => None,
            })
            .expect("the host must emit a Surface node");
        assert_eq!(clip.len(), 1);
        let shape = &clip[0];
        assert_eq!(shape.bounds, clipper.bounds());
        assert!(
            (shape.radii[0] - 12.0).abs() < 1e-6 && shape.radii[1..] == [0.0; 3],
            "only the masked corner must carry the radius: {:?}",
            shape.radii
        );
        // The clip's inverse maps root space back into the clipper's
        // local space — the bounds' origin sits at (-5,-5) in root
        // space (position 20 less the centered anchor's 25) and must
        // project back to (0,0).
        let local = project_point(CGPoint::new(-5.0, -5.0), &shape.inverse)
            .expect("an affine clip inverse projects");
        assert!(local.x.abs() < 1e-6 && local.y.abs() < 1e-6);
    }

    /// The walk's accumulated transform is exactly what `convertPoint`
    /// computes — position, anchor, `transform`, and `sublayerTransform`
    /// composed in Core Animation's order.
    #[test]
    fn the_walk_transform_matches_convert_point() {
        let root = layer(0.0, 0.0, 200.0, 200.0);
        let mid = layer(40.0, 50.0, 100.0, 100.0);
        mid.setAnchorPoint(CGPoint::new(0.25, 0.5));
        mid.setSublayerTransform(CATransform3D::new_scale(1.5, 1.5, 1.0));
        let host = layer(10.0, 10.0, 30.0, 30.0);
        host.setTransform(CATransform3D::new_rotation(0.4, 0.0, 0.0, 1.0));
        mid.addSublayer(&host);
        root.addSublayer(&mid);
        let plan = plan(&root, vec![surface_for(&host)]);

        let transform = plan
            .nodes
            .iter()
            .find_map(|node| match node {
                CaptureNode::Surface { transform, .. } => Some(*transform),
                _ => None,
            })
            .expect("the host must emit a Surface node");
        for point in [
            CGPoint::new(0.0, 0.0),
            CGPoint::new(15.0, 7.0),
            CGPoint::new(30.0, 30.0),
        ] {
            let expected = host.convertPoint_toLayer(point, Some(&root));
            let got = project_point(point, &transform).expect("the point projects");
            assert!(
                (got.x - expected.x).abs() < 1e-6 && (got.y - expected.y).abs() < 1e-6,
                "walk transform {point:?}: got {got:?}, convertPoint says {expected:?}"
            );
        }
    }

    /// A masked ancestor becomes a `Group` whose draw samples a
    /// rasterized alpha mask — the mask segment rasters the mask layer
    /// through its own boundary transform.
    #[test]
    fn a_masked_ancestor_becomes_a_group_with_a_mask_raster() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let mid = layer(20.0, 20.0, 60.0, 60.0);
        let mask = layer(0.0, 0.0, 60.0, 60.0);
        // SAFETY: both layers are freshly-created model layers this test
        // owns; `setMask` takes an optional layer reference.
        unsafe { mid.setMask(Some(&mask)) };
        let host = layer(10.0, 10.0, 20.0, 20.0);
        mid.addSublayer(&host);
        root.addSublayer(&mid);
        let plan = plan(&root, vec![surface_for(&host)]);

        let group = plan
            .nodes
            .iter()
            .find_map(|node| match node {
                CaptureNode::Group(group) => Some(group),
                _ => None,
            })
            .expect("a masked ancestor must emit a Group");
        assert_eq!(group.mask, Some(0));
        assert_eq!(plan.mask_segments.len(), 1);
        let RasterSegment {
            layer: mask_layer,
            space_transform,
            own_content_only,
            ..
        } = &plan.mask_segments[0];
        assert_eq!(layer_key(mask_layer), layer_key(&mask));
        assert!(space_transform.is_some());
        assert!(!*own_content_only);
    }

    /// Hidden and fully transparent subtrees emit nothing — a hidden
    /// host's surface never reaches the plan.
    #[test]
    fn hidden_and_transparent_subtrees_emit_nothing() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let hidden_host = layer(10.0, 10.0, 20.0, 20.0);
        hidden_host.setHidden(true);
        let faint_host = layer(50.0, 50.0, 20.0, 20.0);
        faint_host.setOpacity(0.0);
        root.addSublayer(&hidden_host);
        root.addSublayer(&faint_host);
        let plan = plan(
            &root,
            vec![surface_for(&hidden_host), surface_for(&faint_host)],
        );
        assert_eq!(plan.nodes.len(), 1);
        assert!(matches!(plan.nodes[0], CaptureNode::Native { .. }));
    }

    /// A subtree entirely outside the pass extent emits no raster — the
    /// offscreen native child never reaches the plan.
    #[test]
    fn offscreen_subtrees_are_pruned() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let host = layer(10.0, 10.0, 30.0, 30.0);
        let offscreen = layer(500.0, 500.0, 50.0, 50.0);
        root.addSublayer(&host);
        root.addSublayer(&offscreen);
        let plan = plan(&root, vec![surface_for(&host)]);
        assert!(
            !plan
                .segments
                .iter()
                .any(|segment| { layer_key(&segment.layer) == layer_key(&offscreen) }),
            "a subtree outside the pass extent must emit no raster"
        );
        assert!(
            plan.nodes
                .iter()
                .any(|node| { matches!(node, CaptureNode::Surface { .. }) })
        );
    }

    /// A flipped host marks its surface `source_flip` — the producer
    /// texture's top edge is the host's `bounds.min.y`, not `max.y`.
    #[test]
    fn a_flipped_host_marks_its_surface_source_flip() {
        let root = layer(0.0, 0.0, 100.0, 100.0);
        let host = layer(10.0, 10.0, 30.0, 30.0);
        host.setGeometryFlipped(true);
        root.addSublayer(&host);
        let plan = plan(&root, vec![surface_for(&host)]);
        let flipped = plan
            .nodes
            .iter()
            .find_map(|node| match node {
                CaptureNode::Surface { source_flip, .. } => Some(*source_flip),
                _ => None,
            })
            .expect("the host must emit a Surface node");
        assert!(flipped);
    }
}
