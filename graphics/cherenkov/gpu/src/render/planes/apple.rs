//! The Apple realization of system-compositor planes: a Core Animation
//! layer tree the engine owns under the host view's backing layer.
//!
//! ```text
//! host layer (the view's)
//! ├─ pending displays            candidates proving `readyForDisplay`
//! └─ root                        the surface, in points
//!    ├─ part 0: CAMetalLayer     engine content below the first plane
//!    ├─ plane 0                  pixel space: scale(1 / display scale)
//!    │  └─ level … level         one node per tree layer on the path:
//!    │     └─ display layer        transform → clip → scroll
//!    ├─ part 1: CAMetalLayer     engine content above plane 0
//!    └─ …
//! ```
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
use kurbo::{Affine, Rect, Vec2};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker};
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
use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol};
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
        let layer = match handle.window_handle().expect("window handle").as_raw() {
            #[cfg(target_os = "macos")]
            RawWindowHandle::AppKit(view) => {
                // SAFETY: the window handle guarantees a live view; we are on main.
                let view: &objc2_app_kit::NSView = unsafe { view.ns_view.cast().as_ref() };
                view.setWantsLayer(true);
                view.layer().expect("layer-backed view")
            }
            #[cfg(not(target_os = "macos"))]
            RawWindowHandle::UiKit(view) => {
                // SAFETY: the window handle guarantees a live view; we are on main.
                let view: &objc2_ui_kit::UIView = unsafe { view.ui_view.cast().as_ref() };
                view.layer()
            }
            other => panic!("expected an Apple view, found {other:?}"),
        };
        let _tx = Transaction::begin();
        let root = anchored();
        root.setName(Some(&objc2_foundation::NSString::from_str("cherenkov")));
        #[cfg(target_os = "macos")]
        root.setGeometryFlipped(!layer.contentsAreFlipped());
        layer.addSublayer(&root);
        Self {
            scene: MainOwned::new(
                LayerScene {
                    _window: handle,
                    host: layer,
                    root,
                    parts: Vec::new(),
                    planes: Vec::new(),
                    displays: FxHashMap::default(),
                    retired: Vec::new(),
                    motions: FxHashMap::default(),
                    rasters: FxHashMap::default(),
                    retired_rasters: Vec::new(),
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

struct PlaneLayers {
    layer: LayerId,
    raster: bool,
    shape: Vec<(LayerId, bool)>,
    top: Retained<CALayer>,
    levels: Vec<LevelLayers>,
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

/// All Core Animation access, including hierarchy changes and destruction,
/// is confined to this main-thread scene.
struct LayerScene {
    _window: Box<dyn wgpu::WindowHandle>,
    /// The host layer `root` sits under. Candidates' pending displays
    /// attach beside `root`: inside the committed hierarchy, which
    /// `readyForDisplay` requires, and outside the part/plane stack.
    host: Retained<CALayer>,
    root: Retained<CALayer>,
    parts: Vec<Retained<CAMetalLayer>>,
    planes: Vec<PlaneLayers>,
    displays: FxHashMap<LayerId, DisplayLayer>,
    /// Displays of demoted candidates, logically gone from `displays` at
    /// once but detached from the hierarchy only inside the next `place`'s
    /// transaction, so the removal never commits a frame on its own.
    retired: Vec<DisplayLayer>,
    motions: FxHashMap<LayerId, super::animation::Motion>,
    rasters: FxHashMap<LayerId, Retained<CALayer>>,
    retired_rasters: Vec<Retained<CALayer>>,
}

impl Drop for LayerScene {
    fn drop(&mut self) {
        let _tx = Transaction::begin();
        // Pending and retired displays are `host` sublayers of their own.
        for display in self.displays.values().chain(&self.retired) {
            display.display.removeFromSuperlayer();
        }
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

/// Whether `place` keeps the native nodes it built for `built`. Reuse is the
/// leaf, the path shape [`shape`] records, and the raster class stored when
/// that plane was built. A recorded placement is that class: groom inserts a
/// raster layer exactly for a recorded candidate.
fn reuses_native_nodes(built: &Placement, next: &Placement, rasters: &FxHashSet<LayerId>) -> bool {
    built.layer == next.layer
        && (built.source == Source::Recorded) == rasters.contains(&next.layer)
        && built.path.len() == next.path.len()
        && built
            .path
            .iter()
            .zip(&next.path)
            .all(|(level, next_level)| {
                level.layer == next_level.layer && level.clip.is_some() == next_level.clip.is_some()
            })
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
        if display.superlayer().as_deref() != Some(&*self.host) {
            self.host.addSublayer(display);
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

    fn plane_layers(&self, placement: &Placement, scale: f64) -> PlaneLayers {
        let top = anchored();
        top.setAffineTransform(cg_affine(Affine::scale(1.0 / scale)));
        let mut outer = top.clone();
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
            .map_or(&*top, LevelLayers::inner)
            .addSublayer(display);
        PlaneLayers {
            layer: placement.layer,
            raster: self.rasters.contains_key(&placement.layer),
            shape: shape(placement),
            top,
            levels,
        }
    }

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
                p.layer == placement.layer
                    && p.shape == shape(placement)
                    && p.raster == self.rasters.contains_key(&placement.layer)
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

    fn display(&self, layer: LayerId) -> &CALayer {
        match self.rasters.get(&layer) {
            Some(raster) => raster,
            None => &self.displays[&layer].display,
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
        unsafe { renderer.enqueueSampleBuffer(&sample) };
        assert_ne!(
            unsafe { renderer.status() },
            AVQueuedSampleBufferRenderingStatus::Failed,
            "system compositor rejected the frame: {:?}",
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
            motion: MotionState::default(),
        };
        result.request_parts(1, probe);
        result
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
        self.scene.run(move |scene, _| {
            let result = scene.add_parts(&config, count, probe);
            if send.send((generation, result)).is_err() {
                // Cancellation: the render owner was destroyed while this
                // command was queued. Its scene release is queued behind us.
                scene.root.removeFromSuperlayer();
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

    fn expresses_transform(transform: Affine) -> bool {
        transform.as_coeffs().iter().all(|c| c.is_finite())
    }

    fn expresses_clip(clip: &ShapeData) -> bool {
        LayerClip::of(clip).is_some()
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

impl SystemPlanes for LayerPlanes {
    fn captured_bytes(&self) -> u64 {
        self.buffers.values().map(raster::Buffer::bytes).sum()
    }
    fn animate(&mut self, tree: &cherenkov::SurfaceTree, plan: &super::Plan) {
        let motions: Vec<_> = plan
            .planes
            .iter()
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
                let node = &plane.levels.last().expect("a plane has a layer path").node;
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
                        plane.levels.last().expect("leaf layers"),
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

    fn compose(&mut self, c: Composition<'_>) -> Result<super::Presentation, RenderError> {
        // The immutable IOSurface is published only after the GPU has
        // finished its presentation conversion. Until then the committed
        // scene and all of its old buffers stay visible together.
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
        if !ready {
            return Ok(super::Presentation::Pending);
        }
        self.collect_parts()?;
        self.request_parts(c.parts.len(), None);
        if self.parts.len() < c.parts.len() {
            return Ok(super::Presentation::Pending);
        }
        let mut frames = Vec::with_capacity(c.parts.len());
        for (part, target) in c.parts.iter().zip(&self.parts) {
            let Some(frame) =
                c.presenter
                    .prepare(c.device, c.queue, target, part.view, c.display.headroom)?
            else {
                return Ok(super::Presentation::Retry);
            };
            frames.push(frame);
        }
        let placements: Vec<_> = c
            .planes
            .iter()
            .map(|plane| plane.placement.clone())
            .collect();
        let size = c.size;
        let scale = c.display.scale;
        let tree_changed = placements.iter().any(|placement| {
            self.motion.owns(placement.layer)
                && !self
                    .placements
                    .iter()
                    .any(|built| reuses_native_nodes(built, placement, &self.static_candidates))
        });
        self.placements.clone_from(&placements);
        self.motion.placed(tree_changed);
        let queue = c.queue.clone();
        self.scene.run(move |scene, _| {
            let _tx = Transaction::begin();
            for (layer, contents) in contents {
                contents.set(scene.display(layer));
            }
            scene.place(&placements, size, scale, frames.len());
            for frame in frames {
                queue.present(frame);
            }
        });
        for plane in c.planes {
            if let Some(update) = Update::from_plane(plane) {
                self.enqueue(update);
            }
        }
        Ok(super::Presentation::Presented)
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
            .filter(|id| !candidates.contains_key(id) || frames.contains_key(id))
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
        for &id in candidates.keys().filter(|id| !frames.contains_key(id)) {
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
                    for (level, layers) in placement.path.iter().zip(&layers.levels) {
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
