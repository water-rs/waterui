//! Vector paths and shape masks: `CGMutablePath` as a builder plus the
//! `CAShapeLayer` a view masks with.
//!
//! The path constructors here are the framework's own spell of the shape
//! vocabulary — rect, ellipse, rounded rect, per-corner arcs, free-form
//! commands — without naming any consumer's shape model. Callers translate
//! their normalized commands into points themselves; the builder only knows
//! points and radii.
//!
//! # Safety
//!
//! The `unsafe` calls are `CGPath` constructors and appends on a mutable
//! path this module owns for the call's duration; every transform pointer
//! is a live stack value, and `setMask`/`layer` run on live objects the
//! caller keeps on the main thread.

use core::ptr;

use objc2_core_foundation::{CFRetained, CGAffineTransform};
use objc2_core_graphics::{CGColor, CGMutablePath, CGPath};
use objc2_quartz_core::{CALayer, CAShapeLayer};

use crate::geometry::{Point, Rect};
use crate::{PlatformView, Retained};

/// A `CGMutablePath` under construction.
pub struct PathBuilder {
    path: CFRetained<CGMutablePath>,
}

impl core::fmt::Debug for PathBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PathBuilder").finish_non_exhaustive()
    }
}

impl Default for PathBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PathBuilder {
    /// An empty path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            path: CGMutablePath::new(),
        }
    }

    /// `move(to:)` — lift the pen to `point` without drawing.
    pub fn move_to(&self, point: Point) {
        // SAFETY: `path` is a live mutable path owned by `self`; a null
        // transform is the documented no-transform argument.
        unsafe { CGMutablePath::move_to_point(Some(&self.path), ptr::null(), point.x, point.y) };
    }

    /// `addLine(to:)` — a straight segment to `point`.
    pub fn line_to(&self, point: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_line_to_point(Some(&self.path), ptr::null(), point.x, point.y);
        };
    }

    /// `addQuadCurve(to:control:)` — a quadratic Bézier to `point`.
    pub fn quad_to(&self, control: Point, point: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_quad_curve_to_point(
                Some(&self.path),
                ptr::null(),
                control.x,
                control.y,
                point.x,
                point.y,
            );
        };
    }

    /// `addCurve(to:control1:control2:)` — a cubic Bézier to `point`.
    pub fn cubic_to(&self, control1: Point, control2: Point, point: Point) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_curve_to_point(
                Some(&self.path),
                ptr::null(),
                control1.x,
                control1.y,
                control2.x,
                control2.y,
                point.x,
                point.y,
            );
        };
    }

    /// `addArc(center:radius:startAngle:endAngle:clockwise:transform:)` —
    /// an elliptical arc: `center`, `radius.x`/`radius.y` the ellipse's
    /// semi-axes, `start`/`sweep` in radians (negative sweep = clockwise).
    ///
    /// A `|sweep|` at or beyond a full turn draws the whole ellipse, as the
    /// caller's shape vocabulary expects — `CGPathAddArc` alone cannot spell
    /// a closed ellipse.
    pub fn arc(&self, center: Point, radius: Point, start: f64, sweep: f64) {
        if sweep.abs() >= core::f64::consts::TAU - 0.0001 {
            self.ellipse(Rect::new(
                center.x - radius.x,
                center.y - radius.y,
                radius.x * 2.0,
                radius.y * 2.0,
            ));
            return;
        }
        // The transform maps the unit circle onto the ellipse: scale by the
        // semi-axes, then translate to the center.
        let transform = CGAffineTransform {
            a: radius.x,
            b: 0.0,
            c: 0.0,
            d: radius.y,
            tx: center.x,
            ty: center.y,
        };
        // SAFETY: `transform` is a live stack value for the call's duration;
        // `path` is a live mutable path owned by `self`.
        unsafe {
            CGMutablePath::add_arc(
                Some(&self.path),
                &raw const transform,
                0.0,
                0.0,
                1.0,
                start,
                start + sweep,
                sweep < 0.0,
            );
        };
    }

    /// `addArc(tangent1End:tangent2End:radius:)` — the arc between two
    /// tangent points, the corner primitive a per-corner rounded rect is
    /// built from.
    pub fn arc_to_point(&self, tangent1: Point, tangent2: Point, radius: f64) {
        // SAFETY: see `move_to`.
        unsafe {
            CGMutablePath::add_arc_to_point(
                Some(&self.path),
                ptr::null(),
                tangent1.x,
                tangent1.y,
                tangent2.x,
                tangent2.y,
                radius,
            );
        };
    }

    /// `addRect(_:)` — append `rect` as a subpath.
    pub fn rect(&self, rect: Rect) {
        // SAFETY: see `move_to`.
        unsafe { CGMutablePath::add_rect(Some(&self.path), ptr::null(), rect.into()) };
    }

    /// `addEllipse(in:)` — the ellipse inscribed in `rect`.
    pub fn ellipse(&self, rect: Rect) {
        // SAFETY: see `move_to`.
        unsafe { CGMutablePath::add_ellipse_in_rect(Some(&self.path), ptr::null(), rect.into()) };
    }

    /// `closeSubpath` — draw back to the subpath's start.
    pub fn close(&self) {
        CGMutablePath::close_subpath(Some(&self.path));
    }

    /// The finished immutable path.
    ///
    /// # Panics
    ///
    /// Never in practice: `CGPathCreateCopy` of a live path fails only on
    /// memory exhaustion.
    #[must_use]
    pub fn finish(&self) -> CFRetained<CGPath> {
        CGPath::new_copy(Some(&self.path)).expect(
            "`CGPathCreateCopy` of a live path is non-null: it fails only on memory exhaustion",
        )
    }
}

