//! Type conversions between WaterUI and vello (kurbo/peniko) types.
//!
//! These conversions are internal to the graphics module and allow the Canvas API
//! to use WaterUI's native types while rendering with vello's kurbo/peniko types.

use crate::color::ResolvedColor;
use waterui_core::layout::{Point, Rect, Size};

// Internal imports for rendering (not exposed to users)
use vello::{kurbo, peniko};

// ============================================================================
// Point conversions
// ============================================================================

#[inline]
pub fn point_to_kurbo(p: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(p.x), f64::from(p.y))
}

// ============================================================================
// Size conversions
// ============================================================================

#[inline]
pub fn size_to_kurbo(s: Size) -> kurbo::Size {
    kurbo::Size::new(f64::from(s.width), f64::from(s.height))
}

// ============================================================================
// Rect conversions
// ============================================================================

#[inline]
pub fn rect_to_kurbo(r: Rect) -> kurbo::Rect {
    let origin = point_to_kurbo(r.origin());
    let size = size_to_kurbo(*r.size());
    kurbo::Rect::from_origin_size(origin, size)
}

// ============================================================================
// Color conversions
// ============================================================================

#[inline]
pub fn resolved_color_to_peniko(c: ResolvedColor) -> peniko::Color {
    // Convert linear RGB (with headroom) into sRGB for peniko.
    let srgb = c.to_srgb_with_headroom();
    let opacity = c.opacity.clamp(0.0, 1.0);
    peniko::Color::new([srgb.red, srgb.green, srgb.blue, opacity])
}
