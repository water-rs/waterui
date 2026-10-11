//! The Apple realization of system-compositor planes: on macOS a
//! hierarchy of engine-owned `NSView`s under the host view, whose paint
//! order is the subview order; on iOS a Core Animation layer tree the
//! engine owns under the host view's backing layer.
//!
//! ```text
//! host view
//! ├─ probes view                   layer-hosting, pending displays inside
//! ├─ part 0 view                   layer-hosting, flipped, surface in points
//! │  └─ part 0: CAMetalLayer     engine content below the first plane
//! ├─ plane 0 top                   layer-hosting or layer-backed view
//! │  └─ level … level              node/views per tree layer on the path
//! │     └─ display / leaf            transform → clip → scroll
//! ├─ part 1 view
//! │  └─ part 1: CAMetalLayer     engine content above plane 0
//! └─ …
//! ```
//!
//! On macOS every top-level element is its own `NSView` under the host
//! view and `addSubview:positioned:relativeTo:` is the only ordering —
//! no view's layer is added, removed or reordered through `CALayer`
//! APIs. Parts and frame planes are layer-hosting views: the engine
//! creates the `CALayer`, hands it to `setLayer` before `setWantsLayer`
//! and owns its contents, so engine-owned standalone sublayers (a
//! part's metal layer, a plane's level chain) live only inside them.
//! A hosted plane's clip views and leaf are layer-backed views whose
//! layers `AppKit` owns; the hosted view is the leaf's only subview.
//! Apple's *Rules for Modifying Layers in OS X* forbid setting
//! `anchorPoint`, `bounds`, `frame`, `position`, `hidden` or
//! `transform` on a layer-backed view's layer, so a hosted `NSView` is
//! placed through the view API — translation in a frame origin, scroll in
//! `setBoundsOrigin`, a positive axis-aligned scale in `setBoundsSize`,
//! clips through the frame plus `masksToBounds`/corner properties on the
//! engine views' own layers, opacity through `alphaValue`. On iOS the
//! whole tree keeps the single `root` layer under the host and the
//! layer-based hosted realization.
//!
//! A promoted external frame is shown by an `AVSampleBufferDisplayLayer`
//! fed a `CVPixelBuffer` that wraps the frame's own `IOSurface`, not by an
//! IOSurface-backed `CALayer`. The display layer is the documented path for
//! uncompressed video frames: it reads the `CVImageBuffer` colour
//! attachments (matrix, primaries, transfer, chroma siting), so the system
//! applies its EDR tone mapping to PQ and HLG content, while a `CALayer`'s
//! `IOSurface` contents have no documented HDR metadata path (`CAEDRMetadata`
//! exists only on `CAMetalLayer`). It is also the only layer `FairPlay`
//! decrypts into, so protected frames (#212) reuse this realization.
//!
//! A candidate's display layer is created pending — beside `root` at zero
//! bounds, invisible but inside the committed layer hierarchy — because
//! the platform reports `readyForDisplay` only for a layer the render
//! server can see holding a committed frame. Promotion waits for that
//! readiness: the property is not key-value observable, so the display
//! posts `AVSampleBufferDisplayLayerReadyForDisplayDidChangeNotification`,
//! whose handler re-reads it on main and wakes the render loop. A demotion
//! detaches its display inside the next `place` transaction.
//!
//! Engine parts present through `CAMetalLayer`s with
//! `presentsWithTransaction`, and every geometry change, part presentation
//! and frame hand-off of one frame commits in one `CATransaction`, so parts
//! and planes change on screen together.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak, mpsc};

use dispatch2::{DispatchQueue, MainThreadBound};
use kurbo::{Affine, Rect, Size, Vec2};
#[cfg(target_os = "macos")]
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::NSView;
use objc2_av_foundation::{
    AVLayerVideoGravityResize, AVQueuedSampleBufferRendering, AVQueuedSampleBufferRenderingStatus,
    AVSampleBufferDisplayLayer, AVSampleBufferDisplayLayerReadyForDisplayDidChangeNotification,
};
use objc2_core_foundation::{
    CFBoolean, CFMutableDictionary, CFRetained, CFString, CGAffineTransform, CGPoint, CGRect,
    CGSize,
};
use objc2_core_media::{
    CMSampleBuffer, CMSampleTimingInfo, CMVideoFormatDescription,
    CMVideoFormatDescriptionCreateForImageBuffer, kCMSampleAttachmentKey_DisplayImmediately,
    kCMTimeInvalid,
};
use objc2_core_video::{
    CVAttachmentMode, CVPixelBuffer, CVPixelBufferCreateWithIOSurface,
    kCVImageBufferChromaLocation_Center, kCVImageBufferChromaLocation_Left,
    kCVImageBufferChromaLocation_Top, kCVImageBufferChromaLocation_TopLeft,
    kCVImageBufferChromaLocationBottomFieldKey, kCVImageBufferChromaLocationTopFieldKey,
    kCVImageBufferColorPrimaries_ITU_R_709_2, kCVImageBufferColorPrimaries_ITU_R_2020,
    kCVImageBufferColorPrimaries_P3_D65, kCVImageBufferColorPrimariesKey,
    kCVImageBufferTransferFunction_ITU_R_709_2, kCVImageBufferTransferFunction_ITU_R_2100_HLG,
    kCVImageBufferTransferFunction_Linear, kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ,
    kCVImageBufferTransferFunction_sRGB, kCVImageBufferTransferFunctionKey,
    kCVImageBufferYCbCrMatrix_ITU_R_601_4, kCVImageBufferYCbCrMatrix_ITU_R_709_2,
    kCVImageBufferYCbCrMatrix_ITU_R_2020, kCVImageBufferYCbCrMatrixKey, kCVPixelFormatType_32BGRA,
    kCVPixelFormatType_64RGBAHalf, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange, kCVReturnSuccess,
};
#[cfg(not(target_os = "macos"))]
use objc2_foundation::NSArray;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol};
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{MTLSharedEvent, MTLSharedEventListener, MTLTexture};
use objc2_quartz_core::{
    CACornerMask, CALayer, CAMetalLayer, CATransaction, kCACornerCurveCircular,
    kCACornerCurveContinuous,
};
use rustc_hash::{FxHashMap, FxHashSet};

use cherenkov::{ContinuousRect, LayerId, RenderError, ShapeData, SurfaceError};

use super::{Composition, Compositor, Level, Placement, Plane, PlaneContent, Source, SystemPlanes};
use crate::interop::{
    ChromaOffset, ExternalFrame, FramePlanes, FrameSync, Primaries, RgbAlpha, Transfer, YuvMatrix,
    YuvRange,
};
use crate::render::present::{OutputRequest, WindowSurface};

mod animation;
mod raster;
/// The rendered producer's scan-out frame ring.
pub(in crate::render) mod ring;

/// Main-thread storage with asynchronous destruction. Every reference, including
/// the last one, is released on main; `MainThreadBound::drop` can never dispatch
/// synchronously from the render thread.
struct MainOwned<T: 'static>(Option<Arc<MainThreadBound<RefCell<T>>>>);

impl<T> MainOwned<T> {
    fn new(value: T, mtm: MainThreadMarker) -> Self {
        Self(Some(Arc::new(MainThreadBound::new(
            RefCell::new(value),
            mtm,
        ))))
    }

    fn run(&self, f: impl FnOnce(&mut T, MainThreadMarker) + Send + 'static) {
        let owned = self.clone();
        DispatchQueue::main().exec_async(move || {
            let mtm = MainThreadMarker::new().expect("the main dispatch queue");
            f(
                &mut owned
                    .0
                    .as_ref()
                    .expect("live main owner")
                    .get(mtm)
                    .borrow_mut(),
                mtm,
            );
        });
    }

    /// Borrows the main-thread value on a thread that is already main —
    /// the frame's commit applies through it inside its reply window,
    /// instead of queueing a block the next acquire could outrun.
    fn with<R>(&self, mtm: MainThreadMarker, f: impl FnOnce(&mut T, MainThreadMarker) -> R) -> R {
        f(
            &mut self
                .0
                .as_ref()
                .expect("live main owner")
                .get(mtm)
                .borrow_mut(),
            mtm,
        )
    }

    /// A weak handle to the same storage, for a callback the scene itself
    /// retains — an observer must not keep its owner alive.
    fn downgrade(&self) -> Weak<MainThreadBound<RefCell<T>>> {
        Arc::downgrade(self.0.as_ref().expect("live main owner"))
    }
}

impl<T> Clone for MainOwned<T> {
    fn clone(&self) -> Self {
        Self(Some(Arc::clone(self.0.as_ref().expect("live main owner"))))
    }
}

impl<T> Drop for MainOwned<T> {
    fn drop(&mut self) {
        let value = self.0.take().expect("one release per main owner");
        if MainThreadMarker::new().is_some() {
            drop(value);
        } else {
            DispatchQueue::main().exec_async(move || drop(value));
        }
    }
}

/// A `CALayer` the host supplies, shown on a plane of its own
/// (`cherenkov::HostedLayers`). The iOS hosted object.
///
/// Captured on main, where the layer stays: the render thread holds only
/// this handle, and its last release goes back to main. The view that owns
/// the layer stays in the host's view hierarchy, so it keeps receiving
/// events and first-responder status; while the layer is bound, the engine
/// sets its superlayer, anchor point, position and bounds size, and the
/// host sets everything else — its contents, sublayers, bounds origin and
/// transform.
#[cfg(not(target_os = "macos"))]
#[derive(Clone)]
pub struct HostedLayer {
    layer: MainOwned<Retained<CALayer>>,
    /// The layer's address: its identity, readable off main.
    address: usize,
}

#[cfg(not(target_os = "macos"))]
impl HostedLayer {
    /// Captures `layer` on main.
    #[must_use]
    pub fn new(layer: Retained<CALayer>, mtm: MainThreadMarker) -> Self {
        let address = Retained::as_ptr(&layer).addr();
        Self {
            layer: MainOwned::new(layer, mtm),
            address,
        }
    }

    /// Whether `self` and `other` hold the same `CALayer`.
    #[must_use]
    pub const fn is(&self, other: &Self) -> bool {
        self.address == other.address
    }

    /// The layer, on main.
    fn layer(&self, mtm: MainThreadMarker) -> Retained<CALayer> {
        self.layer
            .0
            .as_ref()
            .expect("live main owner")
            .get(mtm)
            .borrow()
            .clone()
    }
}

#[cfg(not(target_os = "macos"))]
impl std::fmt::Debug for HostedLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("HostedLayer")
            .field(&format_args!("{:#x}", self.address))
            .finish()
    }
}

/// An `NSView` the host supplies, shown on a plane of its own
/// (`cherenkov::HostedLayers`). The macOS hosted object.
///
/// Captured on main, where the view stays: the render thread holds only
/// this handle, and its last release goes back to main. The engine places
/// the view inside its own flipped views through the view API — a subview
/// of the leaf at its extent — because `AppKit` owns a layer-backed view's
/// layer: *Rules for Modifying Layers in OS X* forbid the superlayer,
/// anchor point, position and bounds writes the iOS realization performs
/// on a `CALayer`.
#[cfg(target_os = "macos")]
#[derive(Clone)]
pub struct HostedView {
    view: MainOwned<Retained<NSView>>,
    /// The view's address: its identity, readable off main.
    address: usize,
}

#[cfg(target_os = "macos")]
impl HostedView {
    /// Captures `view` on main.
    #[must_use]
    pub fn new(view: Retained<NSView>, mtm: MainThreadMarker) -> Self {
        let address = Retained::as_ptr(&view).addr();
        Self {
            view: MainOwned::new(view, mtm),
            address,
        }
    }

    /// Whether `self` and `other` hold the same `NSView`.
    #[must_use]
    pub const fn is(&self, other: &Self) -> bool {
        self.address == other.address
    }

    /// The view, on main.
    fn view(&self, mtm: MainThreadMarker) -> Retained<NSView> {
        self.view
            .0
            .as_ref()
            .expect("live main owner")
            .get(mtm)
            .borrow()
            .clone()
    }
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for HostedView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("HostedView")
            .field(&format_args!("{:#x}", self.address))
            .finish()
    }
}

// An `NSView` the engine places: flipped, so its coordinate space matches
// the engine's y-down space, and transparent to clicks — `hitTest:`
// declines the view itself while its subviews stay hittable. Parts and
// frame planes are engine views too, so the engine never answers a hit:
// a click over engine content reaches the hosted view below it or the
// host view, and which content occludes a hosted view is the host's
// decision, made in the view it hosts.
#[cfg(target_os = "macos")]
objc2::define_class!(
    // SAFETY: `NSView` has no subclassing requirements for a flipped,
    // pass-through container; the class implements no Drop — its ivars
    // release on dealloc.
    #[unsafe(super(NSView))]
    #[name = "CherenkovEngineView"]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ()]
    struct EngineView;

    impl EngineView {
        /// The engine's space is y-down.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// A view that never answers a hit itself: `AppKit` walks
        /// subviews first, so a click inside a hosted view lands on it
        /// and every other click falls through to the host view below.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: CGPoint) -> *mut NSView {
            // SAFETY: `hitTest:` returns a view the hierarchy owns; a
            // borrowed pointer is what the method contract carries.
            let hit: *mut NSView = unsafe { objc2::msg_send![super(self), hitTest: point] };
            if hit == std::ptr::from_ref(self).cast_mut().cast() {
                std::ptr::null_mut()
            } else {
                hit
            }
        }
    }
);

