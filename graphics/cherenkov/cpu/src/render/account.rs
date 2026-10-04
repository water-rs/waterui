//! Heap accounting for [`RasterRenderer`](super::RasterRenderer)'s retained
//! state: every byte `memory()` reports is attributed to a category.

use std::mem::size_of;

use cherenkov::kurbo::PathEl;

use super::lower::{ContentData, Emission, Item, clip_bytes};
use super::paint::PaintData;
use super::prepared::Op;

/// The categories `memory()` reports; the sums feed the report table.
#[derive(Clone, Copy, Debug, Default)]
pub struct Categories {
    /// Surface output storage in the host's format.
    pub output: u64,
    /// Reusable band working buffers (shading scratch, isolation aprons).
    pub bands: u64,
    /// Retained per-layer content: source lists, lowered ops, device
    /// realizations (edges and clip masks).
    pub retained: u64,
    /// Registered images.
    pub images: u64,
    /// The glyph mask cache.
    pub glyphs: u64,
    /// `COLRv1` node-tree caches.
    pub colr: u64,
    /// Decoded bitmap glyph images.
    pub bitmaps: u64,
    /// Projective layers' retained local images and mip chains.
    pub projective: u64,
}

impl Categories {
    /// The total `MemoryUsage::cpu` figure.
    pub const fn total(&self) -> u64 {
        self.output
            + self.bands
            + self.retained
            + self.images
            + self.glyphs
            + self.colr
            + self.bitmaps
            + self.projective
    }
}

/// Heap bytes of one retained source-list command.
pub fn command_bytes(command: &cherenkov::Command) -> u64 {
    match command {
        cherenkov::Command::Fill { shape, paint }
        | cherenkov::Command::Stroke { shape, paint, .. } => {
            shape_bytes(shape) + paint_bytes(paint)
        }
        cherenkov::Command::Shadow { shape, .. } | cherenkov::Command::BeginClip { shape, .. } => {
            shape_bytes(shape)
        }
        cherenkov::Command::Glyphs { run, paint } => glyph_run_bytes(run) + paint_bytes(paint),
        cherenkov::Command::Picture { picture, .. } => {
            picture.display_list().heap_bytes(command_bytes)
        }
        _ => 0,
    }
}

/// Heap bytes of one lowered operation's own allocations.
pub fn op_bytes(op: &Op) -> u64 {
    match op {
        Op::Fill { shape, paint, .. } | Op::Stroke { shape, paint, .. } => {
            shape_bytes(shape) + paint_data_bytes(paint)
        }
        Op::Shadow { shape, .. } | Op::BeginClip { shape, .. } => shape_bytes(shape),
        Op::Glyphs { run, paint, .. } => glyph_run_bytes(run) + paint_data_bytes(paint),
        _ => 0,
    }
}

/// Heap bytes of one device realization: placement clip plus the item list.
pub fn emission_bytes(emission: &Emission) -> u64 {
    match emission {
        Emission::Draw(data) => data.heap_bytes(|items| {
            let mut bytes = (items.capacity() * size_of::<Item>()) as u64;
            for item in items {
                bytes += item_bytes(item);
            }
            bytes
        }),
        Emission::Clip(data) => data.heap_bytes(|clip| clip.as_deref().map_or(0, clip_bytes)),
    }
}

/// Heap bytes of one rasterization item's own allocations.
fn item_bytes(item: &Item) -> u64 {
    match item {
        Item::Draw {
            edges, paint, clip, ..
        } => {
            (edges.len() * size_of::<super::raster::Edge>()) as u64
                + paint_data_bytes(paint)
                + clip.as_deref().map_or(0, clip_bytes)
        }
        Item::Glyph { paint, clip, .. } | Item::Silhouette { paint, clip, .. } => {
            paint_data_bytes(paint) + clip.as_deref().map_or(0, clip_bytes)
        }
        Item::Shadow { clip, .. }
        | Item::PopIsolate { clip, .. }
        | Item::PopFilter { clip, .. }
        | Item::Sample { clip, .. } => clip.as_deref().map_or(0, clip_bytes),
        // The image itself is counted by the projective image cache.
        Item::Project(item) => {
            size_of::<super::lower::ProjectItem>() as u64
                + item.clip.as_deref().map_or(0, clip_bytes)
        }
        Item::PushIsolate { .. } | Item::PushFilter { .. } | Item::Capture(_) => 0,
    }
}

/// Heap bytes of a shape's path elements.
fn shape_bytes(shape: &cherenkov::ShapeData) -> u64 {
    match shape {
        cherenkov::ShapeData::Path { elements, .. } => {
            (elements.len() * size_of::<PathEl>()) as u64
        }
        _ => 0,
    }
}

/// Heap bytes of a glyph run's positioned glyphs and coordinates.
fn glyph_run_bytes(run: &cherenkov::GlyphRun) -> u64 {
    (run.glyphs.len() * size_of::<cherenkov::Glyph>() + run.coords.len() * size_of::<i16>()) as u64
}

/// Heap bytes of a front-end paint's own allocations.
fn paint_bytes(paint: &cherenkov::Paint) -> u64 {
    match paint {
        cherenkov::Paint::Linear(g) => {
            (g.stops.capacity() * size_of::<cherenkov::ColorStop>()) as u64
        }
        cherenkov::Paint::Radial(g) => {
            (g.stops.capacity() * size_of::<cherenkov::ColorStop>()) as u64
        }
        cherenkov::Paint::Sweep(g) => {
            (g.stops.capacity() * size_of::<cherenkov::ColorStop>()) as u64
        }
        cherenkov::Paint::Mesh(m) => (size_of_val(m.points()) + size_of_val(m.colors())) as u64,
        cherenkov::Paint::Shader(s) => (s.uniforms.capacity() * size_of::<f32>()) as u64,
        cherenkov::Paint::Transformed(t) => {
            size_of::<cherenkov::Paint>() as u64 + paint_bytes(&t.paint)
        }
        _ => 0,
    }
}

/// Heap bytes of a resolved paint's own allocations. Registered image
/// pixels are counted against `images`, not here.
fn paint_data_bytes(paint: &PaintData) -> u64 {
    match paint {
        PaintData::Linear { stops, .. }
        | PaintData::Radial { stops, .. }
        | PaintData::Sweep { stops, .. } => (stops.len() * size_of::<super::paint::Stop>()) as u64,
        PaintData::Mesh(mesh) => mesh.heap_bytes(),
        PaintData::Image(data) => size_of_val(&**data) as u64,
        PaintData::Transformed(inner, _) => {
            size_of::<PaintData>() as u64 + paint_data_bytes(inner.as_ref())
        }
        PaintData::Solid(_) => 0,
    }
}

/// Heap bytes of one layer's retained content.
pub fn content_bytes(content: &ContentData) -> u64 {
    content.heap_bytes(command_bytes, op_bytes, emission_bytes)
}
