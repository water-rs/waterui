//! Core Animation layers for [`Node`]s, and the in-place edits that follow
//! a live operand.
//!
//! A node builds the cheapest layer that carries it: a colour fill or
//! stroke is a `CAShapeLayer`; a gradient or image pattern is a layer
//! Core Graphics draws, under a `CAShapeLayer` mask of the shape; a mesh
//! gradient is a layer showing the mesh pass's surface under such a mask;
//! an image is a layer's contents; a clip, transform or group is a
//! container layer whose properties carry the operand. A run of commands
//! Core Graphics draws together ([`super::replay`]) is a layer whose
//! contents are that drawing. [`Built::update`] writes only what differs
//! between the old node and the new one.
//!
//! # Safety
//!
//! The `unsafe` here sets layer properties from layer-framework constants
//! and hands Core Animation a `CGImage` as layer contents; every object is
//! alive across its call.

use core::cell::{OnceCell, RefCell};
use core::ops::Range;
use core::sync::atomic::{AtomicU64, Ordering};
use std::rc::Rc;
use std::sync::Arc;

use cocoa_ui::mesh::{Mesh, MeshError, MeshPass, MeshSurface, Placement};
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use cocoa_ui::objc2::{AnyThread, DefinedClass, define_class, msg_send};
use cocoa_ui::objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use cocoa_ui::objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImageAlphaInfo,
    CGImageByteOrderInfo, CGImageComponentInfo, CGPath, kCGColorSpaceExtendedSRGB,
};
use cocoa_ui::objc2_foundation::{NSArray, NSNull, NSNumber, NSObjectProtocol, NSString};
#[cfg(all(test, target_os = "macos"))]
use cocoa_ui::objc2_metal::{MTLCommandBuffer, MTLCommandBufferStatus};
use cocoa_ui::objc2_quartz_core::{
    CAAction, CADynamicRangeHigh, CALayer, CALayerDelegate, CAShapeLayer,
    kCAContentsFormatRGBA16Float, kCAFillRuleEvenOdd, kCAFillRuleNonZero, kCAFilterLinear,
    kCAFilterNearest, kCAGravityResize, kCALineCapButt, kCALineCapRound, kCALineCapSquare,
    kCALineJoinBevel, kCALineJoinMiter, kCALineJoinRound,
};
use kurbo::{Affine, Cap, Join, Rect, Stroke};
use waterui_graphics::backend::draw::{
    BlendMode, Command, FillRule, MeshColorInterpolation, MeshGradient, Paint, Sampling, Shadow,
    ShapeData, WorkingColor,
};

use super::cg;
use super::draw;
use super::glyphs;
use super::plan::{self, Composite, Geometry, Node, PaintClass};
use super::replay::{self, SHADOW_REACH_SIGMAS};
use super::resources::Registry;

/// Tells the canvas's host that a mesh paint has finished, so it shows it
/// ([`super::tree::Tree::show_completed`]); called on the Metal thread
/// that reports the paint complete.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// What building and editing layers reads beyond the node itself.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    /// The registered images and fonts.
    pub registry: &'a Rc<Registry>,
    /// Device pixels per point.
    pub scale: f64,
    /// The mesh gradient pass, opened when the first mesh needs it.
    pub mesh: &'a OnceCell<Option<Rc<MeshPass>>>,
    /// Called once a mesh paint has finished.
    pub wake: &'a Wake,
}

impl core::fmt::Debug for Ctx<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ctx")
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

/// A mesh paint the GPU is running.
struct Pending {
    /// What a test waits on; the host waits for the completion instead.
    #[cfg(all(test, target_os = "macos"))]
    buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    generation: u64,
    /// The surface it paints.
    surface: usize,
}

/// The surfaces a mesh layer alternates between: one shown, the other
/// painted, so the shown one is never written while Core Animation reads
/// it.
struct Surfaces {
    targets: [Option<MeshSurface>; 2],
    /// The surface the layer shows.
    front: Option<usize>,
    /// The latest paint's generation.
    generation: u64,
    /// The latest paint, until it is shown.
    pending: Option<Pending>,
    /// The latest generation the GPU has finished, written by the paint's
    /// completion.
    done: Arc<AtomicU64>,
}

/// A mesh gradient's layer and what repainting it needs.
struct MeshState {
    pass: Rc<MeshPass>,
    layer: Retained<CALayer>,
    mask: Retained<CAShapeLayer>,
    mesh: Mesh,
    /// Maps the mesh's coordinates into the layer's space.
    transform: Affine,
    surfaces: RefCell<Surfaces>,
}

impl core::fmt::Debug for MeshState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeshState").finish_non_exhaustive()
    }
}

