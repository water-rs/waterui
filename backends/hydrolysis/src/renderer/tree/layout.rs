//! Measure/layout for the retained tree: [`RenderNode::layout`] re-reads
//! signals, re-measures through [`NodeSubView`], and caches each container's
//! child frames for the flush pass.

use super::*;

impl RenderNode {
    /// The stretch axis this node exposes when it is a layout child.
    pub(super) fn stretch(&self) -> StretchAxis {
        match self {
            RenderNode::Color(_) => StretchAxis::Both,
            RenderNode::Text(_) => StretchAxis::None,
            RenderNode::Container(container) => container.layout.stretch_axis(),
            RenderNode::Opacity(node) => node.child.stretch(),
            RenderNode::Scale(node) => node.child.stretch(),
            RenderNode::Rotation(node) => node.child.stretch(),
            RenderNode::Offset(node) => node.child.stretch(),
            RenderNode::Retain(node) => node.child.stretch(),
            RenderNode::Env(node) => node.child.stretch(),
            RenderNode::Dynamic(node) => node.child.stretch(),
            RenderNode::SceneView(_) => StretchAxis::Both,
            // A GpuSurface fills its proposal (`GpuView::stretch_axis` default);
            // a ViewEffect is a `StretchAxis::None` raw view; an AppliedFilter is
            // a layout-transparent wrapper delegating to its child.
            RenderNode::GpuSurface(_) => StretchAxis::Both,
            RenderNode::ViewEffect(_) => StretchAxis::None,
            RenderNode::AppliedFilter(node) => node.child.stretch(),
            RenderNode::Scroll(_) => StretchAxis::Both,
            RenderNode::LazyStack(_) => StretchAxis::Both,
            RenderNode::Collection(node) => node.layout.stretch_axis(),
            RenderNode::Wrapper(node) => node.child.stretch(),
            RenderNode::Widget(node) => node.stretch,
        }
    }