/// `CGPath(rect:)` — a standalone rectangle path.
#[must_use]
pub fn rect_path(rect: Rect) -> CFRetained<CGPath> {
    // SAFETY: no transform applied; `rect` is a plain geometry value.
    unsafe { CGPath::with_rect(rect.into(), ptr::null()) }
}

/// `CGPath(ellipseIn:)` — the ellipse inscribed in `rect`.
#[must_use]
pub fn ellipse_path(rect: Rect) -> CFRetained<CGPath> {
    // SAFETY: no transform applied; `rect` is a plain geometry value.
    unsafe { CGPath::with_ellipse_in_rect(rect.into(), ptr::null()) }
}

/// `CGPath(roundedRect:cornerWidth:cornerHeight:)` — `rect` with uniform
/// corners `radius` wide and tall.
#[must_use]
pub fn rounded_rect_path(rect: Rect, radius: f64) -> CFRetained<CGPath> {
    // SAFETY: no transform applied; `rect` and `radius` are plain values.
    unsafe { CGPath::with_rounded_rect(rect.into(), radius, radius, ptr::null()) }
}

/// A fresh `CAShapeLayer` — the mask a `set_path` away from clipping a view.
#[must_use]
pub fn shape_layer() -> Retained<CAShapeLayer> {
    CAShapeLayer::new()
}

/// Sets `layer`'s `path`.
pub fn set_path(layer: &CAShapeLayer, path: &CGPath) {
    layer.setPath(Some(path));
}

/// `view`'s backing `CALayer`, making `AppKit` create one first when the
/// view is not already layer-backed (`UIKit` views always have one).
#[must_use]
pub fn layer(view: &PlatformView) -> Option<Retained<CALayer>> {
    #[cfg(target_os = "macos")]
    {
        view.setWantsLayer(true);
        view.layer()
    }
    #[cfg(target_os = "ios")]
    {
        Some(view.layer())
    }
}

/// Makes `mask` the mask of `view`'s layer — the clip a shape layer
/// applies. `None` clears the mask.
pub fn set_mask(view: &PlatformView, mask: Option<&CALayer>) {
    if let Some(layer) = layer(view) {
        set_layer_mask(&layer, mask);
    }
}

/// Makes `mask` the mask of `layer`. `None` clears the mask.
pub fn set_layer_mask(layer: &CALayer, mask: Option<&CALayer>) {
    // SAFETY: `layer` is a live `CALayer`; `mask` is a live layer or nil,
    // both valid for the property.
    unsafe { layer.setMask(mask) };
}

/// Sets `mask` as `view`'s `maskView` on `UIKit`.
#[cfg(target_os = "ios")]
pub fn set_mask_view(view: &PlatformView, mask: Option<&PlatformView>) {
    view.setMaskView(mask);
}

/// A `CALayer` delegate that rasterizes an off-hierarchy `AppKit` `NSView`
/// subtree into its backing store so it can act as a `CALayer.mask` without
/// fighting `AppKit`'s layer-backed view superlayer invariants.
#[cfg(target_os = "macos")]
mod macos_mask_layer {
    use core::fmt;

    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, ProtocolObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{NSAppearanceCustomization as _, NSGraphicsContext};
    use objc2_core_graphics::CGContext;
    use objc2_foundation::{NSNull, NSObjectProtocol, NSString};
    use objc2_quartz_core::{CAAction, CALayer, CALayerDelegate};

    use crate::PlatformView;
    use crate::callback::guarded;
    use crate::geometry::Rect;

    struct MaskPainterIvars {
        mask_host: Retained<PlatformView>,
    }