impl MeshState {
    /// Paints the mesh at `scale` into the surface the layer does not
    /// show; the layer shows it once the GPU is done and the host is woken.
    fn paint(&self, scale: f64, wake: &Wake) {
        let bounds = self.layer.bounds();
        self.layer.setContentsScale(scale);
        self.mask.setContentsScale(scale);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a layer's pixel size is a small non-negative integer"
        )]
        let (width, height) = (
            (bounds.size.width * scale).ceil() as usize,
            (bounds.size.height * scale).ceil() as usize,
        );
        let mut surfaces = self.surfaces.borrow_mut();
        if width == 0 || height == 0 {
            surfaces.pending = None;
            // SAFETY: nil clears the contents.
            unsafe { self.layer.setContents(None) };
            return;
        }
        let back = surfaces.front.map_or(0, |front| 1 - front);
        surfaces.generation += 1;
        let generation = surfaces.generation;
        let placement = Placement {
            rect: [
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height,
            ],
            inverse: self.transform.inverse().as_coeffs(),
        };
        if surfaces.targets[back].as_ref().map(MeshSurface::size) != Some((width, height)) {
            match self.pass.surface(width, height) {
                Ok(surface) => surfaces.targets[back] = Some(surface),
                Err(error) => {
                    self.abandon(&mut surfaces, &error);
                    return;
                }
            }
        }
        let target = surfaces.targets[back]
            .as_ref()
            .expect("the back surface was just made or already had this size");
        let (done, wake) = (Arc::clone(&surfaces.done), Arc::clone(wake));
        #[cfg_attr(
            not(all(test, target_os = "macos")),
            expect(unused_variables, reason = "only a macOS render test waits on it")
        )]
        let buffer = match self
            .pass
            .paint_to_surface(&self.mesh, placement, target, move || {
                done.fetch_max(generation, Ordering::Release);
                wake();
            }) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.abandon(&mut surfaces, &error);
                return;
            }
        };
        surfaces.pending = Some(Pending {
            #[cfg(all(test, target_os = "macos"))]
            buffer,
            generation,
            surface: back,
        });
    }

    /// A paint the device refused: logged, and the layer shows nothing
    /// rather than a stale mesh — the mesh is not drawn.
    fn abandon(&self, surfaces: &mut Surfaces, error: &MeshError) {
        tracing::error!("the mesh gradient paint failed: {error}; the mesh is not drawn");
        surfaces.pending = None;
        surfaces.front = None;
        // SAFETY: nil clears the contents.
        unsafe { self.layer.setContents(None) };
    }

    /// Shows the latest paint if the GPU has finished it. Paints on one
    /// queue finish in order, so the latest being done is all it takes.
    fn show_completed(&self) {
        let mut surfaces = self.surfaces.borrow_mut();
        let finished = surfaces.done.load(Ordering::Acquire);
        if surfaces
            .pending
            .as_ref()
            .is_some_and(|pending| pending.generation <= finished)
        {
            Self::show(&self.layer, &mut surfaces);
        }
    }

    fn show(layer: &CALayer, surfaces: &mut Surfaces) {
        let pending = surfaces.pending.take().expect("a paint to show");
        surfaces.targets[pending.surface]
            .as_ref()
            .expect("a painted surface exists")
            .show(layer);
        surfaces.front = Some(pending.surface);
    }

    /// Waits for the latest paint and shows it: for a test that
    /// rasterizes the tree before any host is woken.
    #[cfg(all(test, target_os = "macos"))]
    fn settle(&self) {
        let mut surfaces = self.surfaces.borrow_mut();
        let Some(pending) = &surfaces.pending else {
            return;
        };
        pending.buffer.waitUntilCompleted();
        assert_eq!(
            pending.buffer.status(),
            MTLCommandBufferStatus::Completed,
            "the mesh paint completes"
        );
        Self::show(&self.layer, &mut surfaces);
    }
}

/// `paint`'s mesh uploaded for the pass.
fn upload(pass: &MeshPass, mesh: &MeshGradient) -> Mesh {
    let (points, colors) = plan::grid(mesh);
    pass.mesh(
        mesh.columns() as usize,
        mesh.rows() as usize,
        &points,
        &colors,
        mesh.interpolation_mode() == MeshColorInterpolation::Smoothstep,
    )
}

/// What a Core Graphics drawn layer paints.
struct Drawing {
    paint: Paint,
    registry: Rc<Registry>,
}

struct PainterIvars {
    drawing: RefCell<Drawing>,
}