    /// Measure this node under a proposal (recursive). Text shaping runs through
    /// the renderer's [`HydroState`] on the main thread.
    pub(super) fn measure(
        &self,
        state: &mut HydroState,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        match self {
            RenderNode::Color(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            RenderNode::Text(text) => HydrolysisRenderer::measure_text_dimensions(
                state,
                text.content.get(),
                text.alignment.get(),
                env,
                proposal.width,
                None,
            ),
            RenderNode::Container(container) => {
                let cell = RefCell::new(state);
                let subs: Vec<NodeSubView> = container
                    .children
                    .iter()
                    .map(|child| NodeSubView::new(child, &cell, env))
                    .collect();
                let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
                ViewDimensions::new(container.layout.size_that_fits(proposal, &refs))
            }
            // Transform/opacity wrappers are layout-transparent.
            RenderNode::Opacity(node) => node.child.measure(state, env, proposal),
            RenderNode::Scale(node) => node.child.measure(state, env, proposal),
            RenderNode::Rotation(node) => node.child.measure(state, env, proposal),
            RenderNode::Offset(node) => node.child.measure(state, env, proposal),
            RenderNode::Retain(node) => node.child.measure(state, env, proposal),
            RenderNode::Env(node) => node.child.measure(state, &node.env, proposal),
            RenderNode::Dynamic(node) => node.child.measure(state, env, proposal),
            RenderNode::SceneView(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            // A GpuSurface fills its proposal, like a self-drawn scene.
            RenderNode::GpuSurface(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            // A ViewEffect and an AppliedFilter are sized by their content: the
            // effect captures the child into a texture at the child's bounds.
            RenderNode::ViewEffect(node) => node.child.borrow().measure(state, &node.env, proposal),
            RenderNode::AppliedFilter(node) => node.child.measure(state, &node.env, proposal),
            RenderNode::Scroll(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            RenderNode::LazyStack(node) => node.measure(state, proposal),
            RenderNode::Collection(node) => node.measure(state, proposal),
            // Layout-transparent: the wrapper measures its child under the node's
            // scoped environment (effect colors/a11y read env every frame).
            RenderNode::Wrapper(node) => node.child.measure(state, &node.env, proposal),
            RenderNode::Widget(node) => node.behavior.measure(state, proposal, &node.env),
        }
    }

    /// Re-measure and re-place this subtree, caching each container's child
    /// frames. Run on build and whenever a geometry-affecting input changes.
    pub(crate) fn layout(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        size: Size,
    ) {
        // No proposal parameter: at layout time a node is always placed at a concrete
        // `size`, so a layout-transparent wrapper lays its child out at exactly that
        // size (`ProposalSize::new(Some(size.w), Some(size.h))`).
        match self {
            RenderNode::Container(container) => {
                let rects = {
                    let cell = RefCell::new(&mut renderer.state);
                    let subs: Vec<NodeSubView> = container
                        .children
                        .iter()
                        .map(|child| NodeSubView::new(child, &cell, env))
                        .collect();
                    let refs: Vec<&dyn SubView> =
                        subs.iter().map(|sub| sub as &dyn SubView).collect();
                    container.layout.place(Rect::from_size(size), &refs)
                };
                for (child, rect) in container.children.iter_mut().zip(rects.iter()) {
                    child.layout(renderer, env, *rect.size());
                }
                container.placed = rects;
            }
            // Transform/opacity wrappers are layout-transparent: the child lays out
            // at the same concrete size as the wrapper.
            RenderNode::Opacity(node) => node.child.layout(renderer, env, size),
            RenderNode::Scale(node) => node.child.layout(renderer, env, size),
            RenderNode::Rotation(node) => node.child.layout(renderer, env, size),
            RenderNode::Offset(node) => node.child.layout(renderer, env, size),
            RenderNode::Retain(node) => node.child.layout(renderer, env, size),
            RenderNode::Env(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, size);
            }
            // Layout-transparent: the child lays out at the same concrete size,
            // under the wrapper's scoped environment.
            RenderNode::Wrapper(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, size);
            }
            RenderNode::Dynamic(node) => node.child.layout(renderer, env, size),
            RenderNode::Scroll(node) => {
                let child_proposal = match node.axis {
                    ScrollAxis::Horizontal => ProposalSize::new(None, Some(size.height)),
                    ScrollAxis::Vertical => ProposalSize::new(Some(size.width), None),
                    ScrollAxis::All => ProposalSize::UNSPECIFIED,
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                };
                let intrinsic = node
                    .child
                    .measure(&mut renderer.state, env, child_proposal)
                    .size;
                let content_size = match node.axis {
                    ScrollAxis::Horizontal => {
                        Size::new(intrinsic.width.max(size.width), size.height)
                    }
                    ScrollAxis::Vertical => {
                        Size::new(size.width, intrinsic.height.max(size.height))
                    }
                    ScrollAxis::All => Size::new(
                        intrinsic.width.max(size.width),
                        intrinsic.height.max(size.height),
                    ),
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                };
                node.child.layout(renderer, env, content_size);
                let handle = if let Some(handle) = node.handle.as_mut() {
                    handle.rebind(
                        node.axis,
                        f64::from(size.width),
                        f64::from(size.height),
                        f64::from(content_size.width),
                        f64::from(content_size.height),
                    )
                } else {
                    ScrollHandle::new(
                        node.axis,
                        f64::from(size.width),
                        f64::from(size.height),
                        f64::from(content_size.width),
                        f64::from(content_size.height),
                    )
                };
                node.handle = Some(handle);
                node.content_size = content_size;
                node.viewport = size;
            }
            RenderNode::Collection(node) => node.layout(renderer, size),
            // A ViewEffect captures its child into a texture sized to the child's
            // bounds, so the child lays out at the concrete size (re-laid-out only
            // on a change). An AppliedFilter is layout-transparent: its child lays
            // out at the same concrete size under the node's scoped environment.
            RenderNode::ViewEffect(node) => {
                if size != node.laid_out.get() {
                    let node_env = node.env.clone();
                    node.child.borrow_mut().layout(renderer, &node_env, size);
                    node.laid_out.set(size);
                }
            }
            RenderNode::AppliedFilter(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, size);
            }
            // A lazy stack places its items lazily at flush (offset-dependent); a
            // widget leaf or GpuSurface renders itself at flush from `ctx.bounds`.
            // Nothing to pre-lay-out for any of these.
            RenderNode::Color(_)
            | RenderNode::Text(_)
            | RenderNode::SceneView(_)
            | RenderNode::GpuSurface(_)
            | RenderNode::LazyStack(_)
            | RenderNode::Widget(_) => {}
        }
    }
}
