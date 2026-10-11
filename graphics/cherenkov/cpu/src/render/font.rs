//! Registered font state: the shared [`FontData`] plus the `COLRv1`
//! node-tree cache.

use std::mem::size_of_val;
use std::sync::Arc;

use cherenkov::FontData;
use rustc_hash::FxHashMap;

use super::colr::Node;

/// The `COLRv1` cache key: an exact structural key — the glyph index and
/// the run's normalized coordinates. No hashing of floats or strings.
#[derive(PartialEq, Eq, Hash)]
pub struct ColrKey {
    /// The glyph index.
    pub glyph: u32,
    /// The run's normalized variation coordinates.
    pub coords: Box<[i16]>,
}

/// A font validated on the caller thread by `Renderer::prepare_font`:
/// everything registration needs, so `add_font` cannot fail.
#[derive(Debug)]
pub struct PreparedFont {
    /// The shared front-end font record.
    pub data: FontData,
    /// Whether the font carries `COLR`.
    pub has_colr: bool,
    /// The font's validated bitmap strikes, when it carries sbix or
    /// CBDT/CBLC.
    pub bitmap: Option<Arc<super::bitmap::BitmapFont>>,
}

/// A registered font: file bytes and collection index, whether it carries
/// `COLR`, and the colour-glyph node trees built so far.
pub struct Font {
    /// The shared front-end font record.
    pub data: FontData,
    /// Validated once at registration; plain runs do not reparse tables.
    pub has_colr: bool,
    /// Resolved once at registration like `has_colr`: the font carries an
    /// sbix or CBDT/CBLC bitmap strike.
    pub has_bitmap: bool,
    /// Foreground-independent `COLRv1` node trees, per `(glyph, coords)`.
    /// Written only during single-threaded lowering; the raster phase
    /// reads the shared [`Node`] trees through `Arc` handles.
    pub colr: FxHashMap<ColrKey, Arc<[Node]>>,
}

impl Font {
    /// An estimate of the retained `COLRv1` node-tree bytes: one
    /// `size_of::<Node>()` per node plus each `BezPath`'s elements and
    /// each gradient's stops.
    pub fn colr_bytes(&self) -> u64 {
        fn node_bytes(node: &Node) -> u64 {
            let mut bytes = size_of::<Node>() as u64;
            match node {
                Node::Fill { shape, brush } => {
                    if let Some(shape) = shape {
                        bytes += size_of_val(shape.elements()) as u64;
                    }
                    if let super::colr::Brush::Gradient(g) = brush {
                        bytes += size_of_val(g.stops.as_slice()) as u64;
                    }
                }
                Node::Group { clip, children, .. } => {
                    if let Some(clip) = clip {
                        bytes += size_of_val(clip.elements()) as u64;
                    }
                    bytes += children.iter().map(node_bytes).sum::<u64>();
                }
            }
            bytes
        }
        self.colr
            .values()
            .map(|nodes| nodes.iter().map(node_bytes).sum::<u64>())
            .sum()
    }
}