define_class!(
    // SAFETY: an `NSObject` subclass with one interior-mutable ivar and no
    // `Drop`; Core Animation calls its delegate methods synchronously on the
    // thread that draws the layer.
    #[unsafe(super(NSObject))]
    #[name = "WaterUICanvasPainter"]
    #[ivars = PainterIvars]
    struct Painter;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for Painter {}
    // SAFETY: the drawing callback matches `CALayerDelegate`'s contract.
    unsafe impl CALayerDelegate for Painter {}

    impl Painter {
        // SAFETY: the signature is `CALayerDelegate`'s action lookup.
        #[unsafe(method_id(actionForLayer:forKey:))]
        fn action_for_layer_for_key(
            &self,
            _layer: &CALayer,
            _event: &NSString,
        ) -> Option<Retained<ProtocolObject<dyn CAAction>>> {
            // A non-nil `NSNull` ends the action search with no action: a
            // redrawn layer never cross-fades its old contents.
            Some(ProtocolObject::from_retained(NSNull::null()))
        }

        // SAFETY: the signature is `CALayerDelegate`'s drawing callback.
        #[unsafe(method(drawLayer:inContext:))]
        fn draw_layer_in_context(&self, layer: &CALayer, context: &CGContext) {
            let bounds = layer.bounds();
            let drawing = self.ivars().drawing.borrow();
            let extent = Rect::new(
                bounds.origin.x,
                bounds.origin.y,
                bounds.origin.x + bounds.size.width,
                bounds.origin.y + bounds.size.height,
            );
            CGContext::save_g_state(Some(context));
            if !layer.contentsAreFlipped() {
                // The context is y-up: draw in the canvas's y-down space.
                CGContext::translate_ctm(Some(context), 0.0, extent.y0 + extent.y1);
                CGContext::scale_ctm(Some(context), 1.0, -1.0);
            }
            draw::paint(context, &drawing.paint, extent, &drawing.registry);
            CGContext::restore_g_state(Some(context));
        }
    }
);

