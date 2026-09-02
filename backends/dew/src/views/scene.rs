//! Self-drawn scene content: `Canvas` drawings and SVG documents.
//!
//! A `SceneView` hands the backend a [`SceneContent`] that draws through
//! `waterui-graphics`' engine-neutral `Scene2D` contract. Dew installs
//! `SceneViewMergeToParent` (see [`crate::dispatch::DewRenderer::render_tree`]),
//! so the content arrives here rather than falling back to a GPU surface dew
//! has no way to create.
//!
//! The node owns the content and *caches its drawing*. That is the whole
//! difference from a whole-frame renderer: hydrolysis rebuilds a scene every
//! flush because it redraws everything anyway, while dew re-rasterizes only
//! what changed, so a canvas that redrew itself every frame would dirty its
//! whole rect every frame and defeat the point of the backend. The recording
//! is therefore rebuilt only when the content invalidates itself (its own
//! signal watchers fire), when the box it was built for resizes, or when it
//! asked for another frame. Between rebuilds the same recording is re-emitted,
//! and the display-list diff proves it unchanged by pointer.
//!
//! Scene content draws inside the box it was given: the command is emitted
//! under a clip of exactly that box, which is also what the `GpuSurface`
//! realization of the same content does by rendering into a texture of that
//! size.

use core::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use accesskit::{Node as AccessibilityNode, NodeId, Role};
use kurbo::{Affine, Rect};
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_graphics::{
    SceneContent, SceneRecording, SceneView, resolve_scene_proposal, scene_stretch_axis,
};

use crate::dispatch::{DewNode, DewRenderer, RenderContext};
use crate::display_list::DrawCommand;
use crate::text::DewState;

/// A recording together with the box it was built for.
struct CachedScene {
    /// The `width × height` the content was asked to draw itself in.
    size: (f64, f64),
    recording: Arc<SceneRecording>,
}

/// The retained node behind a `SceneView`.
struct SceneNode {
    content: Box<dyn SceneContent>,
    cached: Option<CachedScene>,
    /// Set by the content's invalidator, and by content that asked for another
    /// frame; cleared by the rebuild it triggers.
    invalidated: Rc<Cell<bool>>,
    signals: FrameSignals,
    accessibility_id: NodeId,
}

impl DewNode for SceneNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        // Content that is naturally a size (an SVG's viewBox, a formula's
        // typeset box) answers with it on whichever axis the container left
        // open, and keeps its aspect ratio when only one axis was named.
        // Content that has no size of its own fills whatever it is proposed,
        // like a colour or a shape, and is sized by `.frame()` or its container.
        let proposal = resolve_scene_proposal(self.content.intrinsic_size(), proposal);
        ViewDimensions::new(Size::new(
            proposal
                .width
                .filter(|width| width.is_finite())
                .unwrap_or(0.0)
                .max(0.0),
            proposal
                .height
                .filter(|height| height.is_finite())
                .unwrap_or(0.0)
                .max(0.0),
        ))
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "logical-pixel geometry is far below f32 precision limits"
    )]
    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let width = ctx.bounds.width().max(0.0);
        let height = ctx.bounds.height().max(0.0);
        let stale = self
            .cached
            .as_ref()
            .is_none_or(|cached| cached.size != (width, height));
        if self.invalidated.replace(false) || stale {
            let mut recording = SceneRecording::new();
            let animating = self
                .content
                .build_scene(&mut recording, width as f32, height as f32);
            if animating {
                // Animated content wants another frame: the refresh request
                // schedules it, and the invalidation is what makes that frame
                // rebuild the drawing rather than replay this one.
                self.invalidated.set(true);
                self.signals.request_refresh();
            }
            self.cached = Some(CachedScene {
                size: (width, height),
                recording: Arc::new(recording),
            });
        }
        let recording = Arc::clone(
            &self
                .cached
                .as_ref()
                .expect("a scene recording exists once the node has rendered")
                .recording,
        );
        let transform = ctx.transform * Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        let bounds = Rect::new(0.0, 0.0, width, height);
        let window_bounds = transform.transform_rect_bbox(bounds);
        let list = renderer.list_mut();
        list.push_clip(window_bounds);
        list.push_placed(
            DrawCommand::Scene {
                recording,
                transform,
                bounds,
                clip: None,
            },
            window_bounds,
        );
        list.pop_clip();
        if renderer.accessibility_enabled() {
            renderer.register_built_accessibility_node(
                self.accessibility_id,
                ctx.window_bounds(),
                || (AccessibilityNode::new(Role::Image), None),
            );
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        scene_stretch_axis(self.content.intrinsic_size())
    }
}

/// Builds the retained node for a scene view, wiring the content's
/// invalidation to dew's frame pump.
pub fn build(renderer: &mut DewRenderer, scene: SceneView) -> Box<dyn DewNode> {
    let mut content = scene.into_content();
    let invalidated = Rc::new(Cell::new(false));
    let signals = renderer.signals();
    content.set_invalidator(Some(Rc::new({
        let invalidated = Rc::clone(&invalidated);
        let signals = signals.clone();
        move || {
            invalidated.set(true);
            signals.request_refresh();
        }
    })));
    Box::new(SceneNode {
        content,
        cached: None,
        invalidated,
        signals,
        accessibility_id: renderer.allocate_accessibility_id(),
    })
}