/// A new, unconfigured [`EngineView`].
#[cfg(target_os = "macos")]
fn new_engine_view(mtm: MainThreadMarker) -> Retained<NSView> {
    let this = EngineView::alloc(mtm).set_ivars(());
    // SAFETY: `msg_send!` to `super.init` is the designated superclass
    // initializer for a `define_class!` type.
    let view: Retained<EngineView> = unsafe { objc2::msg_send![super(this), init] };
    let view = Retained::into_super(view);
    view.setAutoresizesSubviews(false);
    view
}

/// A new engine-owned layer-backed view: flipped, click-transparent, and
/// never resizing its subviews — a leaf frame change must leave the
/// hosted view's frame where `place` put it.
#[cfg(target_os = "macos")]
fn engine_view(mtm: MainThreadMarker) -> Retained<NSView> {
    let view = new_engine_view(mtm);
    view.setWantsLayer(true);
    view
}

/// A new layer-hosting part or plane view: the engine's own `layer` is
/// the view's layer, set before `wantsLayer` so the view hosts rather
/// than backs it. It never gets subviews, so — an [`EngineView`] that
/// declines hits itself — it is transparent to every click.
#[cfg(target_os = "macos")]
fn part_view(mtm: MainThreadMarker, layer: &CALayer) -> Retained<NSView> {
    let view = new_engine_view(mtm);
    view.setLayer(Some(layer));
    view.setWantsLayer(true);
    view
}

/// A hosted layer inside the engine's holder: the holder is the plane's
/// leaf, carrying its extent, opacity and opacity animation, so the
/// host's layer keeps its own.
#[cfg(not(target_os = "macos"))]
struct HostedNode {
    holder: Retained<CALayer>,
    layer: Retained<CALayer>,
    extent: Size,
}

#[cfg(not(target_os = "macos"))]
impl HostedNode {
    /// Places the host's layer at the holder's origin at its extent.
    fn place(&self) {
        if self.layer.superlayer().as_deref() != Some(&*self.holder) {
            self.holder.addSublayer(&self.layer);
        }
        let origin = self.layer.bounds().origin;
        self.layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
        self.layer.setPosition(CGPoint::new(0.0, 0.0));
        self.layer.setBounds(CGRect::new(
            origin,
            CGSize::new(self.extent.width, self.extent.height),
        ));
    }

    /// Takes the host's layer out of the engine's tree — unless it already
    /// moved to another holder, which then owns its placement.
    fn release(&self) {
        if self.layer.superlayer().as_deref() == Some(&*self.holder) {
            self.layer.removeFromSuperlayer();
        }
        self.holder.removeFromSuperlayer();
    }
}

/// A hosted view inside the engine's leaf: the leaf is the plane's last
/// view, carrying the extent and opacity, so the host's view keeps its
/// own geometry and responder state.
#[cfg(target_os = "macos")]
struct HostedNode {
    leaf: Retained<NSView>,
    view: Retained<NSView>,
    extent: Size,
}

#[cfg(target_os = "macos")]
impl HostedNode {
    /// Makes the host's view the leaf's only subview, at the leaf's
    /// origin and the extent — the leaf's frame already carries the
    /// scroll's shift.
    fn place(&self) {
        // SAFETY: AppKit accessors on the main thread.
        if unsafe { self.view.superview() }.as_deref() != Some(&*self.leaf) {
            self.leaf.addSubview(&self.view);
        }
        self.view.setFrame(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(self.extent.width, self.extent.height),
        ));
    }

    /// Takes the host's view out of the engine's tree — unless it already
    /// moved to another leaf, which then owns its placement.
    fn release(&self) {
        // SAFETY: AppKit accessors on the main thread.
        if unsafe { self.view.superview() }.as_deref() == Some(&*self.leaf) {
            self.view.removeFromSuperview();
        }
        self.leaf.removeFromSuperview();
    }
}

/// Makes `nodes` host exactly `hosted`: a tree layer no longer hosted lets
/// its object go, a new one gets a holder, a layer rebound to another
/// object swaps it inside the same holder, and every object is placed at
/// its extent — the holder, which is the plane's leaf, is never rebuilt
/// for a geometry change.
#[cfg(not(target_os = "macos"))]
fn host(
    nodes: &mut FxHashMap<LayerId, HostedNode>,
    hosted: Vec<(LayerId, Retained<CALayer>, Size)>,
) {
    nodes.retain(|id, node| {
        let keep = hosted.iter().any(|(layer, ..)| layer == id);
        if !keep {
            node.release();
        }
        keep
    });
    for (id, layer, extent) in hosted {
        let node = nodes.entry(id).or_insert_with(|| HostedNode {
            holder: anchored(),
            layer: layer.clone(),
            extent,
        });
        if Retained::as_ptr(&node.layer) != Retained::as_ptr(&layer) {
            if node.layer.superlayer().as_deref() == Some(&*node.holder) {
                node.layer.removeFromSuperlayer();
            }
            node.layer = layer;
        }
        node.extent = extent;
        node.place();
    }
}

/// The macOS `host`: a tree layer no longer hosted lets its view go, a
/// new one gets a leaf view, a rebound object swaps inside the same leaf,
/// and every view is placed at its extent — the leaf, which is the
/// plane's last view, is never rebuilt for a geometry change.
#[cfg(target_os = "macos")]
fn host(
    nodes: &mut FxHashMap<LayerId, HostedNode>,
    hosted: Vec<(LayerId, Retained<NSView>, Size)>,
    mtm: MainThreadMarker,
) {
    nodes.retain(|id, node| {
        let keep = hosted.iter().any(|(layer, ..)| layer == id);
        if !keep {
            node.release();
        }
        keep
    });
    for (id, view, extent) in hosted {
        let node = nodes.entry(id).or_insert_with(|| HostedNode {
            leaf: engine_view(mtm),
            view: view.clone(),
            extent,
        });
        if Retained::as_ptr(&node.view) != Retained::as_ptr(&view) {
            // SAFETY: AppKit accessors on the main thread.
            if unsafe { node.view.superview() }.as_deref() == Some(&*node.leaf) {
                node.view.removeFromSuperview();
            }
            node.view = view;
        }
        node.extent = extent;
        node.place();
    }
}

/// The hosted planes of a composition, owned for the main queue.
#[cfg(not(target_os = "macos"))]
fn hosted_planes(planes: &[Plane<'_>]) -> Vec<(LayerId, HostedLayer, Size)> {
    planes
        .iter()
        .filter_map(|plane| match &plane.content {
            PlaneContent::Hosted { object, extent } => {
                Some((plane.placement.layer, (*object).clone(), *extent))
            }
            PlaneContent::Raster { .. } | PlaneContent::Frame { .. } => None,
        })
        .collect()
}

/// The transform set a hosted plane's path may carry on macOS: a
/// translation plus a positive, finite, axis-aligned scale — what a
/// view's frame origin, bounds size and bounds origin express.
#[cfg(target_os = "macos")]
fn hosted_transform(transform: Affine) -> bool {
    let [sx, shear_y, shear_x, sy, tx, ty] = transform.as_coeffs();
    [sx, shear_y, shear_x, sy, tx, ty]
        .iter()
        .all(|v| v.is_finite())
        && shear_y == 0.0
        && shear_x == 0.0
        && sx > 0.0
        && sy > 0.0
}

/// Maps a rect in view coordinates to its frame in the parent view's
/// coordinates under `acc`, the accumulated content transform — always
/// a translation and axis-aligned scale by `hosted_transform`'s rule.
#[cfg(target_os = "macos")]
fn view_rect(acc: Affine, bounds: Rect) -> CGRect {
    let [a, _, _, d, x, y] = acc.as_coeffs();
    CGRect::new(
        CGPoint::new(a.mul_add(bounds.x0, x), d.mul_add(bounds.y0, y)),
        CGSize::new(a * bounds.width(), d * bounds.height()),
    )
}

/// Applies the clip's corner geometry to the clip view's own layer —
/// `masksToBounds`, `cornerRadius`, `maskedCorners` and `cornerCurve`,
/// none of which is on a layer-backed view's restricted list.
#[cfg(target_os = "macos")]
fn clip_view(view: &NSView, clip: &LayerClip) {
    let layer = view.layer().expect("a layer-backed engine view");
    layer.setMasksToBounds(true);
    layer.setCornerRadius(clip.radius);
    layer.setMaskedCorners(clip.corners);
    // SAFETY: the corner-curve constants are immutable statics.
    let curve = unsafe {
        if clip.continuous {
            kCACornerCurveContinuous
        } else {
            kCACornerCurveCircular
        }
    };
    layer.setCornerCurve(curve);
}

/// Adds `view` under `outer`. `AppKit` wires a layer-backed view's layer
/// into the window's layer tree itself — the engine never touches it
/// through `CALayer` APIs. Sublayer order inside a view is the subview
/// order `addSubview` keeps.
#[cfg(target_os = "macos")]
fn add_view(view: &NSView, outer: &NSView) {
    // SAFETY: AppKit accessors on the main thread.
    if unsafe { view.superview() }.as_deref() != Some(outer) {
        outer.addSubview(view);
    }
}

/// Places the plane's view chain on macOS: a clip view per clipped
/// level and the leaf, each carrying the transforms folded since the
/// previous view — translation in the frame origin, scroll in the
/// bounds origin, scale in the bounds size.
#[cfg(target_os = "macos")]
fn place_views(
    placement: &Placement,
    container: &NSView,
    clips: &[Option<Retained<NSView>>],
    node: &HostedNode,
) {
    // `acc` is the transform the next created view expresses: the
    // content transforms of every level since the last created view,
    // composed — eligibility keeps it a translation plus an
    // axis-aligned scale.
    let mut acc = Affine::IDENTITY;
    let mut outer: &NSView = container;
    let last = placement.path.len() - 1;
    for (index, level) in placement.path.iter().enumerate() {
        acc = acc * level.transform * Affine::translate(-level.scroll);
        if let Some(view) = clips[index].as_ref() {
            let clip = LayerClip::of(level.clip.as_ref().expect("a clip view has a clip"))
                .expect("eligibility admits only clips a layer expresses");
            // The clip in the level's content space: the clip rect
            // shifted by the scroll it precedes.
            let bounds = Rect::new(
                clip.rect.x0 + level.scroll.x,
                clip.rect.y0 + level.scroll.y,
                clip.rect.x1 + level.scroll.x,
                clip.rect.y1 + level.scroll.y,
            );
            // `setFrame` rescales `bounds` proportionally, so the bounds
            // — scroll in its origin, scale in its size — goes last.
            view.setFrame(view_rect(acc, bounds));
            view.setBounds(cg_rect(bounds));
            clip_view(view, &clip);
            add_view(view, outer);
            outer = view;
            acc = Affine::IDENTITY;
        }
        if index == last {
            // The leaf is the whole extent at its scrolled position:
            // `acc` already carries the scroll's `-scroll` shift, so the
            // hosted view sits at the leaf's origin, and the ancestor
            // clip views bound what is visible and hittable.
            let extent = Rect::new(0.0, 0.0, node.extent.width, node.extent.height);
            node.leaf.setFrame(view_rect(acc, extent));
            node.leaf.setBounds(cg_rect(extent));
            add_view(&node.leaf, outer);
            node.place();
        }
    }
}

/// The hosted planes of a composition, owned for the main queue.
#[cfg(target_os = "macos")]
fn hosted_planes(planes: &[Plane<'_>]) -> Vec<(LayerId, HostedView, Size)> {
    planes
        .iter()
        .filter_map(|plane| match &plane.content {
            PlaneContent::Hosted { object, extent } => {
                Some((plane.placement.layer, (*object).clone(), *extent))
            }
            PlaneContent::Raster { .. } | PlaneContent::Frame { .. } => None,
        })
        .collect()
}

/// Captured on main. The window and all Objective-C objects remain there.
pub struct Parent {
    scene: MainOwned<LayerScene>,
}

impl Parent {
    /// Captures the view's layer on main.
    ///
    /// # Panics
    /// Off main, or if the handle does not name a live Apple view.
    pub fn capture(handle: Box<dyn wgpu::WindowHandle>) -> Self {
        use wgpu::rwh::{HasWindowHandle as _, RawWindowHandle};
        let mtm = MainThreadMarker::new().expect("WindowTarget::new must run on main");
        #[cfg(target_os = "macos")]
        let (layer, host_view): (Retained<CALayer>, Retained<NSView>) =
            match handle.window_handle().expect("window handle").as_raw() {
                RawWindowHandle::AppKit(view) => {
                    // SAFETY: the window handle guarantees a live view; we are on main.
                    let view: &objc2_app_kit::NSView = unsafe { view.ns_view.cast().as_ref() };
                    view.setWantsLayer(true);
                    (
                        view.layer().expect("layer-backed view"),
                        Retained::from(view),
                    )
                }
                other => panic!("expected an AppKit view, found {other:?}"),
            };
        // The engine-owned stand-in for the host's layer: inside the
        // bottommost subview — a layer-hosting view — pending display
        // layers park in the committed hierarchy `readyForDisplay`
        // requires, while the host view's own layer stays AppKit's.
        #[cfg(target_os = "macos")]
        let (flipped, probes_layer, probes_view) = {
            let flipped = !layer.contentsAreFlipped();
            let layer = anchored();
            layer.setGeometryFlipped(flipped);
            layer.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)));
            layer.setName(Some(&objc2_foundation::NSString::from_str(
                "cherenkov-probes",
            )));
            let view = part_view(mtm, &layer);
            view.setFrame(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)));
            // Bottommost: parked probes draw nothing and take no hits.
            host_view.addSubview_positioned_relativeTo(
                &view,
                objc2_app_kit::NSWindowOrderingMode::Below,
                None,
            );
            (flipped, layer, view)
        };
        #[cfg(not(target_os = "macos"))]
        let layer = match handle.window_handle().expect("window handle").as_raw() {
            RawWindowHandle::UiKit(view) => {
                // SAFETY: the window handle guarantees a live view; we are on main.
                let view: &objc2_ui_kit::UIView = unsafe { view.ui_view.cast().as_ref() };
                view.layer()
            }
            other => panic!("expected a UIKit view, found {other:?}"),
        };
        let _tx = Transaction::begin();
        #[cfg(not(target_os = "macos"))]
        let root = {
            let root = anchored();
            root.setName(Some(&objc2_foundation::NSString::from_str("cherenkov")));
            layer.addSublayer(&root);
            root
        };
        Self {
            scene: MainOwned::new(
                LayerScene {
                    _window: handle,
                    #[cfg(target_os = "macos")]
                    host: probes_layer,
                    #[cfg(target_os = "macos")]
                    probes: probes_view,
                    #[cfg(target_os = "macos")]
                    flipped,
                    #[cfg(target_os = "macos")]
                    host_view,
                    #[cfg(not(target_os = "macos"))]
                    root,
                    parts: Vec::new(),
                    planes: Vec::new(),
                    #[cfg(target_os = "macos")]
                    spare_planes: Vec::new(),
                    #[cfg(target_os = "macos")]
                    ordered_views: Vec::new(),
                    #[cfg(target_os = "macos")]
                    desired_views: Vec::new(),
                    displays: FxHashMap::default(),
                    retired: Vec::new(),
                    motions: FxHashMap::default(),
                    rasters: FxHashMap::default(),
                    retired_rasters: Vec::new(),
                    hosted: FxHashMap::default(),
                },
                mtm,
            ),
        }
    }
}

