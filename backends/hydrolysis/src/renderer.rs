//! High-level renderer that builds trees from views and drives backend surfaces.

use waterui_core::{AnyView, Environment, View};

use crate::{
    DirtyReason, RenderTree,
    backend::{FrameResult, RenderBackend},
    build_tree,
};

/// Entry point for rendering `WaterUI` views through Hydrolysis.
///
/// # TODO
/// - diff and reuse render trees rather than rebuilding every frame.
/// - wire reactive watchers and bindings so updates mark nodes dirty automatically.
pub struct HydrolysisRenderer<B: RenderBackend> {
    backend: B,
    tree: RenderTree,
}

impl<B: RenderBackend> core::fmt::Debug for HydrolysisRenderer<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisRenderer")
            .field("tree_nodes", &self.tree.len())
            .finish()
    }
}

impl<B: RenderBackend> HydrolysisRenderer<B> {
    /// Creates a renderer backed by the provided surface implementation.
    #[must_use]
    pub const fn new(backend: B) -> Self {
        Self {
            backend,
            tree: RenderTree::new(),
        }
    }

    /// Renders a view once.
    ///
    /// This rebuilds the entire render tree every call (TODO: incremental diffing).
    pub fn render_view<V: View>(&mut self, env: &Environment, view: V) -> FrameResult {
        self.tree = build_tree(env, AnyView::new(view));
        if let Some(root) = self.tree.root() {
            self.tree.mark_dirty(root, DirtyReason::Layout);
        }
        self.backend.render(&mut self.tree, env)
    }

    /// Returns a reference to the underlying backend.
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Returns a mutable reference to the underlying backend.
    pub const fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}