    define_class!(
        // SAFETY: `NSObject` subclass is `MainThreadOnly` and Quartz invokes
        // its delegate only on the main thread.
        #[unsafe(super(NSObject))]
        #[name = "CocoaUiViewMaskPainter"]
        #[thread_kind = MainThreadOnly]
        #[ivars = MaskPainterIvars]
        struct MaskPainter;

        // SAFETY: `NSObjectProtocol` has no additional requirements.
        unsafe impl NSObjectProtocol for MaskPainter {}
        // SAFETY: The delegate callback matches `CALayer`'s main-thread contract.
        unsafe impl CALayerDelegate for MaskPainter {}

        impl MaskPainter {
            #[unsafe(method_id(actionForLayer:forKey:))]
            fn action_for_layer_for_key(
                &self,
                _layer: &CALayer,
                _event: &NSString,
            ) -> Option<Retained<ProtocolObject<dyn CAAction>>> {
                Some(ProtocolObject::from_retained(NSNull::null()))
            }

            #[unsafe(method(drawLayer:inContext:))]
            fn draw_layer_in_context(&self, layer: &CALayer, context: &CGContext) {
                guarded("CocoaUiViewMaskPainter drawLayer:inContext:", || {
                    let bounds: Rect = layer.bounds().into();
                    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
                        return;
                    }
                    let mask_host = &self.ivars().mask_host;
                    mask_host.layoutSubtreeIfNeeded();
                    let ns_ctx = NSGraphicsContext::graphicsContextWithCGContext_flipped(
                        context,
                        mask_host.isFlipped(),
                    );
                    let previous = NSGraphicsContext::currentContext();
                    NSGraphicsContext::setCurrentContext(Some(&ns_ctx));
                    mask_host.displayRectIgnoringOpacity_inContext(bounds.into(), &ns_ctx);
                    NSGraphicsContext::setCurrentContext(previous.as_deref());
                });
            }
        }
    );

    impl MaskPainter {
        fn new(mtm: MainThreadMarker, mask_host: Retained<PlatformView>) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(MaskPainterIvars { mask_host });
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    impl fmt::Debug for MaskPainter {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("MaskPainter").finish_non_exhaustive()
        }
    }

    /// A `CALayer` backed by rasterizing an `AppKit` mask `NSView` hierarchy.
    #[derive(Debug)]
    pub struct ViewMaskLayer {
        layer: Retained<CALayer>,
        _painter: Retained<MaskPainter>,
    }

    impl ViewMaskLayer {
        /// Creates a mask layer that draws `mask_host` on demand.
        #[must_use]
        pub fn new(mtm: MainThreadMarker, mask_host: Retained<PlatformView>) -> Self {
            let flipped = mask_host.isFlipped();
            let painter = MaskPainter::new(mtm, mask_host);
            let layer = CALayer::new();
            layer.setGeometryFlipped(flipped);
            layer.setDelegate(Some(ProtocolObject::from_ref(&*painter)));
            layer.setNeedsDisplayOnBoundsChange(true);
            Self {
                layer,
                _painter: painter,
            }
        }

        /// The underlying `CALayer` to attach via [`super::set_mask`].
        #[must_use]
        pub fn layer(&self) -> Retained<CALayer> {
            self.layer.clone()
        }

        /// Updates the mask layer's frame, appearance, backing scale, and redraws it.
        pub fn sync(
            &self,
            host: &PlatformView,
            mask_host: &PlatformView,
            bounds: Rect,
            scale: Option<f64>,
        ) {
            mask_host.setAppearance(Some(&host.effectiveAppearance()));
            mask_host.layoutSubtreeIfNeeded();
            self.layer.setFrame(bounds.into());
            if let Some(scale) = scale
                && scale.is_finite()
                && scale > 0.0
                && self.layer.contentsScale().to_bits() != scale.to_bits()
            {
                self.layer.setContentsScale(scale);
            }
            self.layer.setNeedsDisplay();
        }
    }

    impl Drop for ViewMaskLayer {
        fn drop(&mut self) {
            self.layer.setDelegate(None);
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_mask_layer::ViewMaskLayer;
/// A `CAShapeLayer` a view hosts as a sublayer — path, fill colour and
/// frame written imperatively.
/// A shape layer owned by its host view's layer tree.
#[derive(Debug)]
pub struct ShapeLayer {
    layer: Retained<CAShapeLayer>,
}

impl ShapeLayer {
    /// An empty shape layer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            layer: CAShapeLayer::new(),
        }
    }

    /// The backing `CAShapeLayer`, for `addSublayer` and friends.
    #[must_use]
    pub fn layer(&self) -> Retained<CAShapeLayer> {
        self.layer.clone()
    }

    /// The path the layer fills.
    pub fn set_path(&self, path: Option<&CGPath>) {
        self.layer.setPath(path);
    }

    /// The colour the layer fills its path with; `None` clears it.
    pub fn set_fill_color(&self, color: Option<&CGColor>) {
        self.layer.setFillColor(color);
    }

    /// The colour the layer strokes its path with; `None` clears it.
    pub fn set_stroke_color(&self, color: Option<&CGColor>) {
        self.layer.setStrokeColor(color);
    }

    /// The stroke width in logical points.
    pub fn set_line_width(&self, width: f64) {
        self.layer.setLineWidth(width);
    }

    /// The layer's frame — the host re-frames it on every layout pass.
    pub fn set_frame(&self, frame: Rect) {
        self.layer.setFrame(frame.into());
    }
}

impl Default for ShapeLayer {
    /// An empty layer, same as [`ShapeLayer::new`].
    fn default() -> Self {
        Self::new()
    }
}
