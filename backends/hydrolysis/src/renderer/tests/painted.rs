//! The window's committed runs flattened into one recording, read from the
//! mount roots on every painter.

use crate::renderer::HydrolysisRenderer;
use crate::renderer::mount::layers::{LayerVisitor, visit};
use crate::renderer::recording::Recording;

/// Flattens every committed run into one window-space recording.
struct Painted(Recording);

impl<E: crate::text::TextEngine> LayerVisitor<E> for Painted {
    fn run(&mut self, world: kurbo::Affine, recording: &Recording) {
        self.0.append(recording, world);
    }

    fn open_clip(&mut self, world: kurbo::Affine, clip: &cherenkov::ShapeData) {
        self.0.push_clip_data(world, clip.clone());
    }

    fn close_clip(&mut self) {
        self.0.pop_scope();
    }
}

impl HydrolysisRenderer<crate::text::ParleyEngine> {
    /// Everything the committed layers paint, in window points.
    pub fn painted_scene(&self) -> Recording {
        let mut painted = Painted(Recording::default());
        visit(&self.mount_roots(), &mut painted);
        painted.0
    }
}
