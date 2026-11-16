//! `Canvas` realized as a Core Animation layer tree.
//!
//! A canvas records a display list; this module lowers it. [`plan`] decides
//! which layer realizes each command, [`layers`] builds and edits those
//! layers, [`replay`] draws with Core Graphics the runs of commands a group
//! blending against the canvas beneath it needs, and [`tree`] keeps the
//! layer tree in step with the recording as signals move operands and
//! regions are recorded again. No Cherenkov surface is involved.

mod cg;
mod draw;
mod glyphs;
mod host;
mod layers;
mod plan;
mod replay;
mod resources;
mod text;
mod tree;

use crate::contract::{NativeLeaf, RenderContext};
use waterui_graphics::canvas::CanvasView;

/// Installs the `Native<CanvasView>` handler.
pub fn realize_canvas_view(value: CanvasView, ctx: &RenderContext<'_>) -> NativeLeaf {
    host::mount(value, ctx)
}

#[cfg(test)]
mod tests;