impl Painter {
    fn new(drawing: Drawing) -> Retained<Self> {
        let this = Self::alloc().set_ivars(PainterIvars {
            drawing: RefCell::new(drawing),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl core::fmt::Debug for Painter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Painter").finish_non_exhaustive()
    }
}

/// What a built layer keeps beside the layer itself.
#[derive(Debug)]
enum Parts {
    /// Nothing: the layer's own properties are the node.
    Plain,
    /// A `CAShapeLayer` that is the layer.
    Shape(Retained<CAShapeLayer>),
    /// A Core Graphics drawn layer and the mask that shapes it.
    Painted {
        painter: Retained<Painter>,
        mask: Retained<CAShapeLayer>,
    },
    /// A container whose clip is a mask.
    Mask(Retained<CAShapeLayer>),
    /// A clipping window and the layer, out of it, whose shadow shows.
    Shadow(Retained<CAShapeLayer>),
    /// A layer showing a mesh gradient's surface, under a shape mask.
    Mesh(MeshState),
}

/// A node's layer and what editing it needs.
#[derive(Debug)]
pub struct Built {
    /// The layer, to place in its parent.
    pub layer: Retained<CALayer>,
    parts: Parts,
}

/// A layer at the canvas origin: positions and transforms are the canvas's.
pub fn container() -> Retained<CALayer> {
    let layer = CALayer::new();
    layer.setAnchorPoint(CGPoint::ZERO);
    layer
}

fn shape_layer() -> Retained<CAShapeLayer> {
    let layer = CAShapeLayer::new();
    layer.setAnchorPoint(CGPoint::ZERO);
    layer
}

fn place(layer: &CALayer, rect: Rect) {
    layer.setBounds(CGRect::new(
        CGPoint::new(rect.x0, rect.y0),
        CGSize::new(rect.width(), rect.height()),
    ));
    layer.setPosition(CGPoint::new(rect.x0, rect.y0));
}

/// Asks for headroom when the content needs it: the display's extended
/// range shows colours above SDR white.
fn set_dynamic_range(layer: &CALayer, extended: bool) {
    if extended {
        // SAFETY: the static is a constant Core Animation exports.
        unsafe { layer.setPreferredDynamicRange(CADynamicRangeHigh) };
    }
}

/// Backs the layer with half-float storage and asks for headroom when the
/// colour needs it: the default 8-bit backing clamps to SDR.
fn set_range(layer: &CALayer, extended: bool) {
    if extended {
        // SAFETY: the static is a constant Core Animation exports.
        unsafe { layer.setContentsFormat(kCAContentsFormatRGBA16Float) };
    }
    set_dynamic_range(layer, extended);
}

/// What a shape layer draws for a fill or stroke: its path, how the path
/// fills, and the stroke the layer applies itself — `None` when the layer
/// fills the path. A stroke whose two ends take different caps has no
/// layer property, so the layer fills the stroke's outline instead.
struct Coverage {
    path: CFRetained<CGPath>,
    rule: FillRule,
    stroke: Option<Stroke>,
}

fn coverage(geometry: &Geometry, stroke: Option<&Stroke>, ctx: Ctx<'_>) -> Coverage {
    let (elements, rule) = match geometry {
        Geometry::Shape(shape) => (cg::elements(shape), cg::fill_rule(shape)),
        Geometry::Glyphs(run) => (
            glyphs::outline(ctx.registry, run).into_elements(),
            FillRule::NonZero,
        ),
    };
    match stroke {
        Some(stroke) if stroke.start_cap != stroke.end_cap => Coverage {
            path: cg::path_of(cg::stroke_outline(elements, stroke)),
            rule: FillRule::NonZero,
            stroke: None,
        },
        _ => Coverage {
            path: cg::path_of(elements),
            rule,
            stroke: stroke.cloned(),
        },
    }
}

/// The nonzero/even-odd constant for `rule`.
fn rule(layer: &CAShapeLayer, rule: FillRule) {
    // SAFETY: the statics are constants Core Animation exports.
    layer.setFillRule(unsafe {
        match rule {
            FillRule::NonZero => kCAFillRuleNonZero,
            FillRule::EvenOdd => kCAFillRuleEvenOdd,
        }
    });
}

/// Points `layer` at `coverage` and inks it with `color`.
fn shape(
    layer: &CAShapeLayer,
    coverage: &Coverage,
    color: &cocoa_ui::objc2_core_graphics::CGColor,
) {
    layer.setPath(Some(&coverage.path));
    rule(layer, coverage.rule);
    let Some(stroke) = &coverage.stroke else {
        layer.setFillColor(Some(color));
        layer.setStrokeColor(None);
        return;
    };
    debug_assert_eq!(
        stroke.start_cap, stroke.end_cap,
        "coverage outlines unequal caps"
    );
    layer.setFillColor(None);
    layer.setStrokeColor(Some(color));
    layer.setLineWidth(stroke.width);
    layer.setMiterLimit(stroke.miter_limit);
    // SAFETY: the statics are constants Core Animation exports.
    unsafe {
        layer.setLineCap(match stroke.start_cap {
            Cap::Butt => kCALineCapButt,
            Cap::Round => kCALineCapRound,
            Cap::Square => kCALineCapSquare,
        });
        layer.setLineJoin(match stroke.join {
            Join::Miter => kCALineJoinMiter,
            Join::Round => kCALineJoinRound,
            Join::Bevel => kCALineJoinBevel,
        });
    }
    if stroke.dash_pattern.is_empty() {
        layer.setLineDashPattern(None);
    } else {
        let lengths: Vec<_> = stroke
            .dash_pattern
            .iter()
            .map(|&d| NSNumber::new_f64(d))
            .collect();
        layer.setLineDashPattern(Some(&NSArray::from_retained_slice(&lengths)));
    }
    layer.setLineDashPhase(stroke.dash_offset);
}

fn coverage_extent(coverage: &Coverage) -> Rect {
    cg::bounds(&coverage.path).map_or(Rect::ZERO, |bounds| {
        let reach = cg::reach(coverage.stroke.as_ref());
        bounds.inflate(reach, reach)
    })
}

fn black() -> CFRetained<cocoa_ui::objc2_core_graphics::CGColor> {
    cg::color(WorkingColor::BLACK)
}

/// Points `mask` at the coverage and frames `layer` around it.
fn shape_mask(layer: &CALayer, mask: &CAShapeLayer, coverage: &Coverage) {
    shape(mask, coverage, &black());
    place(layer, coverage_extent(coverage));
}

fn set_contents_scale(layer: &CALayer, scale: f64) {
    layer.setContentsScale(scale);
}

fn sampling_filter(layer: &CALayer, sampling: Sampling) {
    // SAFETY: the statics are constants Core Animation exports.
    unsafe {
        let filter = match sampling {
            Sampling::Nearest => kCAFilterNearest,
            Sampling::Linear => kCAFilterLinear,
        };
        layer.setMagnificationFilter(filter);
        layer.setMinificationFilter(filter);
    }
}

fn set_contents(layer: &CALayer, image: &cocoa_ui::objc2_core_graphics::CGImage) {
    // SAFETY: a `CGImage` is a valid `CALayer.contents` value; Core
    // Animation retains it.
    unsafe {
        let contents: &AnyObject = &*core::ptr::from_ref(image).cast::<AnyObject>();
        layer.setContents(Some(contents));
    }
}

/// Composites the group's layer onto what is beneath it with `mode`.
///
/// # Panics
///
/// On a mode that composites against the canvas beneath the group, which
/// a run draws and no layer carries ([`plan::composite`]).
fn set_composite(layer: &CALayer, mode: BlendMode) {
    let (filter, hidden) = match plan::composite(mode) {
        Composite::Normal => (None, false),
        Composite::Filter(name) => (Some(NSString::from_str(name)), false),
        Composite::Hidden => (None, true),
        Composite::Backdrop => {
            unreachable!("a group blended with {mode:?} is drawn by its run, never a layer")
        }
    };
    layer.setHidden(hidden);
    // SAFETY: `compositingFilter` takes a filter name or nil.
    unsafe {
        layer.setCompositingFilter(filter.as_deref().map(|name| {
            let object: &NSObject = name;
            let any: &AnyObject = object;
            any
        }));
    }
}

/// Configures the layers that cast `shadow` from `shape` and draw nothing
/// else.
///
/// `caster` fills the spread shape opaquely and sits to the right of
/// `window` — a clipping layer around exactly the shadow — by a distance
/// its shadow offset brings back, so only the shadow reaches the window's
/// box. The fill is the shadow's shape wherever the layer tree is
/// rasterized: `renderInContext` (every capture of the tree) casts a shadow
/// from the layer's own alpha and ignores `shadowPath`. The same path as
/// `shadowPath` spares the on-screen compositor deriving it from the alpha.
fn set_shadow(window: &CALayer, caster: &CAShapeLayer, shape: &ShapeData, shadow: &Shadow) {
    let path = cg::spread(shape, shadow.spread);
    let Some(extent) = cg::bounds(&path) else {
        // The spread shrank the shape to nothing: no shadow.
        place(window, Rect::ZERO);
        caster.setPath(None);
        caster.setShadowPath(None);
        caster.setShadowOpacity(0.0);
        return;
    };
    // The caster's own space starts at its top-left corner.
    let local = cg::transform(Affine::translate((-extent.x0, -extent.y0)));
    // SAFETY: the transform outlives the call.
    let path = unsafe { CGPath::new_copy_by_transforming_path(Some(&path), &raw const local) }
        .expect("Core Graphics moves a live path");
    let reach = shadow.sigma.mul_add(SHADOW_REACH_SIGMAS, 1.0);
    let visible = (extent + shadow.offset).inflate(reach, reach);
    place(window, visible);
    window.setMasksToBounds(true);
    let away = visible.width() + extent.width() + shadow.offset.x.abs() + 2.0;
    caster.setBounds(CGRect::new(
        CGPoint::ZERO,
        CGSize::new(extent.width(), extent.height()),
    ));
    caster.setPosition(CGPoint::new(extent.x0 + away, extent.y0));
    caster.setPath(Some(&path));
    caster.setFillColor(Some(&black()));
    caster.setShadowPath(Some(&path));
    caster.setShadowColor(Some(&cg::color(shadow.color)));
    caster.setShadowOpacity(1.0);
    // Measured: a layer's shadow radius is the Gaussian's standard deviation.
    caster.setShadowRadius(shadow.sigma);
    caster.setShadowOffset(CGSize::new(shadow.offset.x - away, shadow.offset.y));
}

fn set_clip(built: &mut Built, shape: &ShapeData) {
    let layer = &built.layer;
    match shape {
        ShapeData::Rect(rect) => {
            // The layer's bounds are the clip; sublayers keep canvas
            // coordinates by sharing the bounds' origin.
            if let Parts::Mask(_) = built.parts {
                // SAFETY: nil clears the mask.
                unsafe { layer.setMask(None) };
                built.parts = Parts::Plain;
            }
            layer.setMasksToBounds(true);
            place(layer, *rect);
        }
        other => {
            layer.setMasksToBounds(false);
            place(layer, Rect::ZERO);
            let mask = if let Parts::Mask(mask) = &built.parts {
                mask.clone()
            } else {
                let mask = shape_layer();
                // SAFETY: `mask` is a live layer.
                unsafe { layer.setMask(Some(&mask)) };
                built.parts = Parts::Mask(mask.clone());
                mask
            };
            mask.setPath(Some(&cg::path(other)));
            rule(&mask, cg::fill_rule(other));
            mask.setFillColor(Some(&black()));
        }
    }
}

/// A layer for the run `range` of `commands` that Core Graphics draws.
pub fn run_layer(commands: &[Command], range: Range<usize>, ctx: Ctx<'_>) -> Retained<CALayer> {
    let layer = container();
    // SAFETY: the static is a constant Core Animation exports.
    layer.setContentsGravity(unsafe { kCAGravityResize });
    draw_run(&layer, commands, range, ctx);
    layer
}

/// Draws the run `range` of `commands` into a bitmap at the display scale
/// and shows it as `layer`'s contents, framed around what it draws.
///
/// The bitmap holds half floats in extended sRGB: an encoded space, which
/// is where the run's groups blend, with room for colours above SDR white.
///
/// When Core Graphics refuses a bitmap the size of the run, the failure is
/// logged and the run is not drawn.
pub fn draw_run(layer: &CALayer, commands: &[Command], range: Range<usize>, ctx: Ctx<'_>) {
    let scale = ctx.scale;
    let Some(extent) = replay::extent(commands, range.clone(), ctx.registry) else {
        // SAFETY: nil clears the contents.
        unsafe { layer.setContents(None) };
        place(layer, Rect::ZERO);
        return;
    };
    // Whole device pixels, so the bitmap's texels sit on the screen's.
    let extent = Rect::new(
        (extent.x0 * scale).floor() / scale,
        (extent.y0 * scale).floor() / scale,
        (extent.x1 * scale).ceil() / scale,
        (extent.y1 * scale).ceil() / scale,
    );
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the extent is whole, non-negative device pixels"
    )]
    let (width, height) = (
        (extent.width() * scale).round() as usize,
        (extent.height() * scale).round() as usize,
    );
    // SAFETY: the static is a `CFString` constant Core Graphics exports.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedSRGB }))
        .expect("Core Graphics provides extended sRGB");
    // SAFETY: a null buffer lets Core Graphics allocate one for the context;
    // the layout is four premultiplied half floats per pixel.
    let context = unsafe {
        CGBitmapContextCreate(
            core::ptr::null_mut(),
            width,
            height,
            16,
            width * 8,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0
                | CGImageComponentInfo::Float.0
                | CGImageByteOrderInfo::Order16Little.0,
        )
    };
    let Some(context) = context else {
        tracing::error!(
            "Core Graphics could not make a {width}x{height} half-float bitmap; \
             the run is not drawn"
        );
        // SAFETY: nil clears the contents.
        unsafe { layer.setContents(None) };
        place(layer, Rect::ZERO);
        return;
    };
    // The bitmap is y-up; the canvas is y-down with the extent's corner at
    // the bitmap's top-left.
    #[expect(
        clippy::cast_precision_loss,
        reason = "bitmap sides are far below 2^52"
    )]
    let pixels = (width as f64, height as f64);
    let view = Affine::translate((0.0, pixels.1))
        * Affine::scale_non_uniform(scale, -scale)
        * Affine::translate((-extent.x0, -extent.y0));
    CGContext::concat_ctm(Some(&context), cg::transform(view));
    replay::draw(
        replay::Target {
            context: &context,
            registry: ctx.registry,
            reach: pixels.0.max(pixels.1),
        },
        commands,
        range.clone(),
    );
    let Some(image) = CGBitmapContextCreateImage(Some(&context)) else {
        tracing::error!("Core Graphics could not capture the run's bitmap; the run is not drawn");
        // SAFETY: nil clears the contents.
        unsafe { layer.setContents(None) };
        place(layer, Rect::ZERO);
        return;
    };
    set_contents(layer, &image);
    layer.setContentsScale(scale);
    place(layer, extent);
    set_dynamic_range(layer, replay::extended(commands, range));
}