struct Transaction;

impl Transaction {
    fn begin() -> Self {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        Self
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        CATransaction::commit();
    }
}

/// A display is always fully constructed. Pending state contains only a
/// completion flag, never a partially initialized Objective-C object.
struct DisplayLayer {
    display: Retained<AVSampleBufferDisplayLayer>,
    generation: Option<u64>,
    /// The registration for the layer's readiness signal.
    /// `readyForDisplay` is not key-value observable — the layer posts
    /// `AVSampleBufferDisplayLayerReadyForDisplayDidChangeNotification`
    /// on every change — so the flag the render thread reads is set from
    /// the notification, never latched from a sample.
    observer: Retained<ReadinessObserver>,
}

impl Drop for DisplayLayer {
    fn drop(&mut self) {
        // SAFETY: the registration is this display's own; the scene that
        // owns it releases it on main.
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver_name_object(
                AsRef::<AnyObject>::as_ref(&*self.observer),
                Some(AVSampleBufferDisplayLayerReadyForDisplayDidChangeNotification),
                Some(AsRef::<AnyObject>::as_ref(&*self.display)),
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct PlaneLayers {
    layer: LayerId,
    raster: bool,
    hosted: bool,
    shape: Vec<(LayerId, bool)>,
    top: Retained<CALayer>,
    levels: Vec<LevelLayers>,
}

/// A plane's native nodes on macOS: a layer-hosting container view for
/// content the engine fills, pass-through views for a hosted `NSView`
/// the engine must not touch at layer level.
#[cfg(target_os = "macos")]
enum PlaneNodes {
    /// A frame or raster plane: a layer-hosting full-surface view whose
    /// layer holds the nested level chain and its display.
    Layers {
        view: Retained<NSView>,
        container: Retained<CALayer>,
        levels: Vec<LevelLayers>,
    },
    /// A hosted plane: a flipped full-surface engine view holding a clip
    /// view per clipped level and the leaf, whose only subview is the
    /// host's view.
    Views {
        container: Retained<NSView>,
        clips: Vec<Option<Retained<NSView>>>,
    },
}

#[cfg(target_os = "macos")]
struct PlaneLayers {
    layer: LayerId,
    raster: bool,
    shape: Vec<(LayerId, bool)>,
    nodes: PlaneNodes,
}

struct LevelLayers {
    node: Retained<CALayer>,
    clip: Option<Retained<CALayer>>,
    scroll: Retained<CALayer>,
}

impl LevelLayers {
    fn inner(&self) -> &CALayer {
        &self.scroll
    }
}

/// A part's nodes on macOS: a layer-hosting view under the host view —
/// subview order is the paint order — its layer, and its metal layer.
#[cfg(target_os = "macos")]
struct Part {
    view: Retained<NSView>,
    container: Retained<CALayer>,
    metal: Retained<CAMetalLayer>,
}

/// A part on iOS is the metal layer itself, a `root` sublayer.
#[cfg(not(target_os = "macos"))]
type Part = Retained<CAMetalLayer>;

/// All Core Animation and `AppKit` access, including hierarchy changes and
/// destruction, is confined to this main-thread scene.
struct LayerScene {
    _window: Box<dyn wgpu::WindowHandle>,
    /// The probes layer: the bottommost layer-hosting view's layer,
    /// where a candidate's pending display parks inside the committed
    /// hierarchy `readyForDisplay` requires. The host view's own layer
    /// is `AppKit`'s — the engine never adds sublayers to it.
    #[cfg(target_os = "macos")]
    host: Retained<CALayer>,
    /// The layer-hosting view owning `host`, bottommost subview of
    /// `host_view`.
    #[cfg(target_os = "macos")]
    probes: Retained<NSView>,
    /// Whether the host view's layer tree is y-up — engine layers under
    /// layer-hosting views get `geometryFlipped` to the opposite.
    #[cfg(target_os = "macos")]
    flipped: bool,
    /// The view the engine's parts and plane containers order themselves
    /// inside: subview order is the paint order.
    #[cfg(target_os = "macos")]
    host_view: Retained<NSView>,
    #[cfg(not(target_os = "macos"))]
    root: Retained<CALayer>,
    parts: Vec<Part>,
    planes: Vec<PlaneLayers>,
    #[cfg(target_os = "macos")]
    spare_planes: Vec<PlaneLayers>,
    #[cfg(target_os = "macos")]
    ordered_views: Vec<StackView>,
    #[cfg(target_os = "macos")]
    desired_views: Vec<StackView>,
    displays: FxHashMap<LayerId, DisplayLayer>,
    /// Displays of demoted candidates, logically gone from `displays` at
    /// once but detached from the hierarchy only inside the next `place`'s
    /// transaction, so the removal never commits a frame on its own.
    retired: Vec<DisplayLayer>,
    motions: FxHashMap<LayerId, super::animation::Motion>,
    rasters: FxHashMap<LayerId, Retained<CALayer>>,
    retired_rasters: Vec<Retained<CALayer>>,
    /// The hosted objects the last composition placed, by tree layer.
    hosted: FxHashMap<LayerId, HostedNode>,
}

impl Drop for LayerScene {
    fn drop(&mut self) {
        let _tx = Transaction::begin();
        // Pending and retired displays are `host` sublayers of their own.
        for display in self.displays.values().chain(&self.retired) {
            display.display.removeFromSuperlayer();
        }
        // The host's objects leave the engine's tree with it.
        for node in self.hosted.values() {
            node.release();
        }
        self.detach();
    }
}

#[cfg(target_os = "macos")]
fn append_view_stack<T>(
    desired: &mut Vec<T>,
    parts: usize,
    planes: usize,
    mut part: impl FnMut(usize) -> T,
    mut plane: impl FnMut(usize) -> T,
) {
    for i in 0..parts {
        desired.push(part(i));
        if i < planes {
            desired.push(plane(i));
        }
    }
    for i in parts..planes {
        desired.push(plane(i));
    }
}

/// Reorders the views `previous` last left in place into `desired`,
/// placing each moved view directly above its predecessor in `desired`
/// (`None` for the first). A steady order places nothing.
///
/// The views that stay put are a heaviest subsequence of `desired` that
/// keeps its `previous` order, where a `pinned` view outweighs every
/// unpinned view together: re-placing a view takes it out of its
/// superview, which resigns a first responder inside it, so a pinned
/// view — a hosted plane's — moves only when the pinned views themselves
/// change order. Everything else is placed around the views that stay.
#[cfg(target_os = "macos")]
fn reconcile_view_order<T: Clone>(
    previous: &mut Vec<T>,
    desired: &[T],
    same: impl Fn(&T, &T) -> bool,
    pinned: impl Fn(&T) -> bool,
    mut place: impl FnMut(&T, Option<&T>),
) {
    if previous.len() == desired.len() && previous.iter().zip(desired).all(|(p, d)| same(p, d)) {
        return;
    }
    let placed: Vec<Option<usize>> = desired
        .iter()
        .map(|view| previous.iter().position(|old| same(old, view)))
        .collect();
    let weight = |i: usize| {
        if pinned(&desired[i]) {
            desired.len() + 1
        } else {
            1
        }
    };
    // `chain[i]`: the weight of the heaviest order-keeping run of placed
    // views ending at `i`, and the view before `i` in it.
    let mut chain: Vec<(usize, Option<usize>)> = vec![(0, None); desired.len()];
    for i in 0..desired.len() {
        let Some(at) = placed[i] else {
            continue;
        };
        chain[i] = (weight(i), None);
        for j in 0..i {
            if placed[j].is_some_and(|before| before < at) && chain[j].0 + weight(i) > chain[i].0 {
                chain[i] = (chain[j].0 + weight(i), Some(j));
            }
        }
    }
    let mut stays = vec![false; desired.len()];
    let mut next = (0..desired.len())
        .max_by_key(|&i| chain[i].0)
        .filter(|&i| chain[i].0 > 0);
    while let Some(i) = next {
        stays[i] = true;
        next = chain[i].1;
    }
    for (index, view) in desired.iter().enumerate() {
        if !stays[index] {
            place(view, index.checked_sub(1).map(|above| &desired[above]));
        }
    }
    previous.clear();
    previous.extend_from_slice(desired);
}

/// A top-level engine view under the host view: a part's, or a plane's
/// outermost view — `hosted` for a hosted plane's, whose subtree may hold
/// the window's first responder.
#[cfg(target_os = "macos")]
#[derive(Clone)]
struct StackView {
    view: Retained<NSView>,
    hosted: bool,
}

#[cfg(target_os = "macos")]
fn reconcile_views(
    host: &NSView,
    probes: &NSView,
    previous: &mut Vec<StackView>,
    desired: &[StackView],
) {
    reconcile_view_order(
        previous,
        desired,
        |previous, desired| {
            std::ptr::eq(
                Retained::as_ptr(&previous.view),
                Retained::as_ptr(&desired.view),
            )
        },
        |view| view.hosted,
        |view, above| {
            host.addSubview_positioned_relativeTo(
                &view.view,
                objc2_app_kit::NSWindowOrderingMode::Above,
                Some(above.map_or(probes, |above| &*above.view)),
            );
        },
    );
}

#[cfg(target_os = "macos")]
fn set_geometry_flipped(
    host: &CALayer,
    parts: &[Part],
    planes: &[PlaneLayers],
    current: &mut bool,
    flipped: bool,
) {
    if *current == flipped {
        return;
    }
    *current = flipped;
    host.setGeometryFlipped(flipped);
    for part in parts {
        part.container.setGeometryFlipped(flipped);
    }
    for plane in planes {
        if let PlaneNodes::Layers { container, .. } = &plane.nodes {
            container.setGeometryFlipped(flipped);
        }
    }
}

impl LayerScene {
    /// Takes the engine's top-level nodes out of the host: the whole
    /// scene's visual state in one call.
    fn detach(&self) {
        #[cfg(target_os = "macos")]
        {
            for part in &self.parts {
                part.view.removeFromSuperview();
            }
            for plane in &self.planes {
                match &plane.nodes {
                    PlaneNodes::Layers { view, .. } => view.removeFromSuperview(),
                    PlaneNodes::Views { container, .. } => container.removeFromSuperview(),
                }
            }
            self.probes.removeFromSuperview();
        }
        #[cfg(not(target_os = "macos"))]
        self.root.removeFromSuperlayer();
    }
}

/// One display's readiness notification target. `NSNotificationCenter`'s
/// selector registration needs an Objective-C object; the notification is
/// posted on whatever thread the render server reports on, so the handler
/// bounces the re-evaluation of `readyForDisplay` back through the
/// scene's main queue.
struct ReadinessIvars {
    /// Weak: the scene retains the observer through `DisplayLayer`; a
    /// strong handle here would keep the whole scene alive.
    scene: Weak<MainThreadBound<RefCell<LayerScene>>>,
    layer: LayerId,
    ready: Weak<AtomicBool>,
    waker: cherenkov::CompletionWaker,
}

objc2::define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class
    // implements no Drop — its ivars release on dealloc.
    #[unsafe(super(NSObject))]
    #[name = "CherenkovReadinessObserver"]
    #[ivars = ReadinessIvars]
    struct ReadinessObserver;

    impl ReadinessObserver {
        #[unsafe(method(readyForDisplayDidChange:))]
        fn ready_for_display_did_change(&self, _note: &NSNotification) {
            let ivars = self.ivars();
            let Some(scene) = ivars.scene.upgrade() else {
                return;
            };
            let layer = ivars.layer;
            let ready = ivars.ready.clone();
            let waker = ivars.waker.clone();
            DispatchQueue::main().exec_async(move || {
                let mtm = MainThreadMarker::new().expect("the main dispatch queue");
                let scene = scene.get(mtm).borrow();
                // A demotion that already landed makes this a dead signal.
                let Some(display) = scene.displays.get(&layer) else {
                    return;
                };
                // SAFETY: the layer's properties are read on main.
                let is_ready = unsafe { display.display.isReadyForDisplay() };
                drop(scene);
                if let Some(flag) = ready.upgrade() {
                    flag.store(is_ready, Ordering::Release);
                }
                waker.wake();
            });
        }
    }

    // SAFETY: `NSObjectProtocol` has no requirements beyond being an
    // `NSObject` subclass, which `ReadinessObserver` is.
    unsafe impl NSObjectProtocol for ReadinessObserver {}
);

impl ReadinessObserver {
    fn new(
        scene: &MainOwned<LayerScene>,
        layer: LayerId,
        ready: Weak<AtomicBool>,
        waker: cherenkov::CompletionWaker,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ReadinessIvars {
            scene: scene.downgrade(),
            layer,
            ready,
            waker,
        });
        // SAFETY: the class is an NSObject subclass; `init` is NSObject's.
        unsafe { objc2::msg_send![super(this), init] }
    }
}

#[derive(Clone)]
struct Configuration {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    size: (u32, u32),
    output: OutputRequest,
}

/// The reply's stamp is the config generation the request was issued
/// under: a resize or re-selection between request and reply leaves the
/// arriving parts configured stale, which `collect_parts` repairs.
type PartReply = mpsc::Receiver<(u64, Result<Vec<WindowSurface>, SurfaceError>)>;

/// Render-side ownership consists exclusively of wgpu handles, messages and
/// completion flags. It never borrows main-thread state or waits for main.
pub struct LayerPlanes {
    scene: MainOwned<LayerScene>,
    config: Configuration,
    /// Bumped by `resize` and `reselect`; part requests carry it.
    config_generation: u64,
    parts: Vec<WindowSurface>,
    replies: Vec<PartReply>,
    requested_parts: usize,
    candidates: FxHashMap<LayerId, Arc<AtomicBool>>,
    offered: FxHashSet<LayerId>,
    woke: bool,
    waker: cherenkov::CompletionWaker,
    owned_animations: Vec<LayerId>,
    static_candidates: FxHashSet<LayerId>,
    buffers: FxHashMap<LayerId, raster::Buffer>,
    placements: Vec<Placement>,
    /// The layers the last composed frame hosted — the plane kind the
    /// native nodes were built with, which reuse compares against.
    hosted: FxHashSet<LayerId>,
    motion: MotionState,
}

/// Tracks installed on the native nodes, and whether those nodes were rebuilt
/// since the last installation. A present that reuses the nodes leaves the
/// flag clear: equal descriptors then need no animation transaction.
#[derive(Default)]
struct MotionState {
    tracks: Vec<super::animation::Motion>,
    tree_changed: bool,
}

impl MotionState {
    fn owns(&self, layer: LayerId) -> bool {
        self.tracks.iter().any(|track| track.layer == layer)
    }

