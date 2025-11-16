//! CPU renderer built on top of `tiny-skia`.

use tiny_skia::{Color, Paint, Pixmap};
use waterui_core::Environment;

use crate::{
    backend::{FrameResult, RenderBackend},
    scene::{DrawCommand, Scene},
    tree::{DirtyReason, LayoutEngine, NodeId, RenderCtx, RenderTree},
};

/// CPU surface that renders into a `tiny-skia` pixmap.
pub struct TinySkiaBackend {
    pixmap: Pixmap,
    clear_color: Color,
    scale_factor: f32,
}

impl core::fmt::Debug for TinySkiaBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TinySkiaBackend")
            .field("width", &self.width())
            .field("height", &self.height())
            .field("scale_factor", &self.scale_factor)
            .finish()
    }
}

impl TinySkiaBackend {
    /// Creates a new backend rendering into a pixmap with the provided size (logical pixels).
    #[must_use]
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let pixmap = Pixmap::new(width, height)?;
        Some(Self {
            pixmap,
            clear_color: Color::from_rgba8(0, 0, 0, 0),
            scale_factor: 1.0,
        })
    }

    /// Resizes the underlying pixmap, keeping the previously configured clear color.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        match Pixmap::new(width, height) {
            Some(new_pixmap) => {
                self.pixmap = new_pixmap;
                true
            }
            None => false,
        }
    }

    /// Returns the pixmap width.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    /// Returns the pixmap height.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// Returns a reference to the backing pixmap for presenting or copying.
    #[must_use]
    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// Returns the current scale factor used by the backend.
    #[must_use]
    pub const fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Updates the scale factor (callers are responsible for resizing the pixmap accordingly).
    pub fn set_scale_factor(&mut self, factor: f32) {
        self.scale_factor = factor;
    }

    /// Sets the color used to clear the pixmap each frame.
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    fn clear(&mut self) {
        self.pixmap.fill(self.clear_color);
    }
}

impl RenderBackend for TinySkiaBackend {
    fn render(&mut self, tree: &mut RenderTree, env: &Environment) -> FrameResult {
        let mut had_work = false;
        let dirty_nodes: Vec<_> = tree.drain_dirty().collect();
        for dirty in dirty_nodes {
            had_work = true;
            if let Some(node) = tree.node_mut(dirty.id) {
                if matches!(dirty.reason, DirtyReason::Reactive) {
                    node.update_reactive();
                }
            }
        }

        if !had_work {
            return FrameResult::Idle;
        }

        self.clear();

        let root = tree.root();
        if let Some(root_id) = root {
            let mut engine = LayoutEngine::new(tree, env);
            engine.run();

            let mut render_ctx = RenderCtx::new(env);
            paint_subtree(tree, &mut render_ctx, root_id);
            let scene = render_ctx.finish();
            self.rasterize(&scene);
            FrameResult::Presented
        } else {
            FrameResult::Idle
        }
    }
}

fn paint_subtree(tree: &mut RenderTree, ctx: &mut RenderCtx<'_>, id: NodeId) {
    if let Some(node) = tree.node_mut(id) {
        node.paint(ctx);
    }
    let children = tree.children(id).to_vec();
    for child in children {
        paint_subtree(tree, ctx, child);
    }
}

impl TinySkiaBackend {
    fn rasterize(&mut self, scene: &Scene) {
        for command in scene.commands() {
            match command {
                DrawCommand::SolidRect { rect, color } => {
                    let ts_rect = match tiny_skia::Rect::from_xywh(
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    ) {
                        Some(value) => value,
                        None => continue,
                    };
                    if let Some(ts_color) = tiny_skia::Color::from_rgba(
                        color.red,
                        color.green,
                        color.blue,
                        color.opacity,
                    ) {
                        let mut paint = Paint::default();
                        paint.set_color(ts_color);
                        self.pixmap.fill_rect(
                            ts_rect,
                            &paint,
                            tiny_skia::Transform::identity(),
                            None,
                        );
                    }
                }
                DrawCommand::Text { .. } => {
                    // TODO(text-rendering): integrate cosmic-text to draw glyphs.
                }
                DrawCommand::Placeholder(_) => {}
            }
        }
    }
}