/// The transforms of `old` and `new` when the layer differs between them
/// only by where it is placed: a transform scope, or a picture or text
/// drawn under one. A text of another layout or colour is another layer.
fn moved<'a>(old: &'a Node, new: &'a Node) -> Option<(&'a Affine, &'a Affine)> {
    match (old, new) {
        (Node::Transform(from), Node::Transform(to))
        | (
            Node::Picture {
                transform: from, ..
            },
            Node::Picture { transform: to, .. },
        ) => Some((from, to)),
        (
            Node::Text {
                layout: old_layout,
                transform: from,
                paint: old_paint,
            },
            Node::Text {
                layout,
                transform: to,
                paint,
            },
        ) if old_layout == layout && old_paint == paint => Some((from, to)),
        _ => None,
    }
}

impl Built {
    /// The layer covering `geometry` with `paint`: a shape layer for a
    /// colour, otherwise a drawn or mesh layer under the geometry's mask.
    fn painted(geometry: &Geometry, stroke: Option<&Stroke>, paint: &Paint, ctx: Ctx<'_>) -> Self {
        let coverage = coverage(geometry, stroke, ctx);
        match plan::classify(paint) {
            PaintClass::Solid(color) => {
                let layer = shape_layer();
                shape(&layer, &coverage, &cg::color(color));
                set_range(&layer, cg::color_extended(color));
                Self {
                    layer: Retained::into_super(layer.clone()),
                    parts: Parts::Shape(layer),
                }
            }
            PaintClass::Drawn => {
                let painter = Painter::new(Drawing {
                    paint: paint.clone(),
                    registry: Rc::clone(ctx.registry),
                });
                let layer = container();
                layer.setDelegate(Some(ProtocolObject::from_ref(&*painter)));
                layer.setNeedsDisplayOnBoundsChange(true);
                let mask = shape_layer();
                // SAFETY: `mask` is a live layer.
                unsafe { layer.setMask(Some(&mask)) };
                shape_mask(&layer, &mask, &coverage);
                set_range(&layer, cg::paint_extended(paint));
                layer.setNeedsDisplay();
                Self {
                    layer,
                    parts: Parts::Painted { painter, mask },
                }
            }
            PaintClass::Mesh { mesh, transform } => {
                // The pass opens once per canvas: a refusal is logged then
                // and every mesh of the canvas draws nothing.
                let Some(pass) = ctx
                    .mesh
                    .get_or_init(|| match MeshPass::new() {
                        Ok(pass) => Some(Rc::new(pass)),
                        Err(error) => {
                            tracing::error!(
                                "the mesh gradient pass cannot open: {error}; \
                                 the canvas draws no mesh gradient"
                            );
                            None
                        }
                    })
                    .clone()
                else {
                    return Self {
                        layer: container(),
                        parts: Parts::Plain,
                    };
                };
                let layer = container();
                // SAFETY: the static is a constant Core Animation exports.
                layer.setContentsGravity(unsafe { kCAGravityResize });
                let mask = shape_layer();
                // SAFETY: `mask` is a live layer.
                unsafe { layer.setMask(Some(&mask)) };
                shape_mask(&layer, &mask, &coverage);
                set_dynamic_range(&layer, cg::paint_extended(paint));
                let state = MeshState {
                    mesh: upload(&pass, mesh),
                    pass,
                    layer: layer.clone(),
                    mask,
                    transform,
                    surfaces: RefCell::new(Surfaces {
                        targets: [None, None],
                        front: None,
                        generation: 0,
                        pending: None,
                        done: Arc::new(AtomicU64::new(0)),
                    }),
                };
                Self {
                    layer,
                    parts: Parts::Mesh(state),
                }
            }
        }
    }