    const fn placed(&mut self, tree_changed: bool) {
        self.tree_changed |= tree_changed;
    }

    fn update(&mut self, tracks: &[super::animation::Motion]) -> bool {
        if tracks == self.tracks && !self.tree_changed {
            return false;
        }
        self.tree_changed = false;
        self.tracks.clear();
        self.tracks.extend_from_slice(tracks);
        true
    }
}

/// An origin-anchored layer: its position is its superlayer point for its
/// bounds origin, so a nested chain composes plain affine maps.
fn anchored() -> Retained<CALayer> {
    let layer = CALayer::new();
    layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
    layer.setPosition(CGPoint::new(0.0, 0.0));
    layer.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)));
    layer
}

const fn cg_affine(t: Affine) -> CGAffineTransform {
    let [xx, yx, xy, yy, x0, y0] = t.as_coeffs();
    CGAffineTransform {
        a: xx,
        b: yx,
        c: xy,
        d: yy,
        tx: x0,
        ty: y0,
    }
}

const fn cg_rect(r: Rect) -> CGRect {
    CGRect::new(CGPoint::new(r.x0, r.y0), CGSize::new(r.width(), r.height()))
}

/// A clip a `CALayer` expresses: its rect, one corner radius on the masked
/// corners, and the corner curve.
#[derive(Debug, PartialEq)]
pub struct LayerClip {
    /// The clip rectangle.
    pub rect: Rect,
    /// The radius of every rounded corner.
    pub radius: f64,
    /// The rounded corners.
    pub corners: CACornerMask,
    /// Whether the corners are continuous rather than circular.
    pub continuous: bool,
}

impl LayerClip {
    /// The layer clip equal to `clip`, when one exists: a rectangle, or a
    /// rectangle whose corners are each square or share one radius no larger
    /// than half its shorter side, with circular corners or continuous
    /// corners at the system smoothing
    /// ([`ContinuousRect::DEFAULT_SMOOTHING`]).
    #[must_use]
    #[expect(
        clippy::float_cmp,
        reason = "a layer carries one radius, so corners share it exactly or are exactly square"
    )]
    pub fn of(clip: &ShapeData) -> Option<Self> {
        let rounded = |rect: Rect, radii: [f64; 4], continuous: bool| {
            let radius = radii.into_iter().fold(0.0_f64, f64::max);
            if radii.iter().any(|&r| r != 0.0 && r != radius)
                || radius > rect.width().min(rect.height()) / 2.0
            {
                return None;
            }
            // In the engine's y-down space the minimum y edge is the top.
            let corners = [
                CACornerMask::LayerMinXMinYCorner,
                CACornerMask::LayerMaxXMinYCorner,
                CACornerMask::LayerMaxXMaxYCorner,
                CACornerMask::LayerMinXMaxYCorner,
            ]
            .into_iter()
            .zip(radii)
            .filter(|(_, r)| *r != 0.0)
            .fold(CACornerMask::empty(), |mask, (corner, _)| mask | corner);
            Some(Self {
                rect,
                radius,
                corners,
                continuous,
            })
        };
        match clip {
            ShapeData::Rect(rect) => Some(Self {
                rect: *rect,
                radius: 0.0,
                corners: CACornerMask::empty(),
                continuous: false,
            }),
            ShapeData::RoundedRect(r) => {
                let radii = r.radii();
                rounded(
                    r.rect(),
                    [
                        radii.top_left,
                        radii.top_right,
                        radii.bottom_right,
                        radii.bottom_left,
                    ],
                    false,
                )
            }
            ShapeData::Continuous(ContinuousRect {
                rect,
                radii,
                smoothing,
            }) => {
                let continuous = if *smoothing == 0.0 {
                    false
                } else if *smoothing == ContinuousRect::DEFAULT_SMOOTHING {
                    true
                } else {
                    return None;
                };
                rounded(
                    *rect,
                    [
                        radii.top_left,
                        radii.top_right,
                        radii.bottom_right,
                        radii.bottom_left,
                    ],
                    continuous,
                )
            }
            ShapeData::Circle(c) => {
                let rect = Rect::from_center_size(c.center, (2.0 * c.radius, 2.0 * c.radius));
                rounded(rect, [c.radius; 4], false)
            }
            ShapeData::Ellipse(e) => {
                // A round ellipse is a circle whatever its rotation; its radii
                // come out of a decomposition, equal up to rounding.
                let radii = e.radii();
                ((radii.x - radii.y).abs() <= 1e-9 * radii.x.max(radii.y)).then_some(())?;
                let radius = f64::midpoint(radii.x, radii.y);
                let rect = Rect::from_center_size(e.center(), (2.0 * radius, 2.0 * radius));
                rounded(rect, [radius; 4], false)
            }
            ShapeData::Line(_) | ShapeData::Path { .. } => None,
        }
    }

    fn apply(&self, layer: &CALayer) {
        layer.setBounds(cg_rect(self.rect));
        layer.setPosition(CGPoint::new(self.rect.x0, self.rect.y0));
        layer.setMasksToBounds(true);
        layer.setCornerRadius(self.radius);
        layer.setMaskedCorners(self.corners);
        // SAFETY: the corner-curve constants are immutable statics.
        let curve = unsafe {
            if self.continuous {
                kCACornerCurveContinuous
            } else {
                kCACornerCurveCircular
            }
        };
        layer.setCornerCurve(curve);
    }
}

/// An external frame's `IOSurface` and pixel format, when its planes are
/// the planes of one `IOSurface` in the layout the frame declares.
fn frame_surface(frame: &ExternalFrame) -> Option<Retained<IOSurfaceRef>> {
    /// The `MTLTexture` behind a plane and the `IOSurface` plane it maps.
    fn plane(texture: &wgpu::Texture) -> Option<(Retained<IOSurfaceRef>, usize)> {
        // SAFETY: the texture lives on a Metal device (planes are imported
        // with `interop::metal::import_texture`); the guard is dropped before
        // the texture.
        let hal = unsafe { texture.as_hal::<wgpu::hal::metal::Api>() }?;
        let raw = hal.raw_handle();
        Some((raw.iosurface()?, raw.iosurfacePlane()))
    }
    let (surface, format) = match &frame.planes {
        FramePlanes::Yuv { y, uv } => {
            let (surface, 0) = plane(y)? else {
                return None;
            };
            let (chroma, 1) = plane(uv)? else {
                return None;
            };
            if Retained::as_ptr(&surface) != Retained::as_ptr(&chroma) {
                return None;
            }
            let ten = y.format() == wgpu::TextureFormat::R16Uint;
            let format = match (ten, frame.color.range) {
                (false, YuvRange::Video) => kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                (false, YuvRange::Full) => kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                (true, YuvRange::Video) => kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange,
                (true, YuvRange::Full) => kCVPixelFormatType_420YpCbCr10BiPlanarFullRange,
            };
            (surface, format)
        }
        FramePlanes::Rgb {
            plane: texture,
            alpha,
        } => {
            // A display layer shows opaque video; translucent video stays
            // in the engine, which composites its alpha. A premultiplied
            // f16 frame is the engine's own frame ring: the layer
            // composites its alpha as it does any contents'.
            let format = match (texture.format(), alpha) {
                (wgpu::TextureFormat::Bgra8Unorm, RgbAlpha::Opaque) => kCVPixelFormatType_32BGRA,
                (wgpu::TextureFormat::Rgba16Float, RgbAlpha::Opaque | RgbAlpha::Premultiplied) => {
                    kCVPixelFormatType_64RGBAHalf
                }
                _ => return None,
            };
            let (surface, 0) = plane(texture)? else {
                return None;
            };
            (surface, format)
        }
    };
    (surface.pixel_format() == format).then_some(surface)
}

/// The `CVImageBuffer` colour attachments equal to a frame's declared
/// colour, as `(key, value)` pairs.
fn color_attachments(frame: &ExternalFrame) -> Vec<(&'static CFString, &'static CFString)> {
    let color = &frame.color;
    // SAFETY: CoreVideo's attachment keys and values are immutable statics.
    unsafe {
        let primaries = match color.primaries {
            Primaries::Bt709 => kCVImageBufferColorPrimaries_ITU_R_709_2,
            Primaries::DisplayP3 => kCVImageBufferColorPrimaries_P3_D65,
            Primaries::Bt2020 => kCVImageBufferColorPrimaries_ITU_R_2020,
        };
        let transfer = match color.transfer {
            Transfer::Linear => kCVImageBufferTransferFunction_Linear,
            Transfer::Srgb => kCVImageBufferTransferFunction_sRGB,
            Transfer::Bt709 => kCVImageBufferTransferFunction_ITU_R_709_2,
            Transfer::Pq => kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ,
            Transfer::Hlg => kCVImageBufferTransferFunction_ITU_R_2100_HLG,
        };
        let mut pairs = vec![
            (kCVImageBufferColorPrimariesKey, primaries),
            (kCVImageBufferTransferFunctionKey, transfer),
        ];
        if matches!(frame.planes, FramePlanes::Yuv { .. }) {
            let matrix = match color.matrix {
                YuvMatrix::Bt601 => kCVImageBufferYCbCrMatrix_ITU_R_601_4,
                YuvMatrix::Bt709 => kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                YuvMatrix::Bt2020 => kCVImageBufferYCbCrMatrix_ITU_R_2020,
            };
            let siting = match (color.chroma_siting.x, color.chroma_siting.y) {
                (ChromaOffset::Cosited, ChromaOffset::Centered) => {
                    kCVImageBufferChromaLocation_Left
                }
                (ChromaOffset::Centered, ChromaOffset::Centered) => {
                    kCVImageBufferChromaLocation_Center
                }
                (ChromaOffset::Cosited, ChromaOffset::Cosited) => {
                    kCVImageBufferChromaLocation_TopLeft
                }
                (ChromaOffset::Centered, ChromaOffset::Cosited) => kCVImageBufferChromaLocation_Top,
            };
            pairs.extend([
                (kCVImageBufferYCbCrMatrixKey, matrix),
                (kCVImageBufferChromaLocationTopFieldKey, siting),
                (kCVImageBufferChromaLocationBottomFieldKey, siting),
            ]);
        }
        pairs
    }
}

