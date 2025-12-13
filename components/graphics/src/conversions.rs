//! Type conversions between WaterUI and vello (kurbo/peniko) types.
//!
//! These conversions are internal to the graphics module and allow the Canvas API
//! to use WaterUI's native types while rendering with vello's kurbo/peniko types.

use waterui_color::ResolvedColor;
use waterui_core::layout::{Point, Rect, Size};

// Internal imports for rendering (not exposed to users)
use vello::{kurbo, peniko};

// ============================================================================
// Point conversions
// ============================================================================

#[inline]
pub(crate) fn point_to_kurbo(p: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(p.x), f64::from(p.y))
}

#[inline]
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn point_from_kurbo(p: kurbo::Point) -> Point {
    Point::new(p.x as f32, p.y as f32)
}

// ============================================================================
// Size conversions
// ============================================================================

#[inline]
pub(crate) fn size_to_kurbo(s: Size) -> kurbo::Size {
    kurbo::Size::new(f64::from(s.width), f64::from(s.height))
}

#[inline]
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn size_from_kurbo(s: kurbo::Size) -> Size {
    Size::new(s.width as f32, s.height as f32)
}

// ============================================================================
// Rect conversions
// ============================================================================

#[inline]
pub(crate) fn rect_to_kurbo(r: Rect) -> kurbo::Rect {
    let origin = point_to_kurbo(r.origin());
    let size = size_to_kurbo(*r.size());
    kurbo::Rect::from_origin_size(origin, size)
}

#[inline]
pub(crate) fn rect_from_kurbo(r: kurbo::Rect) -> Rect {
    let origin = point_from_kurbo(r.origin());
    let size = size_from_kurbo(r.size());
    Rect::new(origin, size)
}

// ============================================================================
// Color conversions
// ============================================================================

#[inline]
pub(crate) fn resolved_color_to_peniko(c: ResolvedColor) -> peniko::Color {
    // WaterUI ResolvedColor uses linear RGB with headroom
    // peniko::Color expects RGBA in [0, 1] range
    // Apply headroom to support HDR colors
    let scale = 1.0 + c.headroom;
    peniko::Color::from_rgba8(
        (c.red * scale * 255.0) as u8,
        (c.green * scale * 255.0) as u8,
        (c.blue * scale * 255.0) as u8,
        (c.opacity * 255.0) as u8,
    )
}

// ============================================================================
// Helper functions
// ============================================================================

/// Converts f32 to f64 for use with kurbo APIs.
#[inline]
pub(crate) const fn f32_to_f64(val: f32) -> f64 {
    val as f64
}

/// Converts f64 to f32 from kurbo APIs.
#[inline]
#[allow(clippy::cast_possible_truncation)]
pub(crate) const fn f64_to_f32(val: f64) -> f32 {
    val as f32
}
