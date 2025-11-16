//! Hydrolysis – the WaterUI self-drawn renderer.
//!
//! The crate hosts reusable infrastructure shared by every backend (GPU, CPU, terminal).
//! It exposes a dispatcher for parsing views, a render tree with [`RenderNode`]s, and
//! traits that concrete surfaces (Vello, tiny-skia, TUI) implement.

#![deny(missing_debug_implementations)]

pub mod backend;
pub mod components;
pub mod dispatcher;
pub mod renderer;
pub mod scene;
pub mod tree;

pub use dispatcher::ViewDispatcher;
pub use renderer::HydrolysisRenderer;
pub use scene::{DrawCommand, Scene};
pub use tree::{
    DirtyNode, DirtyReason, NodeId, RenderTree, build_tree,
    layout::{LayoutCtx, LayoutResult, Point, Rect, Size},
    reactive::NodeSignal,
    render::{RenderCtx, RenderNode},
};