/// A pixel buffer over the frame's `IOSurface`, carrying the frame's
/// colour as attachments. No pixel is copied.
///
/// # Errors
/// When the frame's planes are not an `IOSurface` ([`LayerPlanes::shows`]
/// admits only frames whose planes are) or `CoreVideo` refuses the wrap.
fn pixel_buffer(frame: &ExternalFrame) -> Result<CFRetained<CVPixelBuffer>, RenderError> {
    let surface = frame_surface(frame).ok_or_else(|| {
        RenderError::Render("a promoted external frame's planes are not one IOSurface".into())
    })?;
    let mut out = std::ptr::null_mut();
    // SAFETY: `surface` is a live IOSurface retained by `frame_surface`, and
    // `out` is a valid out-pointer that receives a +1 pixel buffer on
    // success.
    let status =
        unsafe { CVPixelBufferCreateWithIOSurface(None, &surface, None, NonNull::from(&mut out)) };
    let buffer = NonNull::new(out)
        .filter(|_| status == kCVReturnSuccess)
        .ok_or_else(|| {
            RenderError::Render(format!(
                "CoreVideo refused to wrap a promoted frame's IOSurface (CVReturn {status})"
            ))
        })?;
    // SAFETY: the create call returned a +1 pixel buffer.
    let buffer = unsafe { CFRetained::from_raw(buffer) };
    for (key, value) in color_attachments(frame) {
        // SAFETY: every colour attachment's value is a CFString.
        unsafe { buffer.set_attachment(key, value, CVAttachmentMode::ShouldPropagate) };
    }
    Ok(buffer)
}

/// A sample buffer the display layer shows as soon as it is enqueued.
fn sample_buffer(buffer: &CVPixelBuffer) -> Result<CFRetained<CMSampleBuffer>, RenderError> {
    let mut format = std::ptr::null();
    // SAFETY: `buffer` is a live CVPixelBuffer and `format` a valid
    // out-pointer that receives a +1 format description on success.
    let status = unsafe {
        CMVideoFormatDescriptionCreateForImageBuffer(None, buffer, NonNull::from(&mut format))
    };
    let format = NonNull::new(format.cast_mut())
        .filter(|_| status == 0)
        .ok_or_else(|| {
            RenderError::Render(format!(
                "CoreMedia refused a promoted frame's format description (OSStatus {status})"
            ))
        })?;
    // SAFETY: the create call returned a +1 format description.
    let format: CFRetained<CMVideoFormatDescription> = unsafe { CFRetained::from_raw(format) };
    // SAFETY: `kCMTimeInvalid` is an immutable static.
    let invalid = unsafe { kCMTimeInvalid };
    let mut timing = CMSampleTimingInfo {
        duration: invalid,
        presentationTimeStamp: invalid,
        decodeTimeStamp: invalid,
    };
    let mut out = std::ptr::null_mut();
    // SAFETY: `buffer` is a live image buffer, `format` the +1 description
    // created for it above, and `timing`/`out` are valid pointers — `timing`
    // supplies one timing record for the single sample.
    let status = unsafe {
        CMSampleBuffer::create_ready_with_image_buffer(
            None,
            buffer,
            &format,
            NonNull::from(&mut timing),
            NonNull::from(&mut out),
        )
    };
    let sample = NonNull::new(out).filter(|_| status == 0).ok_or_else(|| {
        RenderError::Render(format!(
            "CoreMedia refused a promoted frame's sample buffer (OSStatus {status})"
        ))
    })?;
    // SAFETY: the create call returned a +1 sample buffer.
    let sample = unsafe { CFRetained::from_raw(sample) };
    // SAFETY: `sample` is a live CMSampleBuffer; `true` has CoreMedia create
    // the attachments array, and the result is retained.
    let attachments = unsafe { sample.sample_attachments_array(true) }.ok_or_else(|| {
        RenderError::Render("a promoted frame's sample buffer has no attachments".into())
    })?;
    // SAFETY: a one-sample buffer's attachment array holds one mutable
    // dictionary; the key and value are CoreMedia and CoreFoundation statics.
    unsafe {
        let dictionary: &CFMutableDictionary =
            &*attachments.value_at_index(0).cast::<CFMutableDictionary>();
        CFMutableDictionary::set_value(
            Some(dictionary),
            std::ptr::from_ref(kCMSampleAttachmentKey_DisplayImmediately).cast(),
            std::ptr::from_ref::<CFBoolean>(
                objc2_core_foundation::kCFBooleanTrue.expect("kCFBooleanTrue"),
            )
            .cast(),
        );
    }
    Ok(sample)
}

/// The nested-layer shape a placement needs.
fn shape(placement: &Placement) -> Vec<(LayerId, bool)> {
    placement
        .path
        .iter()
        .map(|level| (level.layer, level.clip.is_some()))
        .collect()
}

/// Whether the native nodes built for `built` serve `next`: layer
/// identity, the path shape [`shape`] records, the raster class and the
/// plane kind — a layer that switched between a frame and a hosted
/// object rebuilds, since the node kind differs. `raster`/`hosted` are
/// the built and next flags — `built.source == Source::Recorded` on the
/// render side, `self.rasters` on main; `matches!(nodes, Views)` on main,
/// the last frame's hosted set on the render side.
fn reuses_native_nodes(
    built: (LayerId, &[(LayerId, bool)], bool, bool),
    next: &Placement,
    next_raster: bool,
    next_hosted: bool,
) -> bool {
    let (layer, built_shape, raster, hosted) = built;
    layer == next.layer
        && built_shape.iter().copied().eq(next
            .path
            .iter()
            .map(|level| (level.layer, level.clip.is_some())))
        && raster == next_raster
        && hosted == next_hosted
}

/// The [`reuses_native_nodes`] arguments for a render-side `built`
/// placement: its raster class is the recorded source, its kind the last
/// composed frame's hosted set.
fn reuses_native_placement(
    built: &Placement,
    next: &Placement,
    rasters: &FxHashSet<LayerId>,
    built_hosted: &FxHashSet<LayerId>,
    next_hosted: &FxHashSet<LayerId>,
) -> bool {
    built.layer == next.layer
        && built
            .path
            .iter()
            .map(|level| (level.layer, level.clip.is_some()))
            .eq(next
                .path
                .iter()
                .map(|level| (level.layer, level.clip.is_some())))
        && (built.source == Source::Recorded) == rasters.contains(&next.layer)
        && built_hosted.contains(&built.layer) == next_hosted.contains(&next.layer)
}

/// Applies a level's sampled properties to its layers.
fn place(level: &Level, layers: &LevelLayers, owns_position: bool) {
    if !owns_position {
        layers.node.setPosition(CGPoint::new(0.0, 0.0));
        layers.node.setAffineTransform(cg_affine(level.transform));
    }
    if let (Some(clip), Some(layer)) = (&level.clip, &layers.clip) {
        LayerClip::of(clip)
            .expect("eligibility admits only clips a layer expresses")
            .apply(layer);
    }
    let Vec2 { x, y } = level.scroll;
    layers
        .scroll
        .setBounds(CGRect::new(CGPoint::new(x, y), CGSize::new(0.0, 0.0)));
}

impl LayerScene {
    fn add_parts(
        &mut self,
        config: &Configuration,
        count: usize,
        mut probe: Option<mpsc::Sender<crate::render::present::DisplayProbe>>,
        #[cfg_attr(not(target_os = "macos"), expect(unused_variables))] mtm: MainThreadMarker,
    ) -> Result<Vec<WindowSurface>, SurfaceError> {
        let mut parts = Vec::new();
        let _tx = Transaction::begin();
        while self.parts.len() < count {
            let layer = CAMetalLayer::new();
            layer.setPresentsWithTransaction(true);
            layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
            let surface = WindowSurface::from_layer(
                &config.instance,
                &config.adapter,
                &config.device,
                &layer,
                config.size,
                // Parts above the first composite over it.
                OutputRequest {
                    transparent: config.output.transparent || !self.parts.is_empty(),
                    ..config.output
                },
                probe.take(),
            )?;
            #[cfg(target_os = "macos")]
            self.parts.push({
                let container = anchored();
                container.setGeometryFlipped(self.flipped);
                container.addSublayer(&layer);
                Part {
                    view: part_view(mtm, &container),
                    container,
                    metal: layer,
                }
            });
            #[cfg(not(target_os = "macos"))]
            self.parts.push(layer);
            parts.push(surface);
        }
        Ok(parts)
    }