    /// Builds the layer for `node`.
    ///
    /// # Panics
    ///
    /// On a shader paint, or a group that composites against the canvas
    /// beneath it, which a run draws.
    #[must_use]
    pub fn new(node: &Node, ctx: Ctx<'_>) -> Self {
        let built = match node {
            Node::Painted {
                geometry,
                stroke,
                paint,
            } => Self::painted(geometry, stroke.as_ref(), paint, ctx),
            Node::Image {
                image,
                dst,
                sampling,
            } => {
                let layer = container();
                set_contents(&layer, &ctx.registry.image(*image));
                // SAFETY: the static is a constant Core Animation exports.
                layer.setContentsGravity(unsafe { kCAGravityResize });
                sampling_filter(&layer, *sampling);
                place(&layer, *dst);
                Self {
                    layer,
                    parts: Parts::Plain,
                }
            }
            Node::Shadow { shape, shadow } => {
                let layer = container();
                let caster = shape_layer();
                layer.addSublayer(&caster);
                set_shadow(&layer, &caster, shape, shadow);
                Self {
                    layer,
                    parts: Parts::Shadow(caster),
                }
            }
            Node::Clip(shape) => {
                let mut built = Self {
                    layer: container(),
                    parts: Parts::Plain,
                };
                set_clip(&mut built, shape);
                built
            }
            Node::Transform(transform)
            | Node::Picture { transform, .. }
            | Node::Text { transform, .. } => {
                let layer = container();
                layer.setAffineTransform(cg::transform(*transform));
                Self {
                    layer,
                    parts: Parts::Plain,
                }
            }
            Node::Group(group) => {
                let layer = container();
                layer.setAllowsGroupOpacity(true);
                layer.setOpacity(group.opacity);
                set_composite(&layer, group.blend);
                plan::reject_group_filter(group);
                Self {
                    layer,
                    parts: Parts::Plain,
                }
            }
        };
        built.set_scale(ctx);
        built
    }

