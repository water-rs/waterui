//! Heap accounting shared by retained command-list consumers.

use kurbo::PathEl;
use std::mem::{size_of, size_of_val};

/// Heap bytes of one retained source-list command.
#[must_use]
pub fn command_bytes(command: &crate::Command) -> u64 {
    match command {
        crate::Command::Fill { shape, paint } | crate::Command::Stroke { shape, paint, .. } => {
            shape_bytes(shape) + paint_bytes(paint)
        }
        crate::Command::Shadow { shape, .. } | crate::Command::BeginClip { shape, .. } => {
            shape_bytes(shape)
        }
        crate::Command::Glyphs { run, paint } => glyph_run_bytes(run) + paint_bytes(paint),
        crate::Command::Picture { picture, .. } => picture.display_list().heap_bytes(command_bytes),
        _ => 0,
    }
}

/// Heap bytes of a shape's path elements.
#[must_use]
pub fn shape_bytes(shape: &crate::ShapeData) -> u64 {
    match shape {
        crate::ShapeData::Path { elements, .. } => (elements.len() * size_of::<PathEl>()) as u64,
        _ => 0,
    }
}

/// Heap bytes of a glyph run's positioned glyphs and coordinates.
#[must_use]
pub fn glyph_run_bytes(run: &crate::GlyphRun) -> u64 {
    (run.glyphs.len() * size_of::<crate::Glyph>() + run.coords.len() * size_of::<i16>()) as u64
}

/// Heap bytes of a front-end paint's own allocations.
fn paint_bytes(paint: &crate::Paint) -> u64 {
    match paint {
        crate::Paint::Linear(g) => (g.stops.capacity() * size_of::<crate::ColorStop>()) as u64,
        crate::Paint::Radial(g) => (g.stops.capacity() * size_of::<crate::ColorStop>()) as u64,
        crate::Paint::Sweep(g) => (g.stops.capacity() * size_of::<crate::ColorStop>()) as u64,
        crate::Paint::Mesh(m) => (size_of_val(m.points()) + size_of_val(m.colors())) as u64,
        crate::Paint::Shader(s) => (s.uniforms.capacity() * size_of::<f32>()) as u64,
        crate::Paint::Transformed(t) => size_of::<crate::Paint>() as u64 + paint_bytes(&t.paint),
        _ => 0,
    }
}