    /// Parks `display` beside `root` under the host: inside the committed
    /// layer hierarchy at zero bounds — invisible, which is the state the
    /// platform requires before it reports `readyForDisplay` — and marked
    /// with the pending name the hierarchy's consumers can tell a probe
    /// from a promoted plane by.
    fn park(&self, display: &AVSampleBufferDisplayLayer) {
        display.setName(Some(&objc2_foundation::NSString::from_str(
            "cherenkov-pending",
        )));
        display.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)));
        display.setPosition(CGPoint::new(0.0, 0.0));
        #[cfg(target_os = "macos")]
        let outer = &*self.host;
        #[cfg(not(target_os = "macos"))]
        let outer = &*self.root;
        if display.superlayer().as_deref() != Some(outer) {
            outer.addSublayer(display);
        }
    }

    /// Creates `layer`'s display layer, parks it beside `root`, and
    /// registers for the readiness notification that re-evaluates the flag
    /// `ready` on every change, waking the render loop each time.
    fn attach(
        &mut self,
        layer: LayerId,
        scene: &MainOwned<Self>,
        ready: Weak<AtomicBool>,
        waker: cherenkov::CompletionWaker,
    ) {
        // SAFETY: creation and every subsequent use are on main.
        let display = unsafe { AVSampleBufferDisplayLayer::new() };
        display.setAnchorPoint(CGPoint::new(0.0, 0.0));
        // SAFETY: `display` is a live layer touched only on main, and the
        // gravity argument is the framework's `AVLayerVideoGravityResize`
        // constant.
        unsafe {
            display.setVideoGravity(AVLayerVideoGravityResize.expect("video gravity"));
            display.setPreventsDisplaySleepDuringVideoPlayback(false);
        }
        self.park(&display);
        let observer = ReadinessObserver::new(scene, layer, ready, waker);
        // SAFETY: the immutable name selects this layer's own readiness
        // notification; the selector is the observer's and the object
        // filter is the layer it watches.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                AsRef::<AnyObject>::as_ref(&*observer),
                objc2::sel!(readyForDisplayDidChange:),
                Some(AVSampleBufferDisplayLayerReadyForDisplayDidChangeNotification),
                Some(AsRef::<AnyObject>::as_ref(&*display)),
            );
        }
        assert!(
            self.displays
                .insert(
                    layer,
                    DisplayLayer {
                        display,
                        generation: None,
                        observer,
                    }
                )
                .is_none(),
            "each candidate has one display"
        );
    }

    /// The level chain of a frame or raster plane under `outer`:
    /// node → clip → scroll per level, the display at the leaf.
    fn level_layers(&self, placement: &Placement, outer: &CALayer) -> Vec<LevelLayers> {
        let mut outer: Retained<CALayer> = Retained::from(outer);
        let mut levels = Vec::with_capacity(placement.path.len());
        for level in &placement.path {
            let node = anchored();
            outer.addSublayer(&node);
            let clip = level.clip.as_ref().map(|_| {
                let clip = anchored();
                node.addSublayer(&clip);
                clip
            });
            let scroll = anchored();
            clip.as_deref().unwrap_or(&node).addSublayer(&scroll);
            outer = scroll.clone();
            levels.push(LevelLayers { node, clip, scroll });
        }
        let display = self.display(placement.layer);
        levels
            .last()
            .map_or(&*outer, LevelLayers::inner)
            .addSublayer(display);
        levels
    }

    #[cfg(not(target_os = "macos"))]
    fn plane_layers(&self, placement: &Placement, scale: f64) -> PlaneLayers {
        let top = anchored();
        top.setAffineTransform(cg_affine(Affine::scale(1.0 / scale)));
        let levels = self.level_layers(placement, &top);
        PlaneLayers {
            layer: placement.layer,
            raster: self.rasters.contains_key(&placement.layer),
            hosted: self.hosted.contains_key(&placement.layer),
            shape: shape(placement),
            top,
            levels,
        }
    }

    /// macOS: a frame or raster plane is a flipped full-surface container
    /// layer holding the level chain; a hosted plane is a flipped
    /// full-surface engine view holding a clip view per clipped level —
    /// unclipped levels fold into the next view's transform — and the
    /// hosted node's leaf view, added at `place` time.
    #[cfg(target_os = "macos")]
    fn plane_layers(&self, placement: &Placement, mtm: MainThreadMarker) -> PlaneLayers {
        let nodes = if self.hosted.contains_key(&placement.layer) {
            PlaneNodes::Views {
                container: engine_view(mtm),
                clips: placement
                    .path
                    .iter()
                    .map(|level| level.clip.as_ref().map(|_| engine_view(mtm)))
                    .collect(),
            }
        } else {
            let container = anchored();
            container.setGeometryFlipped(self.flipped);
            PlaneNodes::Layers {
                levels: self.level_layers(placement, &container),
                view: part_view(mtm, &container),
                container,
            }
        };
        PlaneLayers {
            layer: placement.layer,
            raster: self.rasters.contains_key(&placement.layer),
            shape: shape(placement),
            nodes,
        }
    }

    /// The plane's outermost view, ordered under `host_view` by subview
    /// order.
    #[cfg(target_os = "macos")]
    fn top_view(plane: &PlaneLayers) -> StackView {
        match &plane.nodes {
            PlaneNodes::Layers { view, .. } => StackView {
                view: view.clone(),
                hosted: false,
            },
            PlaneNodes::Views { container, .. } => StackView {
                view: container.clone(),
                hosted: true,
            },
        }
    }

    /// macOS `place`: parts and plane tops are direct subviews of the
    /// host view, ordered by `addSubview:positioned:relativeTo:` —
    /// subview order is the paint order — each plane carrying its own
    /// flipped contents in its node chain.
    #[cfg(target_os = "macos")]
    fn place(
        &mut self,
        placements: &[Placement],
        size: (u32, u32),
        scale: f64,
        parts: usize,
        mtm: MainThreadMarker,
    ) {
        // The host's layer can change orientation when the view joins a
        // window or moves between displays — read it every present.
        let flipped = !self
            .host_view
            .layer()
            .expect("a layer-backed host view")
            .contentsAreFlipped();
        set_geometry_flipped(
            &self.host,
            &self.parts,
            &self.planes,
            &mut self.flipped,
            flipped,
        );
        let points = CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(f64::from(size.0) / scale, f64::from(size.1) / scale),
        );
        let pixels = CGSize::new(f64::from(size.0), f64::from(size.1));
        // A demoted candidate's display detaches here, inside this
        // transaction, so its removal never commits ahead of the rebuild
        // that shows the layers below.
        for retired in self.retired.drain(..) {
            retired.display.removeFromSuperlayer();
        }
        for retired in self.retired_rasters.drain(..) {
            retired.removeFromSuperlayer();
        }
        // Identity, not position, determines reuse. Moving B from slot 1 to
        // slot 0 never consumes or replaces A's display.
        let mut old = std::mem::take(&mut self.spare_planes);
        std::mem::swap(&mut old, &mut self.planes);
        for placement in placements {
            let reusable = old.iter().position(|p| {
                reuses_native_nodes(
                    (
                        p.layer,
                        &p.shape,
                        p.raster,
                        matches!(p.nodes, PlaneNodes::Views { .. }),
                    ),
                    placement,
                    self.rasters.contains_key(&placement.layer),
                    self.hosted.contains_key(&placement.layer),
                )
            });
            let built = if let Some(index) = reusable {
                old.remove(index)
            } else {
                // A new native path has no animations, even if its engine
                // track is unchanged. Reinstall the original timed track.
                self.motions.remove(&placement.layer);
                self.plane_layers(placement, mtm)
            };
            self.place_plane(placement, &built, points, pixels);
            self.planes.push(built);
        }
        while let Some(plane) = old.pop() {
            match &plane.nodes {
                PlaneNodes::Layers { view, .. } => view.removeFromSuperview(),
                PlaneNodes::Views { container, .. } => container.removeFromSuperview(),
            }
        }
        self.spare_planes = old;
        for (&layer, raster) in &self.rasters {
            if !placements.iter().any(|placement| placement.layer == layer) {
                // SAFETY: main-thread CALayer; removing the last native
                // reference releases a demoted capture's IOSurface.
                unsafe { raster.setContents(None) };
            }
        }
        // A candidate outside the plan — still probing, or demoted when
        // its readiness was lost — parks its display beside the stack so
        // the probe keeps the hierarchy state the signal requires.
        for (id, display) in &self.displays {
            if !self.planes.iter().any(|plane| plane.layer == *id) {
                self.park(&display.display);
            }
        }
        self.order(points, scale, parts);
    }

    /// Places one plane's nodes: `points` and `pixels` are the surface's
    /// extent in each unit.
    #[cfg(target_os = "macos")]
    fn place_plane(
        &self,
        placement: &Placement,
        built: &PlaneLayers,
        points: CGRect,
        pixels: CGSize,
    ) {
        match &built.nodes {
            PlaneNodes::Layers {
                view,
                container,
                levels,
            } => {
                // The view's frame and bounds are the layer's: AppKit
                // derives the layer's transform from them — `1/scale`,
                // the points-to-pixels conversion — so the scale arrives
                // through `bounds` alone, and `transform` stays
                // AppKit-owned.
                view.setFrame(points);
                view.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), pixels));
                container.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), pixels));
                for (level, layers) in placement.path.iter().zip(levels) {
                    place(
                        level,
                        layers,
                        self.motions
                            .get(&level.layer)
                            .is_some_and(|motion| motion.position.is_some()),
                    );
                }
                let display = self.display(placement.layer);
                display.setBounds(CGRect::new(
                    CGPoint::new(0.0, 0.0),
                    CGSize::new(f64::from(placement.size.0), f64::from(placement.size.1)),
                ));
                display.setAffineTransform(cg_affine(placement.raster));
                if self
                    .motions
                    .get(&placement.layer)
                    .is_none_or(|motion| motion.opacity.is_none())
                {
                    display.setOpacity(placement.opacity);
                }
                display.setName(None);
            }
            PlaneNodes::Views { container, clips } => {
                container.setFrame(points);
                container.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), pixels));
                let node = &self.hosted[&placement.layer];
                place_views(placement, container, clips, node);
                node.leaf.setAlphaValue(f64::from(placement.opacity));
            }
        }
    }

    /// Paint order is subview order: each top-level element — a part's
    /// or a plane's outermost view — sits in `host_view`'s `subviews` at
    /// its paint index, above the element before it. The probes view,
    /// added first, stays bottommost. A steady frame writes nothing to
    /// the hierarchy, and a reorder moves the part and frame-plane views,
    /// which hold no responder state, around the hosted planes' views, so
    /// a hosted view keeps its first-responder state
    /// ([`reconcile_view_order`]).
    #[cfg(target_os = "macos")]
    fn order(&mut self, points: CGRect, scale: f64, parts: usize) {
        for part in &self.parts[..parts] {
            part.view.setFrame(points);
            part.container.setBounds(points);
            part.metal.setFrame(points);
            part.metal.setContentsScale(scale);
        }
        self.desired_views.clear();
        let (scene_parts, scene_planes, desired) =
            (&self.parts, &self.planes, &mut self.desired_views);
        append_view_stack(
            desired,
            parts,
            scene_planes.len(),
            |i| StackView {
                view: scene_parts[i].view.clone(),
                hosted: false,
            },
            |i| Self::top_view(&scene_planes[i]),
        );
        // A part the plan no longer uses leaves the stack: its view
        // detaches, and the `Part` stays for reuse.
        for part in self.parts.iter().skip(parts) {
            // SAFETY: `superview` is a main-thread accessor.
            if unsafe { part.view.superview() }.is_some() {
                part.view.removeFromSuperview();
            }
        }
        reconcile_views(
            &self.host_view,
            &self.probes,
            &mut self.ordered_views,
            &self.desired_views,
        );
    }

    #[cfg(not(target_os = "macos"))]
    fn place(&mut self, placements: &[Placement], size: (u32, u32), scale: f64, parts: usize) {
        let bounds = CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(f64::from(size.0) / scale, f64::from(size.1) / scale),
        );
        self.root.setBounds(bounds);
        // A demoted candidate's display detaches here, inside this
        // transaction, so its removal never commits ahead of the rebuild
        // that shows the layers below.
        for retired in self.retired.drain(..) {
            retired.display.removeFromSuperlayer();
        }
        for retired in self.retired_rasters.drain(..) {
            retired.removeFromSuperlayer();
        }
        // Identity, not position, determines reuse. Moving B from slot 1 to
        // slot 0 never consumes or replaces A's display.
        let mut old = std::mem::take(&mut self.planes);
        for placement in placements {
            let reusable = old.iter().position(|p| {
                reuses_native_nodes(
                    (p.layer, &p.shape, p.raster, p.hosted),
                    placement,
                    self.rasters.contains_key(&placement.layer),
                    self.hosted.contains_key(&placement.layer),
                )
            });
            let built = if let Some(index) = reusable {
                old.remove(index)
            } else {
                // A new native path has no animations, even if its engine
                // track is unchanged. Reinstall the original timed track.
                self.motions.remove(&placement.layer);
                self.plane_layers(placement, scale)
            };
            built
                .top
                .setAffineTransform(cg_affine(Affine::scale(1.0 / scale)));
            for (level, layers) in placement.path.iter().zip(&built.levels) {
                place(
                    level,
                    layers,
                    self.motions
                        .get(&level.layer)
                        .is_some_and(|motion| motion.position.is_some()),
                );
            }
            let display = self.display(placement.layer);
            let extent = self.hosted.get(&placement.layer).map_or_else(
                || CGSize::new(f64::from(placement.size.0), f64::from(placement.size.1)),
                |node| CGSize::new(node.extent.width, node.extent.height),
            );
            display.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), extent));
            display.setAffineTransform(cg_affine(placement.raster));
            if self
                .motions
                .get(&placement.layer)
                .is_none_or(|motion| motion.opacity.is_none())
            {
                display.setOpacity(placement.opacity);
            }
            display.setName(None);
            self.planes.push(built);
        }
        for plane in old {
            plane.top.removeFromSuperlayer();
        }
        for (&layer, raster) in &self.rasters {
            if !placements.iter().any(|placement| placement.layer == layer) {
                // SAFETY: main-thread CALayer; removing the last native
                // reference releases a demoted capture's IOSurface.
                unsafe { raster.setContents(None) };
            }
        }
        // A candidate outside the plan — still probing, or demoted when
        // its readiness was lost — parks its display beside the root so
        // the probe keeps the hierarchy state the signal requires.
        for (id, display) in &self.displays {
            if !self.planes.iter().any(|plane| plane.layer == *id) {
                self.park(&display.display);
            }
        }
        let mut order: Vec<&CALayer> = Vec::new();
        for i in 0..parts {
            let part = &self.parts[i];
            part.setFrame(bounds);
            part.setContentsScale(scale);
            order.push(part);
            if let Some(plane) = self.planes.get(i) {
                order.push(&plane.top);
            }
        }
        for plane in self.planes.iter().skip(parts) {
            order.push(&plane.top);
        }
        // SAFETY: all elements are layers retained by this main-thread scene.
        unsafe { self.root.setSublayers(Some(&NSArray::from_slice(&order))) };
    }

    #[cfg(not(target_os = "macos"))]
    fn display(&self, layer: LayerId) -> &CALayer {
        if let Some(node) = self.hosted.get(&layer) {
            return &node.holder;
        }
        match self.rasters.get(&layer) {
            Some(raster) => raster,
            None => &self.displays[&layer].display,
        }
    }

    /// The display layer of a frame or raster plane: a hosted plane's
    /// "display" is its leaf view, never a layer, so it is not looked
    /// up here — `place_views` owns the hosted path's geometry.
    #[cfg(target_os = "macos")]
    fn display(&self, layer: LayerId) -> &CALayer {
        match self.rasters.get(&layer) {
            Some(raster) => raster,
            None => &self.displays[&layer].display,
        }
    }

    /// Makes the hosted objects exactly `hosted`, resolving each object
    /// on main ([`host`]).
    #[cfg(not(target_os = "macos"))]
    fn host(&mut self, hosted: Vec<(LayerId, HostedLayer, Size)>, mtm: MainThreadMarker) {
        host(
            &mut self.hosted,
            hosted
                .into_iter()
                .map(|(id, object, extent)| (id, object.layer(mtm), extent))
                .collect(),
        );
    }

    /// Makes the hosted views exactly `hosted`, resolving each object
    /// on main ([`host`]).
    #[cfg(target_os = "macos")]
    fn host(&mut self, hosted: Vec<(LayerId, HostedView, Size)>, mtm: MainThreadMarker) {
        host(
            &mut self.hosted,
            hosted
                .into_iter()
                .map(|(id, object, extent)| (id, object.view(mtm), extent))
                .collect(),
            mtm,
        );
    }

    /// The window's first responder when it is a hosted view or lies
    /// inside one: the responder a commit that re-parents the hosted
    /// view's ancestors would resign.
    #[cfg(target_os = "macos")]
    fn hosted_responder(&self) -> Option<Retained<NSView>> {
        let responder = self.host_view.window()?.firstResponder()?;
        let view = responder.downcast::<NSView>().ok()?;
        self.hosted
            .values()
            .any(|node| view.isDescendantOf(&node.view))
            .then_some(view)
    }

    /// Hands first responder back to `responder`, the hosted responder
    /// before this commit, when the commit's re-parenting resigned it.
    /// `order` never moves a hosted plane's views to reorder the stack,
    /// but a path whose shape changed rebuilds the views around the
    /// leaf, and re-parenting the leaf resigns a responder inside it. A
    /// responder that left the window — its hosted view released — keeps
    /// the resignation `AppKit` made.
    #[cfg(target_os = "macos")]
    fn restore_responder(&self, responder: Option<Retained<NSView>>) {
        let Some(responder) = responder else {
            return;
        };
        let Some(window) = self.host_view.window() else {
            return;
        };
        if responder
            .window()
            .is_none_or(|own| !own.isEqual(Some(&*window)))
            || window
                .firstResponder()
                .is_some_and(|current| current.isEqual(Some(&*responder)))
        {
            return;
        }
        let responder: &objc2_app_kit::NSResponder = &responder;
        if !window.makeFirstResponder(Some(responder)) {
            tracing::warn!(
                target: "cherenkov::planes",
                "a re-parented hosted view refused first responder back"
            );
        }
    }

    /// Hands `frame` to `layer`'s display layer.
    ///
    /// # Panics
    /// The failures here are admission-contract violations, not runtime
    /// conditions: `layer` must name an attached display, `frame` must be
    /// one [`Compositor::shows`] admitted — its planes are one
    /// `IOSurface` a `CVPixelBuffer` and `CMSampleBuffer` always wrap —
    /// and the system's sample renderer must accept that sample. A
    /// broken contract cannot be recovered inside this dispatched block,
    /// and a silently dropped frame is the failure this refuses to hide.
    fn show(&mut self, layer: LayerId, frame: &ExternalFrame, generation: u64) {
        let shown = self
            .displays
            .get_mut(&layer)
            .expect("show is called only for an attached candidate");
        if shown.generation == Some(generation) {
            return;
        }
        let buffer = pixel_buffer(frame).expect("an admitted frame's planes are one IOSurface");
        let sample =
            sample_buffer(&buffer).expect("an IOSurface pixel buffer is a valid video sample");
        // SAFETY: the layer and its renderer are accessed only on main.
        let renderer = unsafe { shown.display.sampleBufferRenderer() };
        // SAFETY: `renderer` is this display layer's own renderer, used only
        // on main, and `sample` is a live CMSampleBuffer it may enqueue.
        unsafe { renderer.enqueueSampleBuffer(&sample) };
        assert_ne!(
            // SAFETY: same renderer, still on main.
            unsafe { renderer.status() },
            AVQueuedSampleBufferRenderingStatus::Failed,
            "system compositor rejected the frame: {:?}",
            // SAFETY: same renderer, still on main; `error` is read only on
            // the failure path this assertion takes.
            unsafe { renderer.error() }
        );
        shown.display.setNeedsLayout();
        shown.display.layoutIfNeeded();
        shown.generation = Some(generation);
    }
}