    /// Applies the difference between `old` and `new` — the same node
    /// kind with a live operand moved — and answers whether it could;
    /// `false` means the layer must be built again.
    pub fn update(&mut self, old: &Node, new: &Node, ctx: Ctx<'_>) -> bool {
        if let Some((from, to)) = moved(old, new) {
            if to != from {
                self.layer.setAffineTransform(cg::transform(*to));
            }
            return true;
        }
        match (old, new) {
            (
                Node::Painted {
                    geometry: old_geometry,
                    stroke: old_stroke,
                    paint: old_paint,
                },
                Node::Painted {
                    geometry,
                    stroke,
                    paint,
                },
            ) => self.update_painted(
                (old_geometry, old_stroke, old_paint),
                (geometry, stroke, paint),
                ctx,
            ),
            (
                Node::Image {
                    image: old_image,
                    dst: old_dst,
                    sampling: old_sampling,
                },
                Node::Image {
                    image,
                    dst,
                    sampling,
                },
            ) => {
                if image != old_image {
                    set_contents(&self.layer, &ctx.registry.image(*image));
                }
                if sampling != old_sampling {
                    sampling_filter(&self.layer, *sampling);
                }
                if dst != old_dst {
                    place(&self.layer, *dst);
                }
                true
            }
            (
                Node::Shadow {
                    shape: old_shape,
                    shadow: old_shadow,
                },
                Node::Shadow { shape, shadow },
            ) => {
                if shape != old_shape || shadow != old_shadow {
                    let Parts::Shadow(caster) = &self.parts else {
                        unreachable!("a shadow node is built with its caster");
                    };
                    set_shadow(&self.layer, caster, shape, shadow);
                }
                true
            }
            (Node::Clip(old_shape), Node::Clip(shape)) => {
                if shape != old_shape {
                    set_clip(self, shape);
                }
                true
            }
            (Node::Group(old_group), Node::Group(group)) => {
                if group.opacity.to_bits() != old_group.opacity.to_bits() {
                    self.layer.setOpacity(group.opacity);
                }
                if group.blend != old_group.blend {
                    set_composite(&self.layer, group.blend);
                }
                true
            }
            _ => false,
        }
    }

