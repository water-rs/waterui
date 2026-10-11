//! `CALayer`, the compositor backing both view hierarchies.
//!
//! `UIView` always has a layer; `NSView` has one once [`ensure_layer`] asks
//! for it. Everything a layer carries that a view does not — borders,
//! corners, shadows, affine composition, sublayers — goes through the free
//! functions here so the caller keeps one spelling for both platforms.
//!
//! # Safety
//!
//! The `unsafe` here reads the view's layer and, on `AppKit`, opts the view
//! into layer backing — documented accessors on live main-thread views.

use objc2::rc::Retained;
use objc2_quartz_core::{CALayer, CAShapeLayer};

use crate::PlatformView;
use crate::geometry::{Point, Rect, Size};

/// The view's backing layer, if it has one — `UIView` always answers, an
/// `NSView` answers after [`ensure_layer`].
#[must_use]
pub fn layer_of(view: &PlatformView) -> Option<Retained<CALayer>> {
    #[cfg(target_os = "macos")]
    {
        view.layer()
    }
    #[cfg(target_os = "ios")]
    {
        Some(view.layer())
    }
}

/// Opts `view` into layer backing — `NSView.wantsLayer = true`; a `UIView`
/// is always layer-backed.
#[cfg(target_os = "macos")]
pub fn ensure_layer(view: &PlatformView) {
    view.setWantsLayer(true);
}

/// Opts `view` into layer backing — `NSView.wantsLayer = true`; a `UIView`
/// is always layer-backed.
#[cfg(target_os = "ios")]
pub const fn ensure_layer(_view: &PlatformView) {}

/// The layer's frame in its superlayer's coordinate space.
pub fn set_frame(layer: &CALayer, frame: Rect) {
    layer.setFrame(frame.into());
}

/// The layer's bounds in its own coordinate space.
pub fn set_bounds(layer: &CALayer, bounds: Rect) {
    layer.setBounds(bounds.into());
}

/// The layer's position — the point its anchor sits at in the superlayer.
pub fn set_position(layer: &CALayer, position: Point) {
    layer.setPosition(position.into());
}

/// The point of the bounds the position refers to — `0.0…1.0` fractions,
/// `(0.5, 0.5)` the default center.
pub fn set_anchor_point(layer: &CALayer, anchor: Point) {
    layer.setAnchorPoint(anchor.into());
}

/// Adds `child` above `layer`'s existing sublayers.
pub fn add_sublayer(layer: &CALayer, child: &CALayer) {
    layer.addSublayer(child);
}

/// A plain border stroke: `width` in points around `corner_radius`-rounded
/// corners; `masks_to_bounds` clips the content to them.
pub fn set_border(layer: &CALayer, width: f64, corner_radius: f64, masks_to_bounds: bool) {
    layer.setBorderWidth(width);
    layer.setCornerRadius(corner_radius);
    layer.setMasksToBounds(masks_to_bounds);
}

/// The color the border strokes in — `None` clears it.
pub fn set_border_color(layer: &CALayer, color: Option<&objc2_core_graphics::CGColor>) {
    layer.setBorderColor(color);
}

/// Whether the layer clips its contents to its bounds.
pub fn set_masks_to_bounds(layer: &CALayer, masks: bool) {
    layer.setMasksToBounds(masks);
}

/// A drop shadow: `offset` from the caster and a blur `radius` in points.
pub fn set_shadow(layer: &CALayer, offset: Size, radius: f64) {
    layer.setShadowOffset(offset.into());
    layer.setShadowRadius(radius);
}

/// The shadow's color — `None` clears it.
pub fn set_shadow_color(layer: &CALayer, color: Option<&objc2_core_graphics::CGColor>) {
    layer.setShadowColor(color);
}

/// The shadow's opacity, `0.0…1.0`.
pub fn set_shadow_opacity(layer: &CALayer, opacity: f32) {
    layer.setShadowOpacity(opacity);
}

/// The caster silhouette Core Animation composites the shadow from — `None`
/// derives it from the rendered content every frame.
pub fn set_shadow_path(layer: &CALayer, path: Option<&objc2_core_graphics::CGPath>) {
    layer.setShadowPath(path);
}