/// Owned frame messages preserve textures until their producer's completion.
struct Update {
    layer: LayerId,
    frame: ExternalFrame,
    generation: u64,
}

impl Update {
    fn from_plane(plane: &Plane<'_>) -> Option<Self> {
        let PlaneContent::Frame { frame, generation } = &plane.content else {
            return None;
        };
        Some(Self {
            layer: plane.placement.layer,
            frame: (*frame).clone(),
            generation: *generation,
        })
    }
}

impl LayerPlanes {
    /// Opens a main-owned compositor without waiting for the main queue.
    ///
    /// # Errors
    /// Surface negotiation errors are delivered by the completion to compose.
    #[expect(
        clippy::too_many_arguments,
        reason = "the surface negotiation inputs: device triple, parent layer, size, \
                  output request, probe channel and the completion waker"
    )]
    pub fn new(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        parent: Parent,
        size: (u32, u32),
        output: OutputRequest,
        probe: Option<mpsc::Sender<crate::render::present::DisplayProbe>>,
        waker: cherenkov::CompletionWaker,
    ) -> Self {
        let mut result = Self {
            scene: parent.scene,
            config: Configuration {
                instance: instance.clone(),
                adapter: adapter.clone(),
                device: device.clone(),
                size,
                output,
            },
            config_generation: 0,
            parts: Vec::new(),
            replies: Vec::new(),
            requested_parts: 0,
            candidates: FxHashMap::default(),
            offered: FxHashSet::default(),
            woke: false,
            waker,
            owned_animations: Vec::new(),
            static_candidates: FxHashSet::default(),
            buffers: FxHashMap::default(),
            placements: Vec::new(),
            hosted: FxHashSet::default(),
            motion: MotionState::default(),
        };
        result.request_parts(1, probe);
        result
    }

    /// The raster planes' `IOSurface` contents, converting each new
    /// capture into a buffer of its own; `None` while any buffer's
    /// conversion is still running. The immutable `IOSurface` is published
    /// only after the GPU has finished its presentation conversion — until
    /// then the committed scene and all of its old buffers stay visible
    /// together.
    fn raster_contents(
        &mut self,
        c: &mut Composition<'_>,
    ) -> Result<Option<Vec<(LayerId, raster::Contents)>>, RenderError> {
        let mut ready = true;
        let mut contents = Vec::new();
        self.buffers.retain(|layer, _| {
            c.planes.iter().any(|plane| {
                plane.placement.layer == *layer
                    && matches!(plane.content, PlaneContent::Raster { .. })
            })
        });
        for plane in c.planes {
            let PlaneContent::Raster { view, generation } = &plane.content else {
                continue;
            };
            let layer = plane.placement.layer;
            if self.buffers.get(&layer).is_none_or(|buffer| {
                buffer.generation != *generation
                    || buffer.headroom.to_bits() != c.display.headroom.to_bits()
            }) {
                let buffer = raster::Buffer::new(
                    c.device,
                    plane.placement.size,
                    *generation,
                    c.display.headroom,
                )?;
                buffer.completing(c.queue, self.waker.clone());
                c.presenter.texture(
                    c.device,
                    c.queue,
                    view.expect("a new native capture has engine pixels"),
                    crate::interop::TextureOutput {
                        texture: &buffer.texture,
                        color: crate::interop::OutputColor::LinearDisplayP3,
                        alpha: crate::interop::OutputAlpha::Premultiplied,
                        headroom: c.display.headroom,
                    },
                );
                self.buffers.insert(layer, buffer);
            }
            let buffer = &self.buffers[&layer];
            ready &= buffer.ready.load(Ordering::Acquire);
            contents.push((layer, raster::Contents(buffer.surface.clone())));
        }
        Ok(ready.then_some(contents))
    }

    fn request_parts(
        &mut self,
        count: usize,
        probe: Option<mpsc::Sender<crate::render::present::DisplayProbe>>,
    ) {
        if count <= self.requested_parts {
            return;
        }
        let (send, receive) = mpsc::channel();
        self.replies.push(receive);
        self.requested_parts = count;
        let config = self.config.clone();
        let generation = self.config_generation;
        let waker = self.waker.clone();
        self.scene.run(move |scene, mtm| {
            let result = scene.add_parts(&config, count, probe, mtm);
            if send.send((generation, result)).is_err() {
                // Cancellation: the render owner was destroyed while this
                // command was queued. Its scene release is queued behind us.
                scene.detach();
                return;
            }
            waker.wake();
        });
    }

    fn collect_parts(&mut self) -> Result<(), RenderError> {
        let mut consumed = 0;
        for reply in &self.replies {
            match reply.try_recv() {
                Ok((generation, result)) => {
                    let mut parts =
                        result.map_err(|error| RenderError::Render(error.to_string()))?;
                    if generation != self.config_generation {
                        // The request raced a resize or a re-selection:
                        // configure the arriving parts to the config now
                        // current, not the one the request captured.
                        for part in &mut parts {
                            part.reselect(&self.config.adapter, &self.config.device)
                                .map_err(|error| RenderError::Render(error.to_string()))?;
                            part.resize(&self.config.device, self.config.size);
                        }
                    }
                    self.parts.extend(parts);
                    consumed += 1;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("main part creation lost its reply")
                }
            }
        }
        self.replies.drain(..consumed);
        Ok(())
    }

    fn enqueue(&self, update: Update) {
        let scene = self.scene.clone();
        let wait = update.frame.wait.clone();
        let enqueue = move || {
            scene.run(move |scene, _| {
                // Removal is a valid cancellation of an in-flight producer.
                if scene.displays.contains_key(&update.layer) {
                    let _tx = Transaction::begin();
                    scene.show(update.layer, &update.frame, update.generation);
                }
            });
        };
        match &wait {
            None => enqueue(),
            Some(FrameSync::Metal { event, value }) => {
                let event = event.clone();
                let value = *value;
                let enqueue = std::cell::Cell::new(Some(enqueue));
                let block = block2::RcBlock::new(
                    move |_: NonNull<ProtocolObject<dyn MTLSharedEvent>>, _: u64| {
                        enqueue.take().expect("one producer completion")();
                    },
                );
                // SAFETY: the event retains the block until it fires. The
                // callback only submits owned Send messages to main.
                unsafe {
                    event.notifyListener_atValue_block(
                        &MTLSharedEventListener::sharedListener(),
                        value,
                        block2::RcBlock::as_ptr(&block),
                    );
                }
            }
        }
    }
}

impl Compositor for LayerPlanes {
    const BUDGET: usize = 2;
    // A hosted layer's holder carries the plane's opacity.
    const HOSTS_OPACITY: bool = true;

    fn expresses_transform(transform: Affine) -> bool {
        transform.as_coeffs().iter().all(|c| c.is_finite())
    }

    fn expresses_clip(clip: &ShapeData) -> bool {
        LayerClip::of(clip).is_some()
    }

    /// A hosted view's path may carry only what the view API expresses:
    /// a translation plus a positive, axis-aligned scale — the Android
    /// predicate's shape.
    #[cfg(target_os = "macos")]
    fn hosts_transform(transform: Affine) -> bool {
        hosted_transform(transform)
    }

    fn shows(frame: &ExternalFrame) -> bool {
        // The proven external class is opaque BGRA8/sRGB. YUV range
        // expansion, BT.1886, and HDR tone mapping differ from the engine;
        // they remain engine-composited until their platform parity is
        // established. The second admitted class is the engine's own frame
        // ring — premultiplied f16 in the working space: its declared
        // decode is the identity, so the layer shows exactly the pixels
        // the engine composited.
        let video = matches!(&frame.planes, FramePlanes::Rgb { plane, .. }
            if plane.format() == wgpu::TextureFormat::Bgra8Unorm)
            && frame.color.transfer == Transfer::Srgb
            && frame.color.primaries == Primaries::Bt709;
        let ring = matches!(&frame.planes, FramePlanes::Rgb { plane, .. }
            if plane.format() == wgpu::TextureFormat::Rgba16Float)
            && frame.color.transfer == Transfer::Linear
            && frame.color.primaries == Primaries::DisplayP3;
        (video || ring) && frame_surface(frame).is_some()
    }
}

/// What an Apple window surface's frame commits on the platform main
/// thread: its `CATransaction` of layer geometry and every part's
/// drawable present. [`SystemPlanes::compose`] builds one on the render
/// thread; the engine replies it to the frame's awaiting caller, which
/// applies it before `render` returns — so a part's drawable always
/// presents before the next frame's acquire, and no present waits on a
/// queued main-queue block (#2261). The trait keeps the payload opaque:
/// the concrete commit names main-thread-only types that must never
/// become part of the public interface.
pub trait CommitApply: Send + 'static {
    /// Runs the commit's `CATransaction` — geometry and every part's
    /// drawable present — on main.
    fn apply(self: Box<Self>, mtm: MainThreadMarker);
}

/// A rendered frame's [`CommitApply`]: the whole `CATransaction` of
/// raster `contents`, hosted views, layer `placements` and every part's
/// acquired drawable. Nothing on the render thread may consume one:
/// dropping it would release the drawables unpresented.
#[must_use]
struct MainCommit {
    /// The layer scene the commit rewrites.
    scene: MainOwned<LayerScene>,
    /// New raster `IOSurface` contents, keyed by their promoted layer.
    contents: Vec<(LayerId, raster::Contents)>,
    /// The frame's hosted objects.
    hosted: Vec<(LayerId, super::Hosted, Size)>,
    /// The frame's plane placements.
    placements: Vec<Placement>,
    /// The surface size in device pixels.
    size: (u32, u32),
    /// The display scale.
    scale: f64,
    /// Every part's acquired drawable, presented in part order.
    frames: Vec<wgpu::SurfaceTexture>,
    /// The queue the drawables present through.
    queue: wgpu::Queue,
}