    fn update_painted(
        &mut self,
        old: (&Geometry, &Option<Stroke>, &Paint),
        new: (&Geometry, &Option<Stroke>, &Paint),
        ctx: Ctx<'_>,
    ) -> bool {
        let (old_geometry, old_stroke, old_paint) = old;
        let (geometry, stroke, paint) = new;
        let moved = geometry != old_geometry || stroke != old_stroke;
        let repainted = paint != old_paint;
        match (&self.parts, plan::classify(paint)) {
            (Parts::Shape(layer), PaintClass::Solid(color)) => {
                if moved || repainted {
                    shape(
                        layer,
                        &coverage(geometry, stroke.as_ref(), ctx),
                        &cg::color(color),
                    );
                    set_range(layer, cg::color_extended(color));
                }
                true
            }
            (Parts::Painted { painter, mask }, PaintClass::Drawn) => {
                if moved {
                    shape_mask(&self.layer, mask, &coverage(geometry, stroke.as_ref(), ctx));
                }
                if repainted {
                    painter.ivars().drawing.borrow_mut().paint = paint.clone();
                    set_range(&self.layer, cg::paint_extended(paint));
                }
                if moved || repainted {
                    self.layer.setNeedsDisplay();
                }
                true
            }
            (Parts::Mesh(_), PaintClass::Mesh { mesh, transform }) => {
                let Parts::Mesh(state) = &mut self.parts else {
                    unreachable!("matched as a mesh above");
                };
                if moved {
                    shape_mask(
                        &self.layer,
                        &state.mask,
                        &coverage(geometry, stroke.as_ref(), ctx),
                    );
                }
                if repainted {
                    state.mesh = upload(&state.pass, mesh);
                    state.transform = transform;
                    set_dynamic_range(&self.layer, cg::paint_extended(paint));
                }
                if moved || repainted {
                    state.paint(ctx.scale, ctx.wake);
                }
                true
            }
            // A different realization of the paint: a different layer.
            _ => false,
        }
    }

    /// Places `child` among the layer's sublayers.
    pub fn add_sublayer(&self, child: &CALayer) {
        self.layer.addSublayer(child);
    }

    /// Applies the display scale to the layer and its mask.
    fn set_scale(&self, ctx: Ctx<'_>) {
        let scale = ctx.scale;
        set_contents_scale(&self.layer, scale);
        match &self.parts {
            Parts::Painted { mask, .. } | Parts::Mask(mask) => {
                set_contents_scale(mask, scale);
                if matches!(self.parts, Parts::Painted { .. }) {
                    self.layer.setNeedsDisplay();
                }
            }
            Parts::Shadow(caster) => set_contents_scale(caster, scale),
            Parts::Mesh(state) => state.paint(scale, ctx.wake),
            Parts::Plain | Parts::Shape(_) => {}
        }
    }

    /// Shows the layer's mesh paint if the GPU has finished it.
    pub fn show_completed(&self) {
        if let Parts::Mesh(state) = &self.parts {
            state.show_completed();
        }
    }

    /// Waits for the layer's mesh paint and shows it.
    #[cfg(all(test, target_os = "macos"))]
    pub fn settle(&self) {
        if let Parts::Mesh(state) = &self.parts {
            state.settle();
        }
    }
}