/// A `CAShapeLayer`: a layer that fills and strokes a `CGPath`.
#[derive(Debug)]
pub struct ShapeLayer(Retained<CAShapeLayer>);

impl ShapeLayer {
    /// An empty shape layer.
    #[must_use]
    pub fn new() -> Self {
        Self(CAShapeLayer::new())
    }

    /// The layer as its `CALayer` superclass, for [`add_sublayer`] and the
    /// frame accessors.
    #[must_use]
    pub fn layer(&self) -> &CALayer {
        &self.0
    }

    /// The path the layer fills and strokes — `None` clears it.
    pub fn set_path(&self, path: Option<&objc2_core_graphics::CGPath>) {
        self.0.setPath(path);
    }

    /// The fill color — `None` for a stroke-only layer.
    pub fn set_fill(&self, color: Option<&objc2_core_graphics::CGColor>) {
        self.0.setFillColor(color);
    }

    /// The stroke color — `None` for a fill-only layer.
    pub fn set_stroke(&self, color: Option<&objc2_core_graphics::CGColor>) {
        self.0.setStrokeColor(color);
    }

    /// The stroke width in points.
    pub fn set_line_width(&self, width: f64) {
        self.0.setLineWidth(width);
    }

    /// A butt line cap — stroke ends stop at their endpoints.
    pub fn set_line_cap_butt(&self) {
        use objc2_quartz_core::kCALineCapButt;
        // SAFETY: `kCALineCapButt` is a `CALineCap` constant Core Animation
        // exports.
        self.0.setLineCap(unsafe { kCALineCapButt });
    }

    /// The layer's frame; see [`set_frame`].
    pub fn set_frame(&self, frame: Rect) {
        self.0.setFrame(frame.into());
    }
}

impl Default for ShapeLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Lays `content` out for a layer-driven transform.
///
/// Frame and bounds fill `container_bounds` while the layer's anchor moves
/// to `anchor` (normalized `0.0…1.0`), so a transform written on the layer
/// pivots correctly. `last_bounds_size` carries the container size this was
/// last applied at; the function answers `true` only when something
/// actually changed, which is the caller's signal to re-apply the
/// transform — `AppKit` rewrites a layer-backed view's layer geometry on
/// every layout pass.
///
/// # Panics
///
/// Panics when `content` is not layer-backed — call [`ensure_layer`] first.
#[cfg(target_os = "macos")]
#[allow(clippy::too_many_lines)]
pub fn layout_transformed_content(
    content: &PlatformView,
    container_bounds: Rect,
    anchor: Point,
    last_bounds_size: &mut Size,
) -> bool {
    use objc2_quartz_core::CATransaction;

    ensure_layer(content);
    let layer = layer_of(content).expect("a layer-backed view has a layer");

    let content_bounds = Rect::new(
        0.0,
        0.0,
        container_bounds.size.width,
        container_bounds.size.height,
    );
    let position = Point::new(
        container_bounds
            .size
            .width
            .mul_add(anchor.x, container_bounds.origin.x),
        container_bounds
            .size
            .height
            .mul_add(anchor.y, container_bounds.origin.y),
    );
    let needs_update = *last_bounds_size != container_bounds.size
        || crate::view::frame(content) != container_bounds
        || crate::view::bounds(content) != content_bounds
        || Rect::from(layer.bounds()) != content_bounds
        || Point::from(layer.anchorPoint()) != anchor
        || Point::from(layer.position()) != position;

    if !needs_update {
        return false;
    }

    CATransaction::begin();
    CATransaction::setDisableActions(true);
    // The view's frame drives where the subtree is laid out; writing only
    // bounds leaves the frame at its stale (often zero) rect, so children
    // center on the container's origin instead of filling it.
    crate::view::set_bounds(content, content_bounds);
    crate::view::set_frame(content, container_bounds);
    layer.setBounds(content_bounds.into());
    layer.setAnchorPoint(anchor.into());
    layer.setPosition(position.into());
    CATransaction::commit();

    *last_bounds_size = container_bounds.size;
    content.setNeedsLayout(true);
    true
}