impl CommitApply for MainCommit {
    fn apply(self: Box<Self>, mtm: MainThreadMarker) {
        let Self {
            scene,
            contents,
            hosted,
            placements,
            size,
            scale,
            frames,
            queue,
        } = *self;
        scene.with(mtm, |scene, mtm| {
            let _tx = Transaction::begin();
            #[cfg(target_os = "macos")]
            let responder = scene.hosted_responder();
            for (layer, contents) in contents {
                contents.set(scene.display(layer));
            }
            scene.host(hosted, mtm);
            #[cfg(target_os = "macos")]
            {
                scene.place(&placements, size, scale, frames.len(), mtm);
                scene.restore_responder(responder);
            }
            #[cfg(not(target_os = "macos"))]
            scene.place(&placements, size, scale, frames.len());
            for frame in frames {
                queue.present(frame);
            }
        });
    }
}

impl SystemPlanes for LayerPlanes {
    /// `None` for a frame that produced no main-thread work; `Some` of the
    /// frame's [`CommitApply`] otherwise.
    type Commit = Option<Box<dyn CommitApply>>;

    fn captured_bytes(&self) -> u64 {
        self.buffers.values().map(raster::Buffer::bytes).sum()
    }
    fn animate(&mut self, tree: &cherenkov::SurfaceTree, plan: &super::Plan) {
        let motions: Vec<_> = plan
            .planes
            .iter()
            // A hosted plane's leaf is a view on macOS: AppKit owns a
            // layer-backed view's layer, so a position or opacity track
            // cannot be installed on it — the engine keeps sampling it
            // and each `place` applies the sampled geometry.
            .filter(|plane| {
                cfg!(not(target_os = "macos")) || !matches!(plane.source, Source::Hosted)
            })
            .filter_map(|plane| super::animation::motion(tree, plane.layer))
            .filter(|motion| {
                super::animation::safe_path(tree, motion.layer, plan.planes.iter().map(|p| p.layer))
            })
            .collect();
        self.owned_animations.clear();
        self.owned_animations
            .extend(motions.iter().map(|motion| motion.layer));
        if !self.motion.update(&motions) {
            return;
        }
        let placements = self.placements.clone();
        self.scene.run(move |scene, _| {
            let _tx = Transaction::begin();
            for plane in &scene.planes {
                #[cfg(target_os = "macos")]
                let levels = match &plane.nodes {
                    PlaneNodes::Layers { levels, .. } => levels,
                    PlaneNodes::Views { .. } => {
                        // Hosted planes never own a motion: only the
                        // opacity application above matters, on the leaf.
                        let placement = placements
                            .iter()
                            .find(|p| p.layer == plane.layer)
                            .expect("committed placement");
                        scene.hosted[&plane.layer]
                            .leaf
                            .setAlphaValue(f64::from(placement.opacity));
                        continue;
                    }
                };
                #[cfg(not(target_os = "macos"))]
                let levels = &plane.levels;
                let node = &levels.last().expect("a plane has a layer path").node;
                let display = scene.display(plane.layer);
                let next = motions.iter().find(|motion| motion.layer == plane.layer);
                let previous = scene.motions.get(&plane.layer);
                let placement = placements
                    .iter()
                    .find(|p| p.layer == plane.layer)
                    .expect("committed placement");
                if next.is_none_or(|motion| motion.position.is_none()) {
                    place(
                        placement.path.last().expect("leaf level"),
                        levels.last().expect("leaf layers"),
                        false,
                    );
                }
                if next.is_none_or(|motion| motion.opacity.is_none()) {
                    display.setOpacity(placement.opacity);
                }
                if let Some(motion) = next {
                    if let Some(position) = motion.position {
                        let [a, b, c, d] = motion.linear;
                        node.setAffineTransform(cg_affine(Affine::new([a, b, c, d, 0., 0.])));
                        node.setPosition(CGPoint::new(position[0].target, position[1].target));
                    }
                    if let Some(opacity) = motion.opacity {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "opacity is an f32 property"
                        )]
                        display.setOpacity(opacity.target as f32);
                    }
                }
                if previous == next {
                    continue;
                }
                animation::remove(node, "position.x");
                animation::remove(node, "position.y");
                animation::remove(display, "opacity");
                if let Some(motion) = next {
                    if let Some(position) = motion.position {
                        animation::install(node, position[0], "position.x");
                        animation::install(node, position[1], "position.y");
                    }
                    if let Some(opacity) = motion.opacity {
                        animation::install(display, opacity, "opacity");
                    }
                }
            }
            scene.motions.clear();
            scene
                .motions
                .extend(motions.into_iter().map(|motion| (motion.layer, motion)));
        });
    }

    fn owned_animations(&self) -> &[LayerId] {
        &self.owned_animations
    }

    fn withdraw_animations(&mut self) {
        self.owned_animations.clear();
    }

    fn compose(
        &mut self,
        mut c: Composition<'_>,
    ) -> Result<(super::Presentation, Self::Commit), RenderError> {
        let Some(contents) = self.raster_contents(&mut c)? else {
            return Ok((super::Presentation::Pending, None));
        };
        self.collect_parts()?;
        self.request_parts(c.parts.len(), None);
        if self.parts.len() < c.parts.len() {
            return Ok((super::Presentation::Pending, None));
        }
        let mut frames = Vec::with_capacity(c.parts.len());
        for (part, target) in c.parts.iter().zip(&self.parts) {
            let Some(frame) =
                c.presenter
                    .prepare(c.device, c.queue, target, part.view, c.display.headroom)?
            else {
                return Ok((super::Presentation::Retry, None));
            };
            frames.push(frame);
        }
        let placements: Vec<_> = c
            .planes
            .iter()
            .map(|plane| plane.placement.clone())
            .collect();
        let hosted = hosted_planes(c.planes);
        let next_hosted: FxHashSet<LayerId> = hosted.iter().map(|(id, ..)| *id).collect();
        let size = c.size;
        let scale = c.display.scale;
        let tree_changed = placements.iter().any(|placement| {
            self.motion.owns(placement.layer)
                && !self.placements.iter().any(|built| {
                    reuses_native_placement(
                        built,
                        placement,
                        &self.static_candidates,
                        &self.hosted,
                        &next_hosted,
                    )
                })
        });
        self.placements.clone_from(&placements);
        self.hosted = next_hosted;
        self.motion.placed(tree_changed);
        let commit = MainCommit {
            scene: self.scene.clone(),
            contents,
            hosted,
            placements,
            size,
            scale,
            frames,
            queue: c.queue.clone(),
        };
        for plane in c.planes {
            if let Some(update) = Update::from_plane(plane) {
                self.enqueue(update);
            }
        }
        Ok((super::Presentation::Presented, Some(Box::new(commit) as _)))
    }

    fn groom_with_frames(
        &mut self,
        candidates: &FxHashMap<LayerId, super::Candidate>,
        frames: &FxHashMap<LayerId, (ExternalFrame, u64)>,
    ) {
        let removed: Vec<_> = self
            .candidates
            .keys()
            .filter(|id| !candidates.contains_key(id) || !frames.contains_key(id))
            .copied()
            .collect();
        for id in removed {
            self.candidates.remove(&id);
            self.scene.run(move |scene, _| {
                if let Some(display) = scene.displays.remove(&id) {
                    // The candidate is logically gone now — a queued
                    // `show` finds no display — but the detach is visual
                    // state and commits only inside the next `place`'s
                    // transaction.
                    scene.retired.push(display);
                }
            });
        }
        let removed: Vec<_> = self
            .static_candidates
            .iter()
            .filter(|id| {
                candidates
                    .get(id)
                    .is_none_or(|candidate| candidate.source != Source::Recorded)
            })
            .copied()
            .collect();
        for id in removed {
            self.static_candidates.remove(&id);
            self.buffers.remove(&id);
            self.scene.run(move |scene, _| {
                if let Some(layer) = scene.rasters.remove(&id) {
                    scene.retired_rasters.push(layer);
                }
            });
        }
        for (&id, _) in candidates
            .iter()
            .filter(|(_, candidate)| candidate.source == Source::Recorded)
        {
            if self.static_candidates.insert(id) {
                self.scene.run(move |scene, _| {
                    scene.rasters.insert(id, anchored());
                });
            }
        }
        self.request_parts(candidates.len().min(Self::BUDGET) + 1, None);
        for &layer in candidates.keys() {
            let Some((frame, generation)) = frames.get(&layer).cloned() else {
                continue;
            };
            let std::collections::hash_map::Entry::Vacant(slot) = self.candidates.entry(layer)
            else {
                continue;
            };
            let ready = Arc::new(AtomicBool::new(false));
            let live = Arc::downgrade(&ready);
            let waker = self.waker.clone();
            let owner = self.scene.clone();
            self.scene.run(move |scene, _| {
                if let Some(flag) = live.upgrade() {
                    let _tx = Transaction::begin();
                    // Attach in the hierarchy: the platform reports
                    // `readyForDisplay` only for a layer the render
                    // server can see. The flag stays false until the
                    // gated sample lands and the layer's readiness
                    // notification refreshes it.
                    scene.attach(layer, &owner, Arc::downgrade(&flag), waker.clone());
                    // The attach's completion signal: the flag may
                    // still be false — readiness lands through the
                    // layer's own notification — but the queued work
                    // is done.
                    waker.wake();
                }
            });
            slot.insert(ready);
            // The probe's sample goes through `enqueue`'s producer-sync
            // gate like a promoted plane's: readiness read from a surface
            // whose producer has not signalled would not be the frame's.
            self.enqueue(Update {
                layer,
                frame,
                generation,
            });
        }
        let previous = self.offered.len();
        self.offered.retain(|layer| candidates.contains_key(layer));
        self.woke = previous != self.offered.len();
        for &layer in &self.static_candidates {
            self.woke |= self.offered.insert(layer);
        }
        for (&layer, ready) in &self.candidates {
            if ready.load(Ordering::Acquire) {
                self.woke |= self.offered.insert(layer);
            } else {
                // A layer the plan was offered that lost readiness needs
                // the same re-plan a newly ready one does.
                self.woke |= self.offered.remove(&layer);
            }
        }
    }

    fn wants_plan(&self) -> bool {
        self.woke
    }

    fn prepare(
        &mut self,
        candidates: &FxHashMap<LayerId, super::Candidate>,
        frames: &FxHashMap<LayerId, (ExternalFrame, u64)>,
        ready: &mut FxHashSet<LayerId>,
    ) {
        self.groom_with_frames(candidates, frames);
        ready.clear();
        ready.extend(
            self.candidates
                .iter()
                .filter(|(_, flag)| flag.load(Ordering::Acquire))
                .map(|(&layer, _)| layer),
        );
        ready.extend(self.static_candidates.iter().copied());
    }

    fn refresh<'a>(&mut self, frames: impl Iterator<Item = Plane<'a>>) -> Result<(), RenderError> {
        let mut changed = Vec::new();
        for plane in frames {
            let previous = self
                .placements
                .iter_mut()
                .find(|p| p.layer == plane.placement.layer)
                .expect("refresh names a committed plane");
            if previous != plane.placement {
                previous.clone_from(plane.placement);
                changed.push(plane.placement.clone());
            }
            if let Some(update) = Update::from_plane(&plane) {
                self.enqueue(update);
            }
        }
        if !changed.is_empty() {
            self.scene.run(move |scene, _| {
                let _tx = Transaction::begin();
                for placement in changed {
                    let layers = scene
                        .planes
                        .iter()
                        .find(|plane| plane.layer == placement.layer)
                        .expect("refresh names a committed plane");
                    #[cfg(target_os = "macos")]
                    if let PlaneNodes::Views { container, clips } = &layers.nodes {
                        let node = &scene.hosted[&placement.layer];
                        place_views(&placement, container, clips, node);
                        node.leaf.setAlphaValue(f64::from(placement.opacity));
                        continue;
                    }
                    #[cfg(target_os = "macos")]
                    let PlaneNodes::Layers { levels, .. } = &layers.nodes else {
                        unreachable!()
                    };
                    #[cfg(not(target_os = "macos"))]
                    let levels = &layers.levels;
                    for (level, layers) in placement.path.iter().zip(levels) {
                        place(
                            level,
                            layers,
                            scene
                                .motions
                                .get(&level.layer)
                                .is_some_and(|motion| motion.position.is_some()),
                        );
                    }
                    if scene
                        .motions
                        .get(&placement.layer)
                        .is_none_or(|motion| motion.opacity.is_none())
                    {
                        scene.display(placement.layer).setOpacity(placement.opacity);
                    }
                }
            });
        }
        Ok(())
    }

    fn resize(&mut self, size: (u32, u32)) {
        self.config.size = size;
        self.config_generation += 1;
        for part in &mut self.parts {
            part.resize(&self.config.device, size);
        }
    }

    fn reselect(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> Result<(), SurfaceError> {
        self.config.adapter = adapter.clone();
        self.config.device = device.clone();
        self.config_generation += 1;
        for part in &mut self.parts {
            part.reselect(adapter, device)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
