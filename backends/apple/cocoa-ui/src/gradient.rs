//! A `CAGradientLayer` a view hosts as a sublayer.
//!
//! Imperative only: stops, endpoints, and frame are written explicitly.
//!
//! # Safety
//!
//! `CGColor` stops cross into `setColors` through a `CFArray`/`NSArray`
//! toll-free-bridge cast — the only `unsafe` in the module, discharged next to
//! the call.

use objc2::rc::Retained;
use objc2_core_foundation::CFArray;
use objc2_foundation::{NSArray, NSNumber};
use objc2_quartz_core::{
    CAGradientLayer, kCAGradientLayerAxial, kCAGradientLayerConic, kCAGradientLayerRadial,
};

use crate::geometry::{Point, Rect};

/// The gradient's shape, mapping to `CAGradientLayer`'s three types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GradientKind {
    /// Axial — `startPoint`/`endPoint` are the gradient vector.
    Linear,
    /// Radial — `startPoint` is the center, `endPoint` a point on the rim.
    Radial,
    /// Angular — a conic sweep about `startPoint`.
    Angular,
}

/// One stop: a colour at a position in `[0, 1]`.
#[derive(Debug)]
pub struct GradientStop {
    /// The stop's position along the gradient.
    pub position: f64,
    /// The stop's colour.
    pub color: objc2_core_foundation::CFRetained<objc2_core_graphics::CGColor>,
}

/// A gradient layer owned by its host view's layer tree.
#[derive(Debug)]
pub struct GradientLayer {
    layer: Retained<CAGradientLayer>,
}

impl GradientLayer {
    /// An empty gradient layer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            layer: CAGradientLayer::new(),
        }
    }

    /// The backing `CAGradientLayer`, for `addSublayer` and friends.
    #[must_use]
    pub fn layer(&self) -> Retained<CAGradientLayer> {
        self.layer.clone()
    }

    /// All stops at once: colours and their positions.
    pub fn set_stops(&self, stops: &[GradientStop]) {
        let colors: Vec<&objc2_core_graphics::CGColor> =
            stops.iter().map(|stop| &*stop.color).collect();
        let colors = CFArray::from_objects(&colors);
        // SAFETY: `CFArray` and `NSArray` are toll-free bridged and the
        // elements are `CGColor` objects, which is what `setColors` reads.
        let colors = unsafe { &*core::ptr::from_ref(&*colors).cast::<NSArray>() };
        // SAFETY: the array's elements are `CGColor` objects, the documented
        // element type of `setColors`.
        unsafe { self.layer.setColors(Some(colors)) };
        let locations: Vec<Retained<NSNumber>> = stops
            .iter()
            .map(|stop| NSNumber::new_f64(stop.position))
            .collect();
        let locations = NSArray::from_retained_slice(&locations);
        self.layer.setLocations(Some(&locations));
    }

    /// The gradient's shape and geometry.
    pub fn configure(&self, kind: GradientKind, start: Point, end: Point) {
        // SAFETY: the type statics are system constants.
        let layer_type = unsafe {
            match kind {
                GradientKind::Linear => kCAGradientLayerAxial,
                GradientKind::Radial => kCAGradientLayerRadial,
                GradientKind::Angular => kCAGradientLayerConic,
            }
        };
        self.layer.setType(layer_type);
        self.layer.setStartPoint(start.into());
        self.layer.setEndPoint(end.into());
    }

    /// The layer's frame — the host re-frames it on every layout pass.
    pub fn set_frame(&self, frame: Rect) {
        self.layer.setFrame(frame.into());
    }
}

impl Default for GradientLayer {
    /// An empty layer, same as [`GradientLayer::new`].
    fn default() -> Self {
        Self::new()
    }
}
